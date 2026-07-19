use crate::{
    adapters::{AgentKind, FakeSandbox, ProbeResult, probe_aoe, probe_docker, safe_write},
    db::Database,
    domain::{
        CalendarException, CalendarProfile, DayKind, NewTask, Project, ProjectLink, QuotaSurface,
        RouteCandidate, RouteDecision, RunSummary, ScheduleEvaluation, SchedulerClaim,
        SchedulerDaemonConfig, SchedulerDaemonSummary, SchedulerLeader, SchedulerPreview,
        SchedulerTick, SchedulerWake, Task, TaskStatus,
    },
    evidence::{Handoff, RunEvidence, RunManifest, verification_evidence},
    git,
    policy::{EffectivePolicy, PolicyDecision},
    projections::Projector,
    schedule,
};
use anyhow::{Result, bail};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
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
        secure_directory(&data_dir)?;
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
            schema_version: 3,
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

    pub fn configure_calendar(
        &mut self,
        slug: &str,
        timezone: &str,
        weekly_pattern: &str,
    ) -> Result<CalendarProfile> {
        self.db.configure_calendar(slug, timezone, weekly_pattern)
    }

    pub fn assign_project_calendar(
        &mut self,
        project: &str,
        calendar: &str,
    ) -> Result<CalendarProfile> {
        self.db.assign_project_calendar(project, calendar)
    }

    pub fn set_calendar_exception(
        &mut self,
        calendar: &str,
        local_date: NaiveDate,
        day_kind: DayKind,
        reason: &str,
    ) -> Result<CalendarException> {
        self.db
            .set_calendar_exception(calendar, local_date, day_kind, reason)
    }

    pub fn evaluate_task_schedule_at(
        &self,
        task_id: &str,
        now: DateTime<Utc>,
    ) -> Result<ScheduleEvaluation> {
        let task = self.db.task(task_id)?;
        let calendar = self.db.project_calendar(&task.project_id)?;
        let exceptions = self.db.calendar_exceptions(&calendar.id)?;
        schedule::evaluate(&calendar, &exceptions, task.day_affinity, now)
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
        self.route_task_at(task_id, adapter, provider, account, Utc::now())
    }

    pub fn route_task_at(
        &mut self,
        task_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
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
        let schedule = self.evaluate_task_schedule_at(task_id, now)?;
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
        let (quota_allowed, quota_reason) = if quota.is_empty() {
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
        let allowed = schedule.eligible && quota_allowed;
        let reason = if !schedule.eligible {
            format!(
                "{}: task affinity {} does not match {} day {}",
                schedule.reason_code, schedule.affinity, schedule.day_kind, schedule.local_date
            )
        } else {
            quota_reason
        };
        let selected_adapter = allowed.then(|| adapter.to_owned());
        let quota_wake = (!quota_allowed)
            .then(|| quota.iter().filter_map(|surface| surface.reset_at).max())
            .flatten();
        let schedule_wake = (!schedule.eligible)
            .then_some(schedule.next_eligible_at)
            .flatten();
        let next_wake_at = match (schedule_wake, quota_wake) {
            (Some(schedule), Some(quota)) => Some(schedule.max(quota)),
            (Some(schedule), None) => Some(schedule),
            (None, Some(quota)) => Some(quota),
            (None, None) => None,
        };
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
            schedule: Some(schedule),
            quota,
            policy_hash: self.policy.hash(),
            created_at: now,
        };
        self.db.record_route(&decision)?;
        Ok(decision)
    }

    pub fn scheduler_preview_at(
        &mut self,
        adapter: &str,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
    ) -> Result<SchedulerPreview> {
        let ready = self
            .db
            .list_tasks(None)?
            .into_iter()
            .filter(|task| task.status == TaskStatus::Ready)
            .collect::<Vec<_>>();
        let mut decisions = Vec::with_capacity(ready.len());
        for task in ready {
            decisions.push(self.route_task_at(&task.id, adapter, provider, account, now)?);
        }
        Ok(SchedulerPreview {
            evaluated_at: now,
            adapter: adapter.into(),
            provider: provider.into(),
            account: account.into(),
            decisions,
        })
    }

    pub fn register_scheduler(
        &mut self,
        instance_id: &str,
        hostname: &str,
        process_id: u32,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.db
            .register_scheduler_instance(instance_id, hostname, process_id, now)
    }

    pub fn acquire_scheduler_leader(
        &mut self,
        instance_id: &str,
        now: DateTime<Utc>,
        ttl: std::time::Duration,
    ) -> Result<SchedulerLeader> {
        self.db.acquire_scheduler_leader(instance_id, now, ttl)
    }

    pub fn heartbeat_scheduler_leader(
        &mut self,
        instance_id: &str,
        generation: i64,
        now: DateTime<Utc>,
        ttl: std::time::Duration,
    ) -> Result<SchedulerLeader> {
        self.db
            .heartbeat_scheduler_leader(instance_id, generation, now, ttl)
    }

    pub fn recover_scheduler(&mut self, now: DateTime<Utc>) -> Result<Vec<String>> {
        self.db.recover_expired_scheduler_claims(now)
    }

    pub fn stop_scheduler(&mut self, instance_id: &str, now: DateTime<Utc>) -> Result<Vec<String>> {
        self.db.stop_scheduler_instance(instance_id, now)
    }

    pub fn scheduler_wakes(&self) -> Result<Vec<SchedulerWake>> {
        self.db.scheduler_wakes()
    }

    pub fn run_scheduler_daemon(
        &mut self,
        config: &SchedulerDaemonConfig,
        shutdown: &AtomicBool,
    ) -> Result<SchedulerDaemonSummary> {
        self.run_scheduler_daemon_with(config, shutdown, Utc::now, std::thread::sleep)
    }

    fn run_scheduler_daemon_with<N, S>(
        &mut self,
        config: &SchedulerDaemonConfig,
        shutdown: &AtomicBool,
        mut now: N,
        mut sleep: S,
    ) -> Result<SchedulerDaemonSummary>
    where
        N: FnMut() -> DateTime<Utc>,
        S: FnMut(std::time::Duration),
    {
        if config.max_active_claims == 0 {
            bail!("scheduler concurrency limit must be greater than zero");
        }
        if config.poll_interval.is_zero()
            || config.leader_ttl.is_zero()
            || config.claim_ttl.is_zero()
        {
            bail!("scheduler poll interval and TTLs must be greater than zero");
        }
        if config.leader_ttl <= config.poll_interval {
            bail!("scheduler leader TTL must be greater than the poll interval");
        }
        if config.claim_ttl <= config.poll_interval {
            bail!("scheduler claim TTL must be greater than the poll interval");
        }
        if config.max_ticks == Some(0) {
            bail!("scheduler max ticks must be greater than zero when specified");
        }

        let started_at = now();
        self.register_scheduler(
            &config.instance_id,
            &config.hostname,
            std::process::id(),
            started_at,
        )?;
        let leader =
            self.acquire_scheduler_leader(&config.instance_id, started_at, config.leader_ttl)?;
        let mut ticks = 0;
        let mut claims_created = 0;
        let mut claims_renewed = 0;
        let mut scheduler_claims_recovered = self.recover_scheduler(started_at)?.len();
        let mut run_leases_recovered = self.recover_at(started_at)?.len();
        let mut shutdown_reason = "signal".to_owned();

        let loop_result: Result<()> = (|| {
            loop {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let tick_at = now();
                self.heartbeat_scheduler_leader(
                    &config.instance_id,
                    leader.generation,
                    tick_at,
                    config.leader_ttl,
                )?;
                claims_renewed += self.db.heartbeat_scheduler_claims(
                    &config.instance_id,
                    leader.generation,
                    tick_at,
                    config.claim_ttl,
                )?;
                scheduler_claims_recovered += self.recover_scheduler(tick_at)?.len();
                run_leases_recovered += self.recover_at(tick_at)?.len();
                let tick = self.scheduler_tick_at(
                    &config.instance_id,
                    leader.generation,
                    &config.adapter,
                    &config.provider,
                    &config.account,
                    tick_at,
                    config.max_active_claims,
                    config.claim_ttl,
                )?;
                ticks += 1;
                claims_created += tick.claims.len();
                if config.max_ticks.is_some_and(|limit| ticks >= limit) {
                    shutdown_reason = "max_ticks".to_owned();
                    break;
                }
                sleep(config.poll_interval);
            }
            Ok(())
        })();

        let stopped_at = now();
        let stop_result = self.stop_scheduler(&config.instance_id, stopped_at);
        match (loop_result, stop_result) {
            (Err(error), Ok(_)) => Err(error),
            (Err(error), Err(stop_error)) => {
                Err(error.context(format!("also failed to stop scheduler: {stop_error:#}")))
            }
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(released_task_ids)) => Ok(SchedulerDaemonSummary {
                instance_id: config.instance_id.clone(),
                leader_generation: leader.generation,
                started_at,
                stopped_at,
                ticks,
                claims_created,
                claims_renewed,
                scheduler_claims_recovered,
                run_leases_recovered,
                released_task_ids,
                shutdown_reason,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn scheduler_tick_at(
        &mut self,
        instance_id: &str,
        leader_generation: i64,
        adapter: &str,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
        max_active_claims: usize,
        claim_ttl: std::time::Duration,
    ) -> Result<SchedulerTick> {
        self.db.recover_expired_scheduler_claims(now)?;
        let ready = self
            .db
            .list_tasks(None)?
            .into_iter()
            .filter(|task| task.status == TaskStatus::Ready)
            .collect::<Vec<_>>();
        let mut decisions = Vec::with_capacity(ready.len());
        let mut claims: Vec<SchedulerClaim> = Vec::new();
        for task in ready {
            let decision = self.route_task_at(&task.id, adapter, provider, account, now)?;
            if !decision.allowed {
                let reason_code = decision
                    .schedule
                    .as_ref()
                    .filter(|evaluation| !evaluation.eligible)
                    .map(|evaluation| evaluation.reason_code.as_str())
                    .unwrap_or("quota.unavailable");
                self.db.record_scheduler_wake(
                    &task.id,
                    reason_code,
                    decision.next_wake_at,
                    &serde_json::json!({
                        "route_decision_id": &decision.id,
                        "reason": &decision.reason,
                    }),
                    now,
                )?;
                decisions.push(decision);
                continue;
            }
            match self.db.claim_task_for_scheduler(
                instance_id,
                leader_generation,
                &task.id,
                task.version,
                now,
                claim_ttl,
                max_active_claims,
                &[],
            ) {
                Ok(claim) => claims.push(claim),
                Err(error) => {
                    let message = error.to_string();
                    let reason_code = if message.contains("concurrency limit") {
                        "scheduler.capacity"
                    } else if message.contains("resource lock") {
                        "scheduler.resource_locked"
                    } else {
                        "scheduler.claim_conflict"
                    };
                    self.db.record_scheduler_wake(
                        &task.id,
                        reason_code,
                        None,
                        &serde_json::json!({
                            "route_decision_id": &decision.id,
                            "reason": message,
                        }),
                        now,
                    )?;
                }
            }
            decisions.push(decision);
        }
        let active_claims = self.db.active_scheduler_claim_count(now)?;
        Ok(SchedulerTick {
            evaluated_at: now,
            instance_id: instance_id.into(),
            leader_generation,
            claims,
            decisions,
            active_claims,
            capacity: max_active_claims.saturating_sub(active_claims),
        })
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
        self.recover_at(Utc::now())
    }

    pub fn recover_at(&mut self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let recovered = self.db.recover_expired_leases(now)?;
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

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> Result<()> {
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
            day_affinity: crate::domain::DayAffinity::Both,
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

    #[test]
    fn project_calendar_and_exception_control_task_day_eligibility() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        garnish
            .configure_calendar("uk-week", "Europe/London", "WWWWWOO")
            .unwrap();
        garnish
            .assign_project_calendar(&project.id, "uk-week")
            .unwrap();
        let mut scheduled = task(project.id);
        scheduled.day_affinity = crate::domain::DayAffinity::Off;
        let scheduled = garnish.add_task(&scheduled).unwrap();
        let monday = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let ordinary = garnish
            .evaluate_task_schedule_at(&scheduled.id, monday)
            .unwrap();
        assert!(!ordinary.eligible);
        assert_eq!(ordinary.reason_code, "schedule.ineligible_workday");
        assert_eq!(
            ordinary.next_eligible_at,
            Some(chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 24, 23, 0, 0).unwrap())
        );
        garnish
            .set_calendar_exception(
                "uk-week",
                chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(),
                DayKind::Off,
                "annual leave",
            )
            .unwrap();
        let exception = garnish
            .evaluate_task_schedule_at(&scheduled.id, monday)
            .unwrap();
        assert!(exception.eligible);
        assert_eq!(exception.day_source, "exception:annual leave");
    }

    #[test]
    fn scheduler_preview_is_priority_ordered_and_day_aware() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        garnish
            .configure_calendar("uk-week", "Europe/London", "WWWWWOO")
            .unwrap();
        garnish
            .assign_project_calendar(&project.id, "uk-week")
            .unwrap();
        garnish
            .set_quota(
                "fake",
                "test",
                "five_hour",
                Some(90.0),
                20.0,
                None,
                "fixture",
                None,
            )
            .unwrap();
        let mut work_task = task(project.id.clone());
        work_task.title = "workday task".into();
        work_task.priority = 20;
        work_task.day_affinity = crate::domain::DayAffinity::Work;
        let work_task = garnish.add_task(&work_task).unwrap();
        let mut off_task = task(project.id);
        off_task.title = "off-day task".into();
        off_task.priority = 10;
        off_task.day_affinity = crate::domain::DayAffinity::Off;
        let off_task = garnish.add_task(&off_task).unwrap();
        let monday = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let preview = garnish
            .scheduler_preview_at("fake", "fake", "test", monday)
            .unwrap();
        assert_eq!(preview.decisions.len(), 2);
        assert_eq!(preview.decisions[0].task_id, work_task.id);
        assert!(preview.decisions[0].allowed);
        assert_eq!(preview.decisions[1].task_id, off_task.id);
        assert!(!preview.decisions[1].allowed);
        assert!(
            preview.decisions[1]
                .reason
                .starts_with("schedule.ineligible_workday")
        );
        assert!(preview.decisions[1].next_wake_at.is_some());
        garnish
            .register_scheduler("scheduler-test", "fixture-host", 42, monday)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("scheduler-test", monday, std::time::Duration::from_secs(60))
            .unwrap();
        let tick = garnish
            .scheduler_tick_at(
                "scheduler-test",
                leader.generation,
                "fake",
                "fake",
                "test",
                monday,
                2,
                std::time::Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(tick.claims.len(), 1);
        assert_eq!(tick.claims[0].task_id, work_task.id);
        assert_eq!(
            garnish.task(&work_task.id).unwrap().status,
            TaskStatus::Leased
        );
        let wakes = garnish.scheduler_wakes().unwrap();
        assert_eq!(wakes.len(), 1);
        assert_eq!(wakes[0].task_id, off_task.id);
        assert_eq!(wakes[0].reason_code, "schedule.ineligible_workday");
    }

    #[test]
    fn daemon_renews_claims_and_requeues_them_on_bounded_shutdown() {
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
                Some(90.0),
                20.0,
                None,
                "fake",
                None,
            )
            .unwrap();
        let config = SchedulerDaemonConfig {
            instance_id: "daemon-test".into(),
            hostname: "fixture".into(),
            adapter: "fake".into(),
            provider: "fake".into(),
            account: "test".into(),
            max_active_claims: 1,
            poll_interval: std::time::Duration::from_secs(1),
            leader_ttl: std::time::Duration::from_secs(10),
            claim_ttl: std::time::Duration::from_secs(10),
            max_ticks: Some(2),
        };
        let shutdown = AtomicBool::new(false);
        let mut instant = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let summary = garnish
            .run_scheduler_daemon_with(
                &config,
                &shutdown,
                || {
                    let current = instant;
                    instant += Duration::seconds(1);
                    current
                },
                |_| {},
            )
            .unwrap();

        assert_eq!(summary.ticks, 2);
        assert_eq!(summary.claims_created, 1);
        assert_eq!(summary.claims_renewed, 1);
        assert_eq!(summary.released_task_ids, vec![task.id.clone()]);
        assert_eq!(summary.shutdown_reason, "max_ticks");
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Ready);
    }

    #[cfg(unix)]
    #[test]
    fn data_directory_and_database_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let data = dir.path().join("data");
        Garnish::open(&data).unwrap();
        assert_eq!(
            fs::metadata(&data).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(data.join("state.db"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        for entry in fs::read_dir(&data).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                assert_eq!(
                    entry.metadata().unwrap().permissions().mode() & 0o077,
                    0,
                    "{} was accessible outside the owning user",
                    entry.path().display()
                );
            }
        }
    }
}
