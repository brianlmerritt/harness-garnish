use crate::{
    adapters::{AgentKind, FakeSandbox, ProbeResult, probe_aoe, probe_docker, safe_write},
    db::Database,
    domain::{
        NewTask, Project, ProjectLink, QuotaSurface, RouteCandidate, RouteDecision, RunSummary,
        Task, TaskStatus,
    },
    evidence::{Handoff, RunEvidence, RunManifest, verification_evidence},
    git,
    policy::{EffectivePolicy, PolicyDecision},
    projections::Projector,
};
use anyhow::{Result, bail};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use ulid::Ulid;

pub struct Garnish {
    data_dir: PathBuf,
    db: Database,
    policy: EffectivePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub schema_version: u32,
    pub data_dir: String,
    pub database: String,
    pub probes: Vec<ProbeResult>,
}

impl Garnish {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        fs::create_dir_all(&data_dir)?;
        let db = Database::open(data_dir.join("state.db"))?;
        Ok(Self {
            data_dir,
            db,
            policy: EffectivePolicy::default(),
        })
    }

    pub fn with_policy(mut self, policy: EffectivePolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn doctor(&self) -> DoctorReport {
        DoctorReport {
            schema_version: 1,
            data_dir: self.data_dir.to_string_lossy().into_owned(),
            database: self.db.path().to_string_lossy().into_owned(),
            probes: vec![
                AgentKind::Codex.probe(),
                AgentKind::Claude.probe(),
                AgentKind::Antigravity.probe(),
                probe_aoe(),
                probe_docker(),
            ],
        }
    }

    pub fn add_project(&mut self, slug: &str, title: &str, root: &Path) -> Result<Project> {
        validate_slug(slug)?;
        let project = self.db.add_project(slug, title, root)?;
        Projector::new(&project).initialize(&project)?;
        Ok(project)
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        self.db.list_projects()
    }

    pub fn link_projects(
        &mut self,
        parent: &str,
        child: &str,
        relationship: &str,
    ) -> Result<ProjectLink> {
        self.db.link_projects(parent, child, relationship)
    }

    pub fn project_links(&self) -> Result<Vec<ProjectLink>> {
        self.db.list_project_links()
    }

    pub fn add_task(&mut self, task: &NewTask) -> Result<Task> {
        let project = self.db.project(&task.project_id)?;
        let task = self.db.add_task(task)?;
        self.refresh_tasks(&project)?;
        Ok(task)
    }

    pub fn task(&self, id: &str) -> Result<Task> {
        self.db.task(id)
    }

    pub fn add_dependency(&mut self, task_id: &str, depends_on_task_id: &str) -> Result<Task> {
        let task = self.db.add_dependency(task_id, depends_on_task_id)?;
        let project = self.db.project(&task.project_id)?;
        self.refresh_tasks(&project)?;
        Ok(task)
    }

    pub fn complete_task(&mut self, task_id: &str) -> Result<Vec<Task>> {
        let task = self.db.task(task_id)?;
        let promoted = self.db.complete_review(task_id)?;
        let project = self.db.project(&task.project_id)?;
        self.refresh_tasks(&project)?;
        for promoted_task in &promoted {
            if promoted_task.project_id != project.id {
                let promoted_project = self.db.project(&promoted_task.project_id)?;
                self.refresh_tasks(&promoted_project)?;
            }
        }
        Ok(promoted)
    }

    pub fn tasks(&self, project: Option<&str>) -> Result<Vec<Task>> {
        let project_id = project
            .map(|value| self.db.project(value).map(|p| p.id))
            .transpose()?;
        self.db.list_tasks(project_id.as_deref())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_quota(
        &mut self,
        provider: &str,
        account: &str,
        surface: &str,
        remaining_percent: Option<f64>,
        reserve_percent: f64,
        reset_at: Option<chrono::DateTime<Utc>>,
        source: &str,
        unknown_reason: Option<&str>,
    ) -> Result<QuotaSurface> {
        self.db.set_quota_observation(
            provider,
            account,
            surface,
            remaining_percent,
            reserve_percent,
            reset_at,
            source,
            unknown_reason,
        )
    }

    pub fn override_quota(
        &mut self,
        provider: &str,
        account: &str,
        surface: &str,
        remaining_percent: f64,
        reason: &str,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<QuotaSurface> {
        self.db.override_quota(
            provider,
            account,
            surface,
            remaining_percent,
            reason,
            expires_at,
        )
    }

    pub fn quota(&self) -> Result<Vec<QuotaSurface>> {
        self.db.list_quota()
    }

    pub fn route_task(
        &mut self,
        task_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
    ) -> Result<RouteDecision> {
        let task = self.db.task(task_id)?;
        if task.status != TaskStatus::Ready {
            bail!(
                "task must be ready to route; current status is {}",
                task.status
            );
        }
        if !self.db.dependencies_satisfied(task_id)? {
            bail!("task dependencies are not complete");
        }
        let quota: Vec<_> = self
            .db
            .list_quota()?
            .into_iter()
            .filter(|surface| surface.provider == provider && surface.account == account)
            .collect();
        let forecast = forecast_percent(&task);
        let required_headroom = quota
            .iter()
            .map(|surface| surface.reserve_percent + forecast)
            .fold(self.policy.reserve_percent + forecast, f64::max);
        let (allowed, reason) = if quota.is_empty() {
            (
                self.policy.unknown_quota_unattended,
                "no quota surfaces are available for the selected account".to_owned(),
            )
        } else if let Some(surface) = quota.iter().find(|surface| {
            surface.effective_remaining_percent.is_none()
                || surface
                    .effective_remaining_percent
                    .is_some_and(|remaining| remaining < surface.reserve_percent + forecast)
        }) {
            let reason = match surface.effective_remaining_percent {
                Some(remaining) => format!(
                    "quota_headroom: {} has {:.1}% remaining but {:.1}% is required",
                    surface.surface,
                    remaining,
                    surface.reserve_percent + forecast
                ),
                None => format!(
                    "quota_unknown: {} ({})",
                    surface.surface,
                    surface.unknown_reason.as_deref().unwrap_or("unspecified")
                ),
            };
            (false, reason)
        } else {
            (
                true,
                "all quota surfaces satisfy reserve plus forecast".to_owned(),
            )
        };
        let selected_adapter = allowed.then(|| adapter.to_owned());
        let next_wake_at = (!allowed)
            .then(|| quota.iter().filter_map(|surface| surface.reset_at).min())
            .flatten();
        let minimum_effective_remaining_percent = quota
            .iter()
            .filter_map(|surface| surface.effective_remaining_percent)
            .min_by(f64::total_cmp);
        let decision = RouteDecision {
            id: Ulid::new().to_string(),
            task_id: task.id,
            selected_adapter,
            allowed,
            reason: reason.clone(),
            required_headroom_percent: required_headroom,
            candidates: vec![RouteCandidate {
                adapter: adapter.to_owned(),
                provider: provider.to_owned(),
                account: account.to_owned(),
                allowed,
                filter_reason: reason.clone(),
                forecast_percent: forecast,
                minimum_effective_remaining_percent,
            }],
            next_wake_at,
            quota,
            policy_hash: self.policy.hash(),
            created_at: Utc::now(),
        };
        self.db.record_route(&decision)?;
        Ok(decision)
    }

    pub fn run_task(
        &mut self,
        task_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
    ) -> Result<RunSummary> {
        if !adapter.starts_with("fake") {
            bail!(
                "real agent execution is opt-in and requires a real secure backend; use `garnish agent invocation` to inspect the pinned argv or run the labelled smoke tests"
            );
        }
        let route = self.route_task(task_id, adapter, provider, account)?;
        if !route.allowed {
            bail!("route declined: {}", route.reason);
        }
        if !self.policy.allow_branch_changes {
            bail!("project policy denies automated branch and worktree creation");
        }
        let task = self.db.task(task_id)?;
        let project = self.db.project(&task.project_id)?;
        self.db.transition_task(
            task_id,
            TaskStatus::Ready,
            TaskStatus::Leased,
            "route_reserved",
        )?;
        let worktree_destination = self
            .data_dir
            .join("worktrees")
            .join(&project.slug)
            .join(task_id);
        let worktree = match git::create_task_worktree(
            Path::new(&project.root_path),
            &worktree_destination,
            task_id,
        ) {
            Ok(worktree) => worktree,
            Err(error) => {
                self.db.transition_task(
                    task_id,
                    TaskStatus::Leased,
                    TaskStatus::Failed,
                    "worktree_failed",
                )?;
                return Err(error);
            }
        };
        self.db.transition_task(
            task_id,
            TaskStatus::Leased,
            TaskStatus::Planning,
            "worktree_ready",
        )?;
        let sandbox = FakeSandbox::attest(Path::new(&worktree.path));
        match self
            .policy
            .authorize(task.risk_class, sandbox.secure_container)
        {
            PolicyDecision::Allow => {}
            PolicyDecision::RequireApproval => {
                self.db.transition_task(
                    task_id,
                    TaskStatus::Planning,
                    TaskStatus::AwaitingApproval,
                    "approval_required",
                )?;
                bail!("task risk class {} requires approval", task.risk_class);
            }
            PolicyDecision::Deny(reason) => {
                self.db.transition_task(
                    task_id,
                    TaskStatus::Planning,
                    TaskStatus::Blocked,
                    "policy_denied",
                )?;
                bail!("policy denied task: {reason}");
            }
        }
        self.db.transition_task(
            task_id,
            TaskStatus::Planning,
            TaskStatus::Running,
            "sandbox_attested",
        )?;
        let run_id = Ulid::new().to_string();
        let checkpoint_due = Utc::now() + Duration::seconds(task.checkpoint_seconds as i64);
        self.db.create_run(
            &run_id,
            task_id,
            adapter,
            &route.id,
            &worktree.path,
            &worktree.branch,
            &worktree.base_commit,
            checkpoint_due,
        )?;
        let evidence = RunEvidence::create(&self.data_dir, &run_id)?;
        let manifest = RunManifest {
            schema_version: 1,
            run_id: run_id.clone(),
            task_id: task.id.clone(),
            project_id: project.id.clone(),
            adapter: adapter.to_owned(),
            base_commit: worktree.base_commit.clone(),
            worktree: worktree.path.clone(),
            branch: worktree.branch.clone(),
            policy_hash: self.policy.hash(),
            route_decision_id: route.id.clone(),
            created_at: Utc::now().to_rfc3339(),
            sandbox: sandbox.clone(),
        };
        evidence.write_manifest(&manifest)?;
        evidence.write_route(&route)?;

        if let (Some(relative), Some(content)) = (&task.fake_write_path, &task.fake_write_content) {
            let target = safe_write(
                Path::new(&worktree.path),
                Path::new(relative),
                content.as_bytes(),
            )?;
            self.db.append_run_event(
                task_id,
                &run_id,
                "agent.file_written",
                "fake_agent",
                &serde_json::json!({"relative_path": relative, "target": target}),
            )?;
        }
        self.db.append_run_event(
            task_id,
            &run_id,
            "run.checkpointed",
            "control_plane",
            &serde_json::json!({"checkpoint_seconds": task.checkpoint_seconds}),
        )?;
        self.db.transition_task(
            task_id,
            TaskStatus::Running,
            TaskStatus::Verifying,
            "agent_exited",
        )?;

        let patch = git::patch(Path::new(&worktree.path))?;
        let verifier_destination = self.data_dir.join("verifiers").join(&run_id);
        let verifier = git::create_verification_worktree(
            Path::new(&project.root_path),
            &verifier_destination,
            &worktree.base_commit,
            &patch,
        )?;
        let verifier_sandbox = FakeSandbox::attest(Path::new(&verifier.path));
        let started = Utc::now();
        let output = git::run_argv(Path::new(&verifier.path), &task.verification_argv)?;
        let ended = Utc::now();
        let exit_code = output.status.code().unwrap_or(128);
        let verification = verification_evidence(
            task.verification_argv.clone(),
            started.to_rfc3339(),
            ended.to_rfc3339(),
            exit_code,
            &output.stdout,
            &output.stderr,
            verifier_sandbox,
            verifier.path,
        );
        evidence.write_process_output(&output.stdout, &output.stderr)?;
        evidence.write_verification(&verification)?;
        evidence.write_patch(&patch)?;
        let changed_files = git::changed_files(Path::new(&worktree.path))?;
        let head_commit = git::head(Path::new(&worktree.path))?;
        let handoff = Handoff {
            schema_version: 1,
            task_id: task.id.clone(),
            run_id: run_id.clone(),
            goal: task.goal.clone(),
            acceptance: task.acceptance.clone(),
            base_commit: worktree.base_commit.clone(),
            head_commit: head_commit.clone(),
            worktree: worktree.path.clone(),
            changed_files: changed_files.clone(),
            commands: vec![verification.clone()],
            decisions: vec![format!("route {}: {}", route.id, route.reason)],
            assumptions: vec!["fake backend attestation is deterministic test evidence".into()],
            blocker: (!verification.passed).then(|| "verification failed".into()),
            artifacts: vec![
                evidence.patch_path.to_string_lossy().into_owned(),
                evidence.verification_path.to_string_lossy().into_owned(),
            ],
            next_safe_action: if verification.passed {
                "review the patch and verification evidence; integrate only under project policy"
                    .into()
            } else {
                "diagnose the recorded verification failure before retrying".into()
            },
            unverified_facts: vec![],
            created_at: Utc::now().to_rfc3339(),
        };
        evidence.write_handoff(&handoff)?;
        evidence.write_summary(&task, adapter, &changed_files, &verification)?;
        if verification.passed {
            self.db.transition_task(
                task_id,
                TaskStatus::Verifying,
                TaskStatus::Review,
                "verification_passed",
            )?;
            self.db
                .finish_run(&run_id, "review", Some(&head_commit), exit_code)?;
        } else {
            self.db.transition_task(
                task_id,
                TaskStatus::Verifying,
                TaskStatus::Failed,
                "verification_failed",
            )?;
            self.db
                .finish_run(&run_id, "failed", Some(&head_commit), exit_code)?;
        }
        let events = self.db.events_for_run(&run_id)?;
        evidence.write_events(&events)?;
        Projector::new(&project).tasks(&project, &self.db.list_tasks(Some(&project.id))?)?;
        Projector::new(&project).handoff(&evidence.handoff_json_path, &task)?;
        Projector::new(&project).link_run(&run_id, &evidence.directory)?;

        Ok(RunSummary {
            run_id,
            task_id: task.id,
            status: if verification.passed {
                "review".into()
            } else {
                "failed".into()
            },
            adapter: adapter.to_owned(),
            worktree: worktree.path,
            branch: worktree.branch,
            base_commit: worktree.base_commit,
            patch_path: evidence.patch_path.to_string_lossy().into_owned(),
            manifest_path: evidence.manifest_path.to_string_lossy().into_owned(),
            verification_path: evidence.verification_path.to_string_lossy().into_owned(),
            handoff_path: evidence.handoff_json_path.to_string_lossy().into_owned(),
            route_decision_id: route.id,
        })
    }

    pub fn recover(&mut self) -> Result<Vec<String>> {
        let recovered = self.db.recover_expired_leases(Utc::now())?;
        for task_id in &recovered {
            let task = self.db.task(task_id)?;
            let project = self.db.project(&task.project_id)?;
            self.refresh_tasks(&project)?;
        }
        Ok(recovered)
    }

    pub fn create_approval(
        &mut self,
        task_id: &str,
        effect_class: u8,
        action: &serde_json::Value,
        minutes: i64,
    ) -> Result<String> {
        self.db.create_approval(
            task_id,
            effect_class,
            action,
            Utc::now() + Duration::minutes(minutes),
        )
    }

    pub fn decide_approval(&mut self, approval_id: &str, approve: bool) -> Result<()> {
        self.db.decide_approval(approval_id, approve)
    }

    pub fn consume_approval(
        &mut self,
        approval_id: &str,
        action: &serde_json::Value,
    ) -> Result<()> {
        self.db.consume_approval(approval_id, action)
    }

    fn refresh_tasks(&self, project: &Project) -> Result<()> {
        let tasks = self.db.list_tasks(Some(&project.id))?;
        Projector::new(project).tasks(project, &tasks)
    }
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty()
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("project slug must contain only lowercase ASCII letters, digits, and hyphens");
    }
    Ok(())
}

fn forecast_percent(task: &Task) -> f64 {
    let baseline = (task.estimated_seconds as f64 / 2700.0) * 20.0;
    (baseline * (1.0 + task.uncertainty_percent as f64 / 100.0)).clamp(1.0, 50.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn fixture_repo(path: &Path) {
        fs::create_dir_all(path).unwrap();
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "fixture@example.invalid"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Fixture"])
            .current_dir(path)
            .output()
            .unwrap();
        fs::write(path.join("README.md"), "fixture\n").unwrap();
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "fixture"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    fn task(project_id: String) -> NewTask {
        NewTask {
            project_id,
            title: "Write evidence".into(),
            goal: "Create result.txt".into(),
            rationale: "vertical slice".into(),
            scope: vec!["result.txt".into()],
            non_scope: vec!["remote Git".into()],
            acceptance: vec!["result.txt contains done".into()],
            verification_argv: vec![
                "grep".into(),
                "-q".into(),
                "done".into(),
                "result.txt".into(),
            ],
            dependencies: vec![],
            priority: 10,
            risk_class: 1,
            estimated_seconds: 60,
            uncertainty_percent: 20,
            checkpoint_seconds: 60,
            fake_write_path: Some("result.txt".into()),
            fake_write_content: Some("done\n".into()),
        }
    }

    #[test]
    fn vertical_slice_declines_then_runs_after_override() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id)).unwrap();
        garnish
            .set_quota(
                "fake",
                "test",
                "five_hour",
                Some(5.0),
                20.0,
                None,
                "fake",
                None,
            )
            .unwrap();
        let declined = garnish
            .route_task(&task.id, "fake", "fake", "test")
            .unwrap();
        assert!(!declined.allowed);
        assert!(declined.reason.contains("quota_headroom"));
        garnish
            .override_quota("fake", "test", "five_hour", 90.0, "test override", None)
            .unwrap();
        let summary = garnish.run_task(&task.id, "fake", "fake", "test").unwrap();
        assert_eq!(summary.status, "review");
        assert!(Path::new(&summary.patch_path).exists());
        assert_eq!(
            git::snapshot(&source)
                .unwrap()
                .status_porcelain_v2
                .lines()
                .filter(|line| !line.contains(".harness-garnish"))
                .count(),
            0
        );
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Review);
    }

    #[test]
    fn unknown_quota_fails_closed() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id)).unwrap();
        let decision = garnish
            .route_task(&task.id, "fake", "fake", "missing")
            .unwrap();
        assert!(!decision.allowed);
    }

    #[test]
    fn user_managed_git_policy_denies_run_before_branch_creation() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let before = git::snapshot(&source).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data"))
            .unwrap()
            .with_policy(EffectivePolicy::for_garnish_repository());
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id)).unwrap();
        garnish
            .set_quota(
                "fake",
                "test",
                "five_hour",
                Some(90.0),
                20.0,
                None,
                "fake",
                None,
            )
            .unwrap();
        let error = garnish
            .run_task(&task.id, "fake", "fake", "test")
            .unwrap_err()
            .to_string();
        assert!(error.contains("denies automated branch"));
        let after = git::snapshot(&source).unwrap();
        assert_eq!(before.base_commit, after.base_commit);
        assert_eq!(before.branch, after.branch);
        assert!(
            !dir.path()
                .join("data/worktrees/fixture")
                .join(&task.id)
                .exists()
        );
    }
}
