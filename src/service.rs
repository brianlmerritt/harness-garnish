use crate::{
    adapters::{
        AgentKind, FakeSandbox, Invocation, ProbeResult, probe_aoe, probe_docker, probe_podman,
        run_invocation_with_tick, safe_write,
    },
    db::Database,
    domain::{
        AgentCapabilityProbe, AgentCapabilityStatus, ApprovalRequest, BackupRecord,
        CalendarException, CalendarProfile, CheckpointAction, CircuitBreaker, ControlState,
        DayKind, EmergencyStopResult, FailureCategory, LocalNotification, NewTask, Project,
        ProjectLink, QuotaCollectionAttempt, QuotaReservation, QuotaSurface, RetryPlan, RetryState,
        RouteCandidate, RouteDecision, RouteTarget, RunCheckpoint, RunSummary, ScheduleEvaluation,
        SchedulerClaim, SchedulerClaimRejection, SchedulerDaemonConfig, SchedulerDaemonSummary,
        SchedulerLeader, SchedulerPreview, SchedulerTick, SchedulerWake, Task, TaskStatus,
    },
    evidence::{Handoff, RunEvidence, RunManifest, verification_evidence},
    git,
    policy::{EffectivePolicy, PolicyDecision},
    process::{ExitClassification, ProcessOutcome},
    projections::Projector,
    quota::{CODEXBAR_CONTRACT, collect_codexbar},
    routing::{
        AdapterHealth, CandidateIdentity, ProbeFreshness, RoutingCandidateInput, RoutingRequest,
        select_candidate,
    },
    schedule,
};
use anyhow::{Result, bail};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisedInvocationResult {
    pub outcome: ProcessOutcome,
    pub failure_category: Option<FailureCategory>,
    pub retry: Option<RetryPlan>,
    pub circuit: CircuitBreaker,
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
            schema_version: 12,
            data_dir: self.data_dir.to_string_lossy().into_owned(),
            database: self.db.path().to_string_lossy().into_owned(),
            probes: vec![
                AgentKind::Codex.probe(),
                AgentKind::Claude.probe(),
                AgentKind::Antigravity.probe(),
                probe_aoe(),
                probe_docker(),
                probe_podman(),
            ],
        }
    }

    pub fn refresh_agent_capabilities(
        &mut self,
        valid_for: std::time::Duration,
    ) -> Result<Vec<AgentCapabilityStatus>> {
        self.refresh_agent_capabilities_at(Utc::now(), valid_for)
    }

    pub fn refresh_agent_capabilities_at(
        &mut self,
        now: DateTime<Utc>,
        valid_for: std::time::Duration,
    ) -> Result<Vec<AgentCapabilityStatus>> {
        self.refresh_agent_capabilities_with(now, valid_for, AgentKind::probe)
    }

    fn refresh_agent_capabilities_with(
        &mut self,
        now: DateTime<Utc>,
        valid_for: std::time::Duration,
        mut probe_agent: impl FnMut(AgentKind) -> ProbeResult,
    ) -> Result<Vec<AgentCapabilityStatus>> {
        if valid_for.is_zero() {
            bail!("agent capability probe validity must be greater than zero");
        }
        let valid_until = now
            + Duration::from_std(valid_for)
                .map_err(|_| anyhow::anyhow!("agent capability validity is too large"))?;
        for kind in [AgentKind::Codex, AgentKind::Claude, AgentKind::Antigravity] {
            let observed = probe_agent(kind);
            self.db
                .record_agent_capability_probe(&AgentCapabilityProbe {
                    id: Ulid::new().to_string(),
                    adapter: observed.adapter,
                    executable: observed.executable,
                    version: observed.version,
                    health: observed.health,
                    capabilities: observed.capabilities,
                    failure: observed.failure,
                    probed_at: now,
                    valid_until,
                })?;
        }
        self.agent_capability_status_at(now)
    }

    pub fn agent_capability_status(&self) -> Result<Vec<AgentCapabilityStatus>> {
        self.agent_capability_status_at(Utc::now())
    }

    pub fn agent_capability_status_at(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<AgentCapabilityStatus>> {
        let mut latest = self
            .db
            .latest_agent_capability_probes()?
            .into_iter()
            .map(|probe| (probe.adapter.clone(), probe))
            .collect::<BTreeMap<_, _>>();
        Ok(["codex", "claude", "antigravity"]
            .into_iter()
            .map(|adapter| {
                let probe = latest.remove(adapter);
                let (freshness, health) = match probe.as_ref() {
                    None => ("unknown", "unknown"),
                    Some(probe) if probe.valid_until <= now => ("stale", probe.health.as_str()),
                    Some(probe) => ("fresh", probe.health.as_str()),
                };
                AgentCapabilityStatus {
                    adapter: adapter.into(),
                    freshness: freshness.into(),
                    health: health.into(),
                    probe,
                }
            })
            .collect())
    }

    pub fn add_project(&mut self, slug: &str, title: &str, root: &Path) -> Result<Project> {
        validate_slug(slug)?;
        let canonical_root = root.canonicalize().map_err(|error| {
            anyhow::anyhow!("canonicalizing project path {}: {error}", root.display())
        })?;
        validate_project_root_for_platform(&canonical_root, is_wsl2())?;
        let project = self.db.add_project(slug, title, root)?;
        Projector::new(&project).initialize(&project)?;
        Ok(project)
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        self.db.list_projects()
    }

    pub fn set_project_scheduler_pause(
        &mut self,
        project: &str,
        paused: bool,
        reason: &str,
    ) -> Result<Project> {
        self.db
            .set_project_scheduler_pause(project, paused, reason, Utc::now())
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

    pub fn set_task_route_pin(
        &mut self,
        task_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
        reason: &str,
    ) -> Result<Task> {
        let task = self.db.set_task_route_pin(
            task_id,
            Some((adapter, provider, account)),
            reason,
            Utc::now(),
        )?;
        let project = self.db.project(&task.project_id)?;
        self.refresh_tasks(&project)?;
        Ok(task)
    }

    pub fn clear_task_route_pin(&mut self, task_id: &str, reason: &str) -> Result<Task> {
        let task = self
            .db
            .set_task_route_pin(task_id, None, reason, Utc::now())?;
        let project = self.db.project(&task.project_id)?;
        self.refresh_tasks(&project)?;
        Ok(task)
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

    pub fn quota_reservations(&self) -> Result<Vec<QuotaReservation>> {
        self.db.list_quota_reservations()
    }

    pub fn quota_collection_attempts(&self) -> Result<Vec<QuotaCollectionAttempt>> {
        self.db.list_quota_collection_attempts()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn refresh_quota_codexbar(
        &mut self,
        executable: Option<&Path>,
        provider: &str,
        account: &str,
        collector_account: Option<&str>,
        source: &str,
        reserve_percent: f64,
        valid_for: std::time::Duration,
    ) -> Result<Vec<QuotaSurface>> {
        let attempted_at = Utc::now();
        let observations = match collect_codexbar(
            executable,
            &self.data_dir,
            provider,
            account,
            collector_account,
            source,
            reserve_percent,
            valid_for,
        ) {
            Ok(observations) => observations,
            Err(error) => {
                let detail: String = format!("{error:#}").chars().take(1_000).collect();
                if let Err(evidence_error) = self.db.record_quota_collection_attempt(
                    provider,
                    account,
                    CODEXBAR_CONTRACT,
                    "failed",
                    &detail,
                    attempted_at,
                ) {
                    return Err(anyhow::anyhow!(
                        "{error:#}; recording quota collection failure also failed: {evidence_error:#}"
                    ));
                }
                return Err(error);
            }
        };
        let surfaces = self.db.record_quota_observations(&observations)?;
        self.db.record_quota_collection_attempt(
            provider,
            account,
            CODEXBAR_CONTRACT,
            "succeeded",
            &format!("normalized {} quota surfaces", surfaces.len()),
            attempted_at,
        )?;
        Ok(surfaces)
    }

    pub fn retry_state(&self, task_id: &str) -> Result<RetryState> {
        self.db.retry_state(task_id)
    }

    pub fn set_retry_limit(&mut self, task_id: &str, limit: u32) -> Result<RetryState> {
        self.db.set_retry_limit(task_id, limit)
    }

    pub fn adapter_circuits(&self) -> Result<Vec<CircuitBreaker>> {
        self.db.adapter_circuits()
    }

    pub fn control_state(&self) -> Result<ControlState> {
        self.db.control_state()
    }

    pub fn pause_new_work(&mut self, reason: &str) -> Result<ControlState> {
        self.db.set_pause_new_work(true, Some(reason), Utc::now())
    }

    pub fn resume_operations(&mut self, reason: &str) -> Result<ControlState> {
        self.db.resume_operations(reason, Utc::now())
    }

    pub fn emergency_stop(&mut self, reason: &str) -> Result<EmergencyStopResult> {
        self.db.emergency_stop(reason, Utc::now())
    }

    pub fn operational_status(&self) -> Result<serde_json::Value> {
        self.db.operational_counts(Utc::now())
    }

    pub fn diagnostics(&self) -> Result<serde_json::Value> {
        let mut circuits = self.db.adapter_circuits()?;
        circuits.truncate(100);
        let mut wakes = self.db.scheduler_wakes()?;
        wakes.truncate(100);
        Ok(serde_json::json!({
            "schema_version": self.db.schema_version(),
            "data_dir": self.data_dir,
            "database": {
                "path": self.db.path(),
                "size_bytes": fs::metadata(self.db.path())?.len(),
            },
            "status": self.db.operational_counts(Utc::now())?,
            "pending_notifications": self.db.local_notifications(false, 20)?,
            "adapter_circuits": circuits,
            "scheduler_wakes": wakes,
            "bounds": {
                "notifications": 20,
                "adapter_circuits": 100,
                "scheduler_wakes": 100,
            },
        }))
    }

    pub fn create_backup(&self, destination: Option<&Path>) -> Result<BackupRecord> {
        let now = Utc::now();
        let default_path = self.data_dir.join("backups").join(format!(
            "state-{}-{}.db",
            now.format("%Y%m%dT%H%M%SZ"),
            Ulid::new()
        ));
        self.db.backup_to(destination.unwrap_or(&default_path), now)
    }

    pub fn local_notifications(
        &self,
        include_acknowledged: bool,
        limit: usize,
    ) -> Result<Vec<LocalNotification>> {
        self.db.local_notifications(include_acknowledged, limit)
    }

    pub fn acknowledge_notification(&mut self, id: &str) -> Result<LocalNotification> {
        self.db.acknowledge_notification(id, Utc::now())
    }

    pub fn request_run_cancellation(&mut self, run_id: &str, reason: &str) -> Result<bool> {
        self.db.request_run_cancellation(run_id, reason, Utc::now())
    }

    pub fn checkpoint_run_at(
        &mut self,
        run_id: &str,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
    ) -> Result<RunCheckpoint> {
        let (task_id, owner, generation) = self.db.run_lease_context(run_id)?;
        let task = self.db.task(&task_id)?;
        let schedule = self.evaluate_task_schedule_at(&task_id, now)?;
        let cancellation_requested = self.db.run_cancellation_requested(run_id)?;
        let quota = self
            .db
            .list_quota()?
            .into_iter()
            .filter(|surface| surface.provider == provider && surface.account == account)
            .collect::<Vec<_>>();
        let forecast = forecast_percent(&task);
        let policy_allowed = self.policy.allow_branch_changes
            && matches!(
                self.policy.authorize(task.risk_class, true),
                PolicyDecision::Allow
            );
        let quota_available = !quota.is_empty()
            && quota.iter().all(|surface| {
                (surface.override_reason.is_some()
                    || surface
                        .valid_until
                        .is_none_or(|valid_until| valid_until > now))
                    && surface
                        .effective_remaining_percent
                        .is_some_and(|remaining| remaining >= surface.reserve_percent + forecast)
            });
        let near_quota_boundary = quota_available
            && quota.iter().any(|surface| {
                surface
                    .effective_remaining_percent
                    .is_some_and(|remaining| remaining < surface.reserve_percent + (forecast * 2.0))
            });
        let (action, reason_code, interval_seconds) = if cancellation_requested {
            (CheckpointAction::Cancel, "cancel.requested", None)
        } else if !schedule.eligible {
            (CheckpointAction::Pause, schedule.reason_code.as_str(), None)
        } else if !policy_allowed {
            (CheckpointAction::Pause, "policy.revoked", None)
        } else if !quota_available {
            let reason = if quota.iter().any(|surface| {
                surface.override_reason.is_none()
                    && surface
                        .valid_until
                        .is_some_and(|valid_until| valid_until <= now)
            }) {
                "quota.stale"
            } else {
                "quota.insufficient"
            };
            (CheckpointAction::Pause, reason, None)
        } else if near_quota_boundary {
            (
                CheckpointAction::ShortenCheckpoint,
                "quota.near_reserve",
                Some((task.checkpoint_seconds / 2).max(1)),
            )
        } else {
            (
                CheckpointAction::Continue,
                "supervision.healthy",
                Some(task.checkpoint_seconds),
            )
        };
        let next_checkpoint_at =
            interval_seconds.map(|seconds| now + Duration::seconds(seconds as i64));
        self.db.apply_run_checkpoint(
            run_id,
            &owner,
            generation,
            now,
            std::time::Duration::from_secs(task.checkpoint_seconds.max(1)),
            action,
            reason_code,
            next_checkpoint_at,
            &serde_json::json!({
                "schedule": schedule,
                "quota": quota,
                "policy_hash": self.policy.hash(),
                "forecast_percent": forecast,
            }),
        )
    }

    pub fn supervise_invocation_for_run(
        &mut self,
        run_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
        invocation: &Invocation,
        cancelled: std::sync::Arc<AtomicBool>,
    ) -> Result<SupervisedInvocationResult> {
        let (task_id, _, _) = self.db.run_lease_context(run_id)?;
        let task = self.db.task(&task_id)?;
        let mut checkpoint_due =
            std::time::Instant::now() + std::time::Duration::from_secs(task.checkpoint_seconds);
        let outcome = run_invocation_with_tick(
            invocation,
            cancelled,
            std::time::Duration::from_secs(1),
            || {
                let cancellation_requested = self.db.run_cancellation_requested(run_id)?;
                if !cancellation_requested && std::time::Instant::now() < checkpoint_due {
                    return Ok(false);
                }
                let checkpoint = self.checkpoint_run_at(run_id, provider, account, Utc::now())?;
                let interval = checkpoint
                    .next_checkpoint_at
                    .map(|next| (next - checkpoint.evaluated_at).num_seconds().max(1) as u64)
                    .unwrap_or(task.checkpoint_seconds.max(1));
                checkpoint_due =
                    std::time::Instant::now() + std::time::Duration::from_secs(interval);
                Ok(matches!(
                    checkpoint.action,
                    CheckpointAction::Pause | CheckpointAction::Cancel
                ))
            },
        )?;
        let failure_category = match outcome.classification {
            ExitClassification::Success => None,
            ExitClassification::Failed => Some(FailureCategory::ProcessFailed),
            ExitClassification::TimedOut => Some(FailureCategory::TimedOut),
            ExitClassification::Cancelled => Some(FailureCategory::Cancelled),
            ExitClassification::Signalled => Some(FailureCategory::Signalled),
        };
        let now = Utc::now();
        let outcome_json = serde_json::to_value(&outcome)?;
        let termination_json = outcome
            .termination
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        let task_id = self.db.record_process_outcome(
            run_id,
            failure_category,
            outcome.exit_code,
            &outcome_json,
            termination_json.as_ref(),
            now,
        )?;
        let circuit = self.db.record_adapter_outcome(
            adapter,
            provider,
            account,
            failure_category,
            now,
            3,
            std::time::Duration::from_secs(300),
        )?;
        let retry = failure_category
            .filter(|failure| *failure != FailureCategory::Cancelled)
            .map(|failure| {
                self.db.plan_retry(
                    &task_id,
                    run_id,
                    failure,
                    now,
                    std::time::Duration::from_secs(30),
                    std::time::Duration::from_secs(1_800),
                )
            })
            .transpose()?;
        Ok(SupervisedInvocationResult {
            outcome,
            failure_category,
            retry,
            circuit,
        })
    }

    pub fn route_task(
        &mut self,
        task_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
    ) -> Result<RouteDecision> {
        self.route_task_at_mode(task_id, adapter, provider, account, Utc::now(), true, true)
    }

    pub fn route_task_at(
        &mut self,
        task_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
    ) -> Result<RouteDecision> {
        self.route_task_at_mode(task_id, adapter, provider, account, now, true, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn route_task_at_mode(
        &mut self,
        task_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
        claim_circuit_probe: bool,
        record_decision: bool,
    ) -> Result<RouteDecision> {
        let task = self.db.task(task_id)?;
        if !matches!(task.status, TaskStatus::Ready | TaskStatus::Draft) {
            bail!(
                "task must be ready or waiting on dependencies to route; current status is {}",
                task.status
            );
        }
        let dependencies_allowed = self.db.dependencies_satisfied(task_id)?;
        let pin_allowed = match (
            task.pinned_adapter.as_deref(),
            task.pinned_provider.as_deref(),
            task.pinned_account.as_deref(),
        ) {
            (None, None, None) => true,
            (Some(pinned_adapter), Some(pinned_provider), Some(pinned_account)) => {
                pinned_adapter == adapter
                    && pinned_provider == provider
                    && pinned_account == account
            }
            _ => bail!("task manual pin is incomplete in canonical state"),
        };
        let project = self.db.project(&task.project_id)?;
        let project_allowed = !project.scheduler_paused;
        let control = self.db.control_state()?;
        let operations_allowed = !control.pause_new_work && !control.emergency_stop;
        let schedule = self.evaluate_task_schedule_at(task_id, now)?;
        let retry = self.db.retry_state(task_id)?;
        let retry_allowed = retry
            .retry_not_before
            .is_none_or(|retry_at| retry_at <= now);
        let deadline_allowed = task.deadline_at.is_none_or(|deadline| now <= deadline);
        let (capability_allowed, capability_reason) =
            evaluate_adapter_capabilities(adapter, &task.required_capabilities);
        let (mut circuit_allowed, mut circuit_wake, mut circuit_reason) = self
            .db
            .adapter_circuit_gate(adapter, provider, account, now, false)?;
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
        let (quota_allowed, quota_reason_code, quota_reason) = if quota.is_empty() {
            (
                self.policy.unknown_quota_unattended,
                "quota.unavailable",
                "no quota surfaces are available for the selected account".to_owned(),
            )
        } else if let Some(surface) = quota.iter().find(|surface| {
            surface.override_reason.is_none()
                && surface
                    .valid_until
                    .is_some_and(|valid_until| valid_until <= now)
        }) {
            (
                false,
                "quota.stale",
                format!(
                    "quota_stale: {} expired at {}",
                    surface.surface,
                    surface.valid_until.expect("checked as present")
                ),
            )
        } else if let Some(surface) = quota.iter().find(|surface| {
            surface.effective_remaining_percent.is_none()
                || surface
                    .effective_remaining_percent
                    .is_some_and(|remaining| remaining < surface.reserve_percent + forecast)
        }) {
            let (reason_code, reason) = match surface.effective_remaining_percent {
                Some(remaining) => (
                    "quota.insufficient",
                    format!(
                        "quota_headroom: {} has {:.1}% remaining but {:.1}% is required",
                        surface.surface,
                        remaining,
                        surface.reserve_percent + forecast
                    ),
                ),
                None => (
                    "quota.unknown",
                    format!(
                        "quota_unknown: {} ({})",
                        surface.surface,
                        surface.unknown_reason.as_deref().unwrap_or("unspecified")
                    ),
                ),
            };
            (false, reason_code, reason)
        } else {
            (
                true,
                "route.allowed",
                "all quota surfaces satisfy reserve plus forecast".to_owned(),
            )
        };
        let (policy_allowed, policy_reason_code, policy_reason) = if !self
            .policy
            .allow_branch_changes
        {
            (
                false,
                "policy.git_branch_denied",
                "policy.git_branch_denied: project policy denies automated branch and worktree creation"
                    .to_owned(),
            )
        } else {
            match self.policy.authorize(task.risk_class, true) {
                PolicyDecision::Allow => {
                    (true, "route.allowed", "policy allows the task".to_owned())
                }
                PolicyDecision::RequireApproval => (
                    false,
                    "policy.approval_required",
                    format!(
                        "policy.approval_required: task risk class {} requires approval",
                        task.risk_class
                    ),
                ),
                PolicyDecision::Deny(reason) => {
                    (false, "policy.denied", format!("policy.denied: {reason}"))
                }
            }
        };
        if claim_circuit_probe
            && circuit_allowed
            && circuit_reason == "circuit.probe_available"
            && operations_allowed
            && project_allowed
            && dependencies_allowed
            && pin_allowed
            && schedule.eligible
            && deadline_allowed
            && retry_allowed
            && capability_allowed
            && policy_allowed
            && quota_allowed
        {
            (circuit_allowed, circuit_wake, circuit_reason) = self
                .db
                .adapter_circuit_gate(adapter, provider, account, now, true)?;
        }
        let allowed = schedule.eligible
            && operations_allowed
            && project_allowed
            && dependencies_allowed
            && pin_allowed
            && deadline_allowed
            && retry_allowed
            && circuit_allowed
            && capability_allowed
            && policy_allowed
            && quota_allowed;
        let (reason_code, reason) = if control.emergency_stop {
            (
                "operations.emergency_stop".to_owned(),
                format!(
                    "operations.emergency_stop: {}",
                    control.reason.as_deref().unwrap_or("emergency stop active")
                ),
            )
        } else if control.pause_new_work {
            (
                "operations.paused".to_owned(),
                format!(
                    "operations.paused: {}",
                    control.reason.as_deref().unwrap_or("new work is paused")
                ),
            )
        } else if !project_allowed {
            (
                "project.paused".to_owned(),
                format!(
                    "project.paused: {}",
                    project
                        .scheduler_pause_reason
                        .as_deref()
                        .unwrap_or("project scheduling is paused")
                ),
            )
        } else if !dependencies_allowed {
            (
                "dependency.incomplete".to_owned(),
                "dependency.incomplete: one or more prerequisite tasks are not completed".into(),
            )
        } else if !pin_allowed {
            (
                "manual_pin.mismatch".to_owned(),
                format!(
                    "manual_pin.mismatch: task is pinned to {}:{}:{}",
                    task.pinned_adapter.as_deref().unwrap_or("invalid"),
                    task.pinned_provider.as_deref().unwrap_or("invalid"),
                    task.pinned_account.as_deref().unwrap_or("invalid")
                ),
            )
        } else if !schedule.eligible {
            (
                schedule.reason_code.clone(),
                format!(
                    "{}: task affinity {} does not match {} day {}",
                    schedule.reason_code, schedule.affinity, schedule.day_kind, schedule.local_date
                ),
            )
        } else if !deadline_allowed {
            (
                "deadline.expired".to_owned(),
                format!(
                    "deadline.expired: task deadline {} has passed",
                    task.deadline_at.expect("checked as present")
                ),
            )
        } else if !retry_allowed {
            (
                "retry.backoff".to_owned(),
                format!(
                    "retry.backoff: retry is deferred until {}",
                    retry.retry_not_before.expect("checked as present")
                ),
            )
        } else if !circuit_allowed {
            (
                circuit_reason.clone(),
                format!("{circuit_reason}: adapter is temporarily unavailable"),
            )
        } else if !capability_allowed {
            ("capability.missing".to_owned(), capability_reason)
        } else if !policy_allowed {
            (policy_reason_code.to_owned(), policy_reason)
        } else if !quota_allowed {
            (quota_reason_code.to_owned(), quota_reason)
        } else {
            ("route.allowed".to_owned(), quota_reason)
        };
        let selected_adapter = allowed.then(|| adapter.to_owned());
        let selected_provider = allowed.then(|| provider.to_owned());
        let selected_account = allowed.then(|| account.to_owned());
        let quota_wake = (!quota_allowed)
            .then(|| quota.iter().filter_map(|surface| surface.reset_at).max())
            .flatten();
        let schedule_wake = (!schedule.eligible)
            .then_some(schedule.next_eligible_at)
            .flatten();
        let retry_wake = (!retry_allowed).then_some(retry.retry_not_before).flatten();
        let next_wake_at = [schedule_wake, quota_wake, retry_wake, circuit_wake]
            .into_iter()
            .flatten()
            .max();
        let minimum_effective_remaining_percent = quota
            .iter()
            .filter_map(|surface| surface.effective_remaining_percent)
            .min_by(f64::total_cmp);
        let decision = RouteDecision {
            id: Ulid::new().to_string(),
            task_id: task.id,
            selected_adapter,
            selected_provider,
            selected_account,
            allowed,
            reason_code: reason_code.clone(),
            reason: reason.clone(),
            required_headroom_percent: required_headroom,
            candidates: vec![RouteCandidate {
                adapter: adapter.to_owned(),
                provider: provider.to_owned(),
                account: account.to_owned(),
                allowed,
                reason_code: reason_code.clone(),
                filter_reason: reason.clone(),
                forecast_percent: forecast,
                minimum_effective_remaining_percent,
                score: None,
                score_components: None,
            }],
            next_wake_at,
            schedule: Some(schedule),
            quota,
            policy_hash: self.policy.hash(),
            created_at: now,
        };
        if record_decision {
            self.db.record_route(&decision)?;
        }
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
            .filter(|task| matches!(task.status, TaskStatus::Ready | TaskStatus::Draft))
            .collect::<Vec<_>>();
        let mut decisions = Vec::with_capacity(ready.len());
        for task in ready {
            decisions.push(
                self.route_task_at_mode(&task.id, adapter, provider, account, now, false, true)?,
            );
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
        if config.max_active_claims == 0
            || config.max_active_per_adapter == 0
            || config.max_active_per_account == 0
        {
            bail!("scheduler global, adapter, and account limits must be greater than zero");
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
        let route_targets = if config.route_candidates.is_empty() {
            vec![RouteTarget {
                adapter: config.adapter.clone(),
                provider: config.provider.clone(),
                account: config.account.clone(),
            }]
        } else {
            config.route_candidates.clone()
        };

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
        let mut runs_completed = 0;
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
                let tick = self.scheduler_tick_candidates_with_limits_at(
                    &config.instance_id,
                    leader.generation,
                    &route_targets,
                    tick_at,
                    config.max_active_claims,
                    config.max_active_per_adapter,
                    config.max_active_per_account,
                    config.claim_ttl,
                )?;
                ticks += 1;
                claims_created += tick.claims.len();
                if config.execute_fake_claims {
                    for claim in &tick.claims {
                        let route_decision_id =
                            claim.route_decision_id.as_deref().ok_or_else(|| {
                                anyhow::anyhow!("claim {} has no route decision", claim.id)
                            })?;
                        let route = tick
                            .decisions
                            .iter()
                            .find(|decision| decision.id == route_decision_id)
                            .cloned()
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "claim {} route decision is missing from its scheduler tick",
                                    claim.id
                                )
                            })?;
                        let selected_adapter = route.selected_adapter.clone().ok_or_else(|| {
                            anyhow::anyhow!("allowed route has no selected adapter")
                        })?;
                        if !selected_adapter.starts_with("fake") {
                            bail!(
                                "--execute-fake cannot execute selected real adapter {selected_adapter}"
                            );
                        }
                        self.run_scheduler_claim(
                            claim,
                            &config.instance_id,
                            leader.generation,
                            &selected_adapter,
                            route,
                            tick_at,
                        )?;
                        runs_completed += 1;
                    }
                }
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
                runs_completed,
                scheduler_claims_recovered,
                run_leases_recovered,
                released_task_ids,
                shutdown_reason,
            }),
        }
    }

    fn route_task_candidates_at(
        &mut self,
        task_id: &str,
        route_targets: &[RouteTarget],
        now: DateTime<Utc>,
    ) -> Result<(RouteDecision, Option<RouteTarget>)> {
        let task = self.db.task(task_id)?;
        let mut targets = route_targets.to_vec();
        for target in &targets {
            if [&target.adapter, &target.provider, &target.account]
                .iter()
                .any(|value| value.trim().is_empty() || value.chars().any(char::is_whitespace))
            {
                bail!("route candidate adapter, provider, and account must be non-empty names");
            }
        }
        targets.sort_by(|left, right| {
            (&left.adapter, &left.provider, &left.account).cmp(&(
                &right.adapter,
                &right.provider,
                &right.account,
            ))
        });
        targets.dedup();
        if targets.is_empty() {
            bail!("scheduler requires at least one route candidate");
        }

        let mut preliminary = Vec::with_capacity(targets.len());
        for target in &targets {
            preliminary.push((
                target.clone(),
                self.route_task_at_mode(
                    task_id,
                    &target.adapter,
                    &target.provider,
                    &target.account,
                    now,
                    false,
                    false,
                )?,
            ));
        }

        let allowed_targets = preliminary
            .iter()
            .filter(|(_, decision)| decision.allowed)
            .map(|(target, _)| target.clone())
            .collect::<Vec<_>>();
        if allowed_targets.is_empty() {
            let mut decision = preliminary[0].1.clone();
            let pin = task_route_pin(&task)?;
            if pin.as_ref().is_some_and(|pin| {
                !targets.iter().any(|target| {
                    target.adapter == pin.adapter
                        && target.provider == pin.provider
                        && target.account == pin.account
                })
            }) {
                decision.reason_code = "manual_pin.unavailable".into();
                decision.reason =
                    "manual_pin.unavailable: the pinned route is not configured for this scheduler"
                        .into();
            }
            decision.selected_adapter = None;
            decision.selected_provider = None;
            decision.selected_account = None;
            decision.allowed = false;
            decision.candidates = preliminary
                .iter()
                .map(|(_, candidate)| candidate.candidates[0].clone())
                .collect();
            self.db.record_route(&decision)?;
            return Ok((decision, None));
        }

        let quota = self.db.list_quota()?;
        let status = self
            .agent_capability_status_at(now)?
            .into_iter()
            .map(|entry| (entry.adapter.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        let mut inputs = Vec::with_capacity(allowed_targets.len());
        for target in &allowed_targets {
            let (freshness, health, capabilities) = if target.adapter.starts_with("fake") {
                let probe = AgentKind::Fake.probe();
                (
                    ProbeFreshness::Fresh,
                    AdapterHealth::Healthy,
                    probe.capabilities,
                )
            } else if let Some(entry) = status.get(&target.adapter) {
                let freshness = match entry.freshness.as_str() {
                    "fresh" => ProbeFreshness::Fresh,
                    "stale" => ProbeFreshness::Stale,
                    _ => ProbeFreshness::Unknown,
                };
                let health = adapter_health(&entry.health);
                let capabilities = entry
                    .probe
                    .as_ref()
                    .map(|probe| probe.capabilities.clone())
                    .unwrap_or_default();
                (freshness, health, capabilities)
            } else {
                (ProbeFreshness::Unknown, AdapterHealth::Unknown, vec![])
            };
            let surfaces = quota
                .iter()
                .filter(|surface| {
                    surface.provider == target.provider && surface.account == target.account
                })
                .collect::<Vec<_>>();
            let remaining_percent = surfaces
                .iter()
                .filter_map(|surface| surface.effective_remaining_percent)
                .min_by(|left, right| left.total_cmp(right));
            let reserve_percent = surfaces
                .iter()
                .map(|surface| surface.reserve_percent)
                .fold(self.policy.reserve_percent, f64::max);
            inputs.push(RoutingCandidateInput {
                identity: CandidateIdentity {
                    adapter: target.adapter.clone(),
                    provider: target.provider.clone(),
                    account: target.account.clone(),
                },
                freshness,
                health,
                capabilities,
                remaining_percent,
                reserve_percent,
                historical_success_percent: None,
                continuity: false,
                preference: 0.0,
            });
        }
        let selection = select_candidate(
            &RoutingRequest {
                required_capabilities: task.required_capabilities.clone(),
                forecast_percent: forecast_percent(&task),
                pin: task_route_pin(&task)?.map(|pin| CandidateIdentity {
                    adapter: pin.adapter,
                    provider: pin.provider,
                    account: pin.account,
                }),
            },
            &inputs,
        )?;
        let evaluation_by_identity = selection
            .evaluations
            .iter()
            .map(|evaluation| {
                (
                    (
                        evaluation.identity.adapter.clone(),
                        evaluation.identity.provider.clone(),
                        evaluation.identity.account.clone(),
                    ),
                    evaluation,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let selected_target = selection.selected.as_ref().and_then(|selected| {
            targets
                .iter()
                .find(|target| {
                    target.adapter == selected.adapter
                        && target.provider == selected.provider
                        && target.account == selected.account
                })
                .cloned()
        });
        let mut decision = if let Some(target) = selected_target.as_ref() {
            self.route_task_at_mode(
                task_id,
                &target.adapter,
                &target.provider,
                &target.account,
                now,
                true,
                false,
            )?
        } else {
            let mut denied = preliminary
                .iter()
                .find(|(_, decision)| decision.allowed)
                .expect("allowed targets came from preliminary decisions")
                .1
                .clone();
            denied.allowed = false;
            denied.selected_adapter = None;
            denied.selected_provider = None;
            denied.selected_account = None;
            denied.reason_code = selection.reason_code.clone();
            denied.reason = format!(
                "{}: no configured route candidate passed capability evidence and quota scoring",
                selection.reason_code
            );
            denied
        };
        decision.candidates = preliminary
            .iter()
            .map(|(target, preliminary_decision)| {
                let key = (
                    target.adapter.clone(),
                    target.provider.clone(),
                    target.account.clone(),
                );
                if let Some(evaluation) = evaluation_by_identity.get(&key) {
                    let score_components = evaluation.score.as_ref().map(|score| {
                        serde_json::json!({
                            "quota_margin": score.quota_margin,
                            "reliability": score.reliability,
                            "continuity_bonus": score.continuity_bonus,
                            "preference": score.preference,
                            "total": score.total,
                        })
                    });
                    RouteCandidate {
                        adapter: target.adapter.clone(),
                        provider: target.provider.clone(),
                        account: target.account.clone(),
                        allowed: evaluation.allowed,
                        reason_code: evaluation.reason_code.clone(),
                        filter_reason: evaluation.reason.clone(),
                        forecast_percent: forecast_percent(&task),
                        minimum_effective_remaining_percent: preliminary_decision.candidates[0]
                            .minimum_effective_remaining_percent,
                        score: evaluation.score.as_ref().map(|score| score.total),
                        score_components,
                    }
                } else {
                    preliminary_decision.candidates[0].clone()
                }
            })
            .collect();
        if !decision.allowed
            && let Some(target) = selected_target.as_ref()
            && let Some(candidate) = decision.candidates.iter_mut().find(|candidate| {
                candidate.adapter == target.adapter
                    && candidate.provider == target.provider
                    && candidate.account == target.account
            })
        {
            candidate.allowed = false;
            candidate.reason_code = decision.reason_code.clone();
            candidate.filter_reason = decision.reason.clone();
            candidate.score = None;
            candidate.score_components = None;
        }
        self.db.record_route(&decision)?;
        let selected_target = decision.allowed.then_some(selected_target).flatten();
        Ok((decision, selected_target))
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
        self.scheduler_tick_with_limits_at(
            instance_id,
            leader_generation,
            adapter,
            provider,
            account,
            now,
            max_active_claims,
            max_active_claims,
            max_active_claims,
            claim_ttl,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn scheduler_tick_with_limits_at(
        &mut self,
        instance_id: &str,
        leader_generation: i64,
        adapter: &str,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
        max_active_claims: usize,
        max_active_per_adapter: usize,
        max_active_per_account: usize,
        claim_ttl: std::time::Duration,
    ) -> Result<SchedulerTick> {
        self.scheduler_tick_candidates_with_limits_at(
            instance_id,
            leader_generation,
            &[RouteTarget {
                adapter: adapter.into(),
                provider: provider.into(),
                account: account.into(),
            }],
            now,
            max_active_claims,
            max_active_per_adapter,
            max_active_per_account,
            claim_ttl,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn scheduler_tick_candidates_with_limits_at(
        &mut self,
        instance_id: &str,
        leader_generation: i64,
        route_targets: &[RouteTarget],
        now: DateTime<Utc>,
        max_active_claims: usize,
        max_active_per_adapter: usize,
        max_active_per_account: usize,
        claim_ttl: std::time::Duration,
    ) -> Result<SchedulerTick> {
        if route_targets.is_empty() {
            bail!("scheduler requires at least one route candidate");
        }
        self.db.recover_expired_scheduler_claims(now)?;
        let ready = self
            .db
            .list_tasks(None)?
            .into_iter()
            .filter(|task| matches!(task.status, TaskStatus::Ready | TaskStatus::Draft))
            .collect::<Vec<_>>();
        let mut decisions = Vec::with_capacity(ready.len());
        let mut claims: Vec<SchedulerClaim> = Vec::new();
        for task in ready {
            let (decision, selected_target) =
                self.route_task_candidates_at(&task.id, route_targets, now)?;
            if !decision.allowed {
                self.db.record_scheduler_wake(
                    &task.id,
                    &decision.reason_code,
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
            let selected_target = selected_target.ok_or_else(|| {
                anyhow::anyhow!("allowed multi-candidate route has no selected target")
            })?;
            match self.db.claim_task_for_scheduler_with_route_limits(
                instance_id,
                leader_generation,
                &task.id,
                task.version,
                now,
                claim_ttl,
                max_active_claims,
                Some(&decision.id),
                &[],
                &selected_target.adapter,
                &selected_target.provider,
                &selected_target.account,
                max_active_per_adapter,
                max_active_per_account,
                forecast_percent(&task),
            ) {
                Ok(claim) => claims.push(claim),
                Err(error) => {
                    let message = error.to_string();
                    let reason_code = error
                        .downcast_ref::<SchedulerClaimRejection>()
                        .map_or("scheduler.claim_conflict", |rejection| {
                            rejection.reason_code()
                        });
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
        self.finish_fake_run(task, project, adapter, route, worktree, run_id)
    }

    fn run_scheduler_claim(
        &mut self,
        claim: &SchedulerClaim,
        instance_id: &str,
        leader_generation: i64,
        adapter: &str,
        route: RouteDecision,
        now: DateTime<Utc>,
    ) -> Result<RunSummary> {
        if !adapter.starts_with("fake") {
            bail!("scheduler claim execution only supports the quota-free fake adapter");
        }
        if claim.route_decision_id.as_deref() != Some(route.id.as_str()) {
            bail!("scheduler claim and route decision do not match");
        }
        if !self.policy.allow_branch_changes {
            bail!("project policy denies automated branch and worktree creation");
        }
        let task = self.db.task(&claim.task_id)?;
        if task.status != TaskStatus::Leased {
            bail!(
                "scheduler claim task must be leased; current status is {}",
                task.status
            );
        }
        let project = self.db.project(&task.project_id)?;
        let worktree_destination = self
            .data_dir
            .join("worktrees")
            .join(&project.slug)
            .join(&task.id);
        let worktree = match git::create_or_reuse_task_worktree(
            Path::new(&project.root_path),
            &worktree_destination,
            &task.id,
        ) {
            Ok(worktree) => worktree,
            Err(error) => {
                self.db.transition_task(
                    &task.id,
                    TaskStatus::Leased,
                    TaskStatus::Failed,
                    "worktree_failed",
                )?;
                return Err(error);
            }
        };
        let sandbox = FakeSandbox::attest(Path::new(&worktree.path));
        match self
            .policy
            .authorize(task.risk_class, sandbox.secure_container)
        {
            PolicyDecision::Allow => {}
            PolicyDecision::RequireApproval => bail!(
                "task risk class {} requires approval before claim consumption",
                task.risk_class
            ),
            PolicyDecision::Deny(reason) => bail!("policy denied task: {reason}"),
        }
        let run_id = Ulid::new().to_string();
        self.db.begin_claimed_run(
            &claim.id,
            instance_id,
            leader_generation,
            &run_id,
            adapter,
            &worktree.path,
            &worktree.branch,
            &worktree.base_commit,
            now,
            std::time::Duration::from_secs(task.checkpoint_seconds),
        )?;
        self.finish_fake_run(task, project, adapter, route, worktree, run_id)
    }

    fn finish_fake_run(
        &mut self,
        task: Task,
        project: Project,
        adapter: &str,
        route: RouteDecision,
        worktree: crate::git::Worktree,
        run_id: String,
    ) -> Result<RunSummary> {
        let sandbox = FakeSandbox::attest(Path::new(&worktree.path));
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
                &task.id,
                &run_id,
                "agent.file_written",
                "fake_agent",
                &serde_json::json!({"relative_path": relative, "target": target}),
            )?;
        }
        self.db.append_run_event(
            &task.id,
            &run_id,
            "run.checkpointed",
            "control_plane",
            &serde_json::json!({"checkpoint_seconds": task.checkpoint_seconds}),
        )?;
        self.db.transition_task(
            &task.id,
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
                &task.id,
                TaskStatus::Verifying,
                TaskStatus::Review,
                "verification_passed",
            )?;
            self.db
                .finish_run(&run_id, "review", Some(&head_commit), exit_code)?;
            self.db.enqueue_notification(
                "review",
                "info",
                Some(&task.id),
                Some(&run_id),
                "Task ready for review",
                &format!("{} passed independent verification.", task.title),
                Utc::now(),
            )?;
        } else {
            self.db.transition_task(
                &task.id,
                TaskStatus::Verifying,
                TaskStatus::Failed,
                "verification_failed",
            )?;
            self.db
                .finish_run(&run_id, "failed", Some(&head_commit), exit_code)?;
            self.db.enqueue_notification(
                "failure",
                "error",
                Some(&task.id),
                Some(&run_id),
                "Task verification failed",
                &format!("{} failed independent verification.", task.title),
                Utc::now(),
            )?;
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

    pub fn approvals(&self, limit: usize) -> Result<Vec<ApprovalRequest>> {
        self.db.list_approvals(limit)
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

fn validate_project_root_for_platform(root: &Path, wsl2: bool) -> Result<()> {
    if wsl2 && is_windows_mounted_path(root) {
        bail!(
            "wsl.windows_mount_denied: project roots under /mnt/<drive> are denied by default; clone the repository into the WSL2 Linux filesystem"
        );
    }
    Ok(())
}

fn is_windows_mounted_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(std::path::Component::RootDir))
        && components
            .next()
            .is_some_and(|component| component.as_os_str() == "mnt")
        && components.next().is_some_and(|component| {
            let value = component.as_os_str().to_string_lossy();
            value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic()
        })
}

fn is_wsl2() -> bool {
    std::env::var_os("WSL_INTEROP").is_some()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || fs::read_to_string("/proc/sys/kernel/osrelease")
            .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
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

fn task_route_pin(task: &Task) -> Result<Option<RouteTarget>> {
    match (
        task.pinned_adapter.as_ref(),
        task.pinned_provider.as_ref(),
        task.pinned_account.as_ref(),
    ) {
        (None, None, None) => Ok(None),
        (Some(adapter), Some(provider), Some(account)) => Ok(Some(RouteTarget {
            adapter: adapter.clone(),
            provider: provider.clone(),
            account: account.clone(),
        })),
        _ => bail!("task manual pin is incomplete in canonical state"),
    }
}

fn adapter_health(value: &str) -> AdapterHealth {
    match value {
        "healthy" => AdapterHealth::Healthy,
        "missing" => AdapterHealth::Missing,
        "unsupported" => AdapterHealth::Unsupported,
        "unhealthy" => AdapterHealth::Unhealthy,
        _ => AdapterHealth::Unknown,
    }
}

fn evaluate_adapter_capabilities(adapter: &str, required: &[String]) -> (bool, String) {
    if required.is_empty() {
        return (true, "no task-specific capabilities are required".into());
    }
    let kind = if adapter.starts_with("fake") {
        Some(AgentKind::Fake)
    } else {
        match adapter {
            "codex" => Some(AgentKind::Codex),
            "claude" => Some(AgentKind::Claude),
            "antigravity" => Some(AgentKind::Antigravity),
            _ => None,
        }
    };
    let Some(kind) = kind else {
        return (
            false,
            format!("capability.missing: adapter {adapter} is unknown"),
        );
    };
    let probe = kind.probe();
    if probe.health != "healthy" {
        return (
            false,
            format!(
                "capability.missing: adapter {adapter} is {} ({})",
                probe.health,
                probe.failure.as_deref().unwrap_or("no probe detail")
            ),
        );
    }
    let missing: Vec<_> = required
        .iter()
        .filter(|required| !probe.capabilities.contains(required))
        .cloned()
        .collect();
    if missing.is_empty() {
        (true, "adapter satisfies every required capability".into())
    } else {
        (
            false,
            format!(
                "capability.missing: adapter {adapter} lacks {}",
                missing.join(",")
            ),
        )
    }
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

    #[test]
    fn wsl2_denies_windows_mounted_project_roots_by_default() {
        assert!(validate_project_root_for_platform(Path::new("/mnt/c/dev/project"), true).is_err());
        assert!(validate_project_root_for_platform(Path::new("/mnt/z/project"), true).is_err());
        assert!(validate_project_root_for_platform(Path::new("/home/user/project"), true).is_ok());
        assert!(validate_project_root_for_platform(Path::new("/mnt/wsl/project"), true).is_ok());
        assert!(validate_project_root_for_platform(Path::new("/mnt/c/dev/project"), false).is_ok());
    }

    #[test]
    fn capability_matrix_distinguishes_unknown_stale_and_fresh_health() {
        let dir = tempdir().unwrap();
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let unknown = garnish.agent_capability_status_at(now).unwrap();
        assert!(unknown.iter().all(|entry| entry.freshness == "unknown"));

        garnish
            .db
            .record_agent_capability_probe(&AgentCapabilityProbe {
                id: "codex-stale".into(),
                adapter: "codex".into(),
                executable: Some("/fixture/codex".into()),
                version: Some("codex-cli 0.144.2".into()),
                health: "healthy".into(),
                capabilities: vec!["agent.headless".into()],
                failure: None,
                probed_at: now - Duration::minutes(10),
                valid_until: now - Duration::minutes(5),
            })
            .unwrap();
        garnish
            .db
            .record_agent_capability_probe(&AgentCapabilityProbe {
                id: "claude-fresh".into(),
                adapter: "claude".into(),
                executable: Some("/fixture/claude".into()),
                version: Some("2.1.215 (Claude Code)".into()),
                health: "unsupported".into(),
                capabilities: vec!["agent.headless".into()],
                failure: Some("fixture drift".into()),
                probed_at: now - Duration::minutes(1),
                valid_until: now + Duration::minutes(4),
            })
            .unwrap();

        let status = garnish.agent_capability_status_at(now).unwrap();
        let codex = status
            .iter()
            .find(|entry| entry.adapter == "codex")
            .unwrap();
        assert_eq!(codex.freshness, "stale");
        assert_eq!(codex.health, "healthy");
        let claude = status
            .iter()
            .find(|entry| entry.adapter == "claude")
            .unwrap();
        assert_eq!(claude.freshness, "fresh");
        assert_eq!(claude.health, "unsupported");
        let antigravity = status
            .iter()
            .find(|entry| entry.adapter == "antigravity")
            .unwrap();
        assert_eq!(antigravity.freshness, "unknown");
        assert_eq!(antigravity.health, "unknown");
    }

    #[test]
    fn capability_refresh_records_all_initial_agents_without_running_tasks() {
        let dir = tempdir().unwrap();
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let status = garnish
            .refresh_agent_capabilities_with(now, std::time::Duration::from_secs(300), |kind| {
                ProbeResult {
                    adapter: kind.key().into(),
                    executable: Some(format!("/fixture/{}", kind.key())),
                    version: Some("fixture 1.0.0".into()),
                    health: "healthy".into(),
                    capabilities: vec!["agent.headless".into()],
                    failure: None,
                }
            })
            .unwrap();
        assert_eq!(status.len(), 3);
        assert!(
            status
                .iter()
                .all(|entry| entry.freshness == "fresh" && entry.health == "healthy")
        );
        assert_eq!(
            garnish.db.latest_agent_capability_probes().unwrap().len(),
            3
        );
        assert!(
            garnish
                .refresh_agent_capabilities_with(
                    now,
                    std::time::Duration::ZERO,
                    |_| unreachable!(),
                )
                .is_err()
        );
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
            deadline_at: None,
            required_capabilities: vec![],
            pinned_adapter: None,
            pinned_provider: None,
            pinned_account: None,
            fake_write_path: Some("result.txt".into()),
            fake_write_content: Some("done\n".into()),
        }
    }

    fn prepare_active_run(garnish: &mut Garnish, task: &Task, run_id: &str, now: DateTime<Utc>) {
        let decision = RouteDecision {
            id: Ulid::new().to_string(),
            task_id: task.id.clone(),
            selected_adapter: Some("fake".into()),
            selected_provider: Some("fake".into()),
            selected_account: Some("test".into()),
            allowed: true,
            reason_code: "fixture.allowed".into(),
            reason: "fixture".into(),
            required_headroom_percent: 0.0,
            candidates: vec![],
            next_wake_at: None,
            schedule: None,
            quota: vec![],
            policy_hash: garnish.policy.hash(),
            created_at: now,
        };
        garnish.db.record_route(&decision).unwrap();
        garnish
            .db
            .transition_task(&task.id, TaskStatus::Ready, TaskStatus::Leased, "fixture")
            .unwrap();
        garnish
            .db
            .transition_task(
                &task.id,
                TaskStatus::Leased,
                TaskStatus::Planning,
                "fixture",
            )
            .unwrap();
        garnish
            .db
            .transition_task(
                &task.id,
                TaskStatus::Planning,
                TaskStatus::Running,
                "fixture",
            )
            .unwrap();
        garnish
            .db
            .create_run(
                run_id,
                &task.id,
                "fake",
                &decision.id,
                "/fixture/worktree",
                "fixture",
                "0123456789abcdef",
                now + Duration::minutes(10),
            )
            .unwrap();
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
        assert_eq!(declined.reason_code, "quota.insufficient");
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
        let notifications = garnish.local_notifications(false, 10).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].kind, "review");
        assert_eq!(notifications[0].task_id.as_deref(), Some(task.id.as_str()));
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
        assert_eq!(decision.reason_code, "quota.unavailable");
    }

    #[test]
    fn stale_provider_quota_fails_closed_but_a_live_override_remains_explicit() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id)).unwrap();
        let observed_at = Utc::now() - Duration::minutes(10);
        garnish
            .db
            .record_quota_observations(&[crate::quota::QuotaObservation {
                provider: "fake".into(),
                account: "test".into(),
                surface: "five_hour".into(),
                remaining_percent: Some(90.0),
                reserve_percent: 20.0,
                reset_at: None,
                source: "codexbar:oauth".into(),
                confidence: "provider_reported".into(),
                unknown_reason: None,
                observed_at,
                valid_until: observed_at + Duration::minutes(5),
                collector_contract: "codexbar-usage-json-v1".into(),
                provider_version: Some("fixture".into()),
                payload_sha256: "c".repeat(64),
            }])
            .unwrap();
        let stale = garnish
            .route_task(&task.id, "fake", "fake", "test")
            .unwrap();
        assert!(!stale.allowed);
        assert_eq!(stale.reason_code, "quota.stale");

        garnish
            .override_quota(
                "fake",
                "test",
                "five_hour",
                90.0,
                "fresh user observation",
                Some(Utc::now() + Duration::minutes(5)),
            )
            .unwrap();
        let overridden = garnish
            .route_task(&task.id, "fake", "fake", "test")
            .unwrap();
        assert!(overridden.allowed);
    }

    #[test]
    fn operational_pause_declines_routes_with_stable_reason() {
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
                "fixture",
                None,
            )
            .unwrap();
        garnish.pause_new_work("host maintenance").unwrap();
        let decision = garnish
            .route_task(&task.id, "fake", "fake", "test")
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.selected_adapter, None);
        assert_eq!(decision.reason_code, "operations.paused");
        assert!(decision.reason.starts_with("operations.paused:"));
        garnish.resume_operations("maintenance complete").unwrap();
        assert!(
            garnish
                .route_task(&task.id, "fake", "fake", "test")
                .unwrap()
                .allowed
        );
    }

    #[test]
    fn durable_manual_pin_rejects_mismatch_without_bypassing_route_gates() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let data_dir = dir.path().join("data");
        let mut garnish = Garnish::open(&data_dir).unwrap();
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
                "fixture",
                None,
            )
            .unwrap();
        let pinned = garnish
            .set_task_route_pin(
                &task.id,
                "fake-secondary",
                "fake",
                "test",
                "keep continuity",
            )
            .unwrap();
        assert_eq!(pinned.pinned_adapter.as_deref(), Some("fake-secondary"));
        drop(garnish);

        let mut reopened = Garnish::open(&data_dir).unwrap();
        let mismatch = reopened
            .route_task(&task.id, "fake", "fake", "test")
            .unwrap();
        assert!(!mismatch.allowed);
        assert_eq!(mismatch.reason_code, "manual_pin.mismatch");
        let allowed = reopened
            .route_task(&task.id, "fake-secondary", "fake", "test")
            .unwrap();
        assert!(allowed.allowed);
        let cleared = reopened
            .clear_task_route_pin(&task.id, "continuity no longer required")
            .unwrap();
        assert!(cleared.pinned_adapter.is_none());
        assert!(
            reopened
                .route_task(&task.id, "fake", "fake", "test")
                .unwrap()
                .allowed
        );
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
        let now = Utc::now();
        garnish
            .register_scheduler("policy-scheduler", "fixture", 1, now)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("policy-scheduler", now, std::time::Duration::from_secs(30))
            .unwrap();
        let tick = garnish
            .scheduler_tick_at(
                "policy-scheduler",
                leader.generation,
                "fake",
                "fake",
                "test",
                now,
                1,
                std::time::Duration::from_secs(30),
            )
            .unwrap();
        assert!(tick.claims.is_empty());
        let wakes = garnish.scheduler_wakes().unwrap();
        assert_eq!(wakes.len(), 1);
        assert_eq!(wakes[0].reason_code, "policy.git_branch_denied");
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
    fn runtime_checkpoint_pauses_when_task_day_changes() {
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
        let mut work = task(project.id);
        work.day_affinity = crate::domain::DayAffinity::Work;
        let work = garnish.add_task(&work).unwrap();
        let friday = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 24, 22, 59, 0).unwrap();
        prepare_active_run(&mut garnish, &work, "run-day-boundary", friday);
        let saturday = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 24, 23, 1, 0).unwrap();
        let checkpoint = garnish
            .checkpoint_run_at("run-day-boundary", "fake", "test", saturday)
            .unwrap();
        assert_eq!(checkpoint.action, CheckpointAction::Pause);
        assert_eq!(checkpoint.reason_code, "schedule.ineligible_off_day");
        assert_eq!(garnish.task(&work.id).unwrap().status, TaskStatus::Running);
        garnish
            .db
            .record_process_outcome(
                "run-day-boundary",
                Some(FailureCategory::Cancelled),
                None,
                &serde_json::json!({"classification": "cancelled"}),
                Some(&serde_json::json!({"term_sent": true})),
                saturday + Duration::seconds(1),
            )
            .unwrap();
        assert_eq!(garnish.task(&work.id).unwrap().status, TaskStatus::Paused);
    }

    #[test]
    fn runtime_checkpoint_adapts_to_mid_run_quota_changes() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
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
        let task = garnish.add_task(&task(project.id)).unwrap();
        let now = Utc::now();
        prepare_active_run(&mut garnish, &task, "run-quota", now);
        let healthy = garnish
            .checkpoint_run_at("run-quota", "fake", "test", now + Duration::seconds(1))
            .unwrap();
        assert_eq!(healthy.action, CheckpointAction::Continue);

        garnish
            .set_quota(
                "fake",
                "test",
                "five_hour",
                Some(21.5),
                20.0,
                None,
                "fixture",
                None,
            )
            .unwrap();
        let shortened = garnish
            .checkpoint_run_at("run-quota", "fake", "test", now + Duration::seconds(2))
            .unwrap();
        assert_eq!(shortened.action, CheckpointAction::ShortenCheckpoint);

        garnish
            .set_quota(
                "fake",
                "test",
                "five_hour",
                Some(20.5),
                20.0,
                None,
                "fixture",
                None,
            )
            .unwrap();
        let paused = garnish
            .checkpoint_run_at("run-quota", "fake", "test", now + Duration::seconds(3))
            .unwrap();
        assert_eq!(paused.action, CheckpointAction::Pause);
        assert_eq!(paused.reason_code, "quota.insufficient");
    }

    #[test]
    fn scheduler_respects_persisted_retry_not_before() {
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
                "fixture",
                None,
            )
            .unwrap();
        garnish
            .db
            .transition_task(&task.id, TaskStatus::Ready, TaskStatus::Leased, "fixture")
            .unwrap();
        garnish
            .db
            .transition_task(&task.id, TaskStatus::Leased, TaskStatus::Failed, "fixture")
            .unwrap();
        let now = Utc::now();
        let plan = garnish
            .db
            .plan_retry(
                &task.id,
                "run-retry",
                FailureCategory::Infrastructure,
                now,
                std::time::Duration::from_secs(30),
                std::time::Duration::from_secs(300),
            )
            .unwrap();
        let decision = garnish
            .route_task_at(&task.id, "fake", "fake", "test", now)
            .unwrap();
        assert!(!decision.allowed);
        assert_eq!(decision.reason_code, "retry.backoff");
        assert!(decision.reason.starts_with("retry.backoff"));
        assert_eq!(decision.next_wake_at, plan.retry_at);
    }

    #[test]
    fn scheduler_records_dependency_project_deadline_and_capability_exclusions() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(&data_dir).unwrap();

        let dependency_root = dir.path().join("dependency");
        let paused_root = dir.path().join("paused");
        let deadline_root = dir.path().join("deadline");
        let capability_root = dir.path().join("capability");
        for root in [
            &dependency_root,
            &paused_root,
            &deadline_root,
            &capability_root,
        ] {
            fixture_repo(root);
        }

        let dependency_project = garnish
            .add_project("dependency", "Dependency", &dependency_root)
            .unwrap();
        let prerequisite = garnish
            .add_task(&task(dependency_project.id.clone()))
            .unwrap();
        let mut dependent_spec = task(dependency_project.id);
        dependent_spec.title = "Dependent".into();
        dependent_spec.dependencies = vec![prerequisite.id.clone()];
        let dependent = garnish.add_task(&dependent_spec).unwrap();
        assert_eq!(dependent.status, TaskStatus::Draft);

        let paused_project = garnish
            .add_project("paused", "Paused", &paused_root)
            .unwrap();
        let paused_task = garnish.add_task(&task(paused_project.id.clone())).unwrap();
        garnish
            .set_project_scheduler_pause(&paused_project.id, true, "project maintenance")
            .unwrap();

        let deadline_project = garnish
            .add_project("deadline", "Deadline", &deadline_root)
            .unwrap();
        let mut deadline_spec = task(deadline_project.id);
        deadline_spec.deadline_at = Some(now - Duration::seconds(1));
        let expired_task = garnish.add_task(&deadline_spec).unwrap();

        let capability_project = garnish
            .add_project("capability", "Capability", &capability_root)
            .unwrap();
        let mut capability_spec = task(capability_project.id);
        capability_spec.required_capabilities = vec!["agent.nonexistent".into()];
        let capability_task = garnish.add_task(&capability_spec).unwrap();

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
        garnish
            .register_scheduler("matrix", "fixture", 1, now)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("matrix", now, std::time::Duration::from_secs(60))
            .unwrap();
        let tick = garnish
            .scheduler_tick_at(
                "matrix",
                leader.generation,
                "fake",
                "fake",
                "test",
                now,
                10,
                std::time::Duration::from_secs(60),
            )
            .unwrap();
        let wakes = garnish.scheduler_wakes().unwrap();
        for (task_id, expected_code) in [
            (&dependent.id, "dependency.incomplete"),
            (&paused_task.id, "project.paused"),
            (&expired_task.id, "deadline.expired"),
            (&capability_task.id, "capability.missing"),
        ] {
            let wake = wakes.iter().find(|wake| wake.task_id == *task_id).unwrap();
            assert_eq!(wake.reason_code, expected_code);
            let decision = tick
                .decisions
                .iter()
                .find(|decision| decision.task_id == *task_id)
                .unwrap();
            assert_eq!(decision.reason_code, expected_code);
            assert!(!decision.allowed);
        }
    }

    #[test]
    fn priority_and_deadline_order_survives_restart() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let data_dir = dir.path().join("data");
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(&data_dir).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();

        let mut no_deadline = task(project.id.clone());
        no_deadline.title = "No deadline".into();
        let no_deadline = garnish.add_task(&no_deadline).unwrap();
        let mut later = task(project.id.clone());
        later.title = "Later".into();
        later.deadline_at = Some(now + Duration::hours(2));
        let later = garnish.add_task(&later).unwrap();
        let mut earlier = task(project.id.clone());
        earlier.title = "Earlier".into();
        earlier.deadline_at = Some(now + Duration::hours(1));
        let earlier = garnish.add_task(&earlier).unwrap();
        let mut high_priority = task(project.id);
        high_priority.title = "High priority".into();
        high_priority.priority = 20;
        let high_priority = garnish.add_task(&high_priority).unwrap();
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
        drop(garnish);

        let mut reopened = Garnish::open(&data_dir).unwrap();
        let preview = reopened
            .scheduler_preview_at("fake", "fake", "test", now)
            .unwrap();
        assert_eq!(
            preview
                .decisions
                .iter()
                .map(|decision| decision.task_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                high_priority.id.as_str(),
                earlier.id.as_str(),
                later.id.as_str(),
                no_deadline.id.as_str(),
            ]
        );
    }

    #[test]
    fn scheduler_records_project_resource_lock_exclusion() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let mut first_spec = task(project.id.clone());
        first_spec.title = "First".into();
        first_spec.priority = 20;
        let first = garnish.add_task(&first_spec).unwrap();
        let mut second_spec = task(project.id);
        second_spec.title = "Second".into();
        second_spec.priority = 10;
        let second = garnish.add_task(&second_spec).unwrap();
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
        garnish
            .register_scheduler("lock-matrix", "fixture", 1, now)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("lock-matrix", now, std::time::Duration::from_secs(60))
            .unwrap();
        let tick = garnish
            .scheduler_tick_at(
                "lock-matrix",
                leader.generation,
                "fake",
                "fake",
                "test",
                now,
                2,
                std::time::Duration::from_secs(60),
            )
            .unwrap();

        assert_eq!(tick.claims.len(), 1);
        assert_eq!(tick.claims[0].task_id, first.id);
        let wakes = garnish.scheduler_wakes().unwrap();
        let wake = wakes.iter().find(|wake| wake.task_id == second.id).unwrap();
        assert_eq!(wake.reason_code, "scheduler.resource_locked");
    }

    #[test]
    fn scheduler_selects_scored_candidate_and_honors_durable_exact_pin() {
        let dir = tempdir().unwrap();
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let unpinned_root = dir.path().join("unpinned");
        let pinned_root = dir.path().join("pinned");
        let unavailable_root = dir.path().join("unavailable");
        for root in [&unpinned_root, &pinned_root, &unavailable_root] {
            fixture_repo(root);
        }
        let unpinned_project = garnish
            .add_project("unpinned", "Unpinned", &unpinned_root)
            .unwrap();
        let pinned_project = garnish
            .add_project("pinned", "Pinned", &pinned_root)
            .unwrap();
        let unavailable_project = garnish
            .add_project("unavailable", "Unavailable", &unavailable_root)
            .unwrap();
        let unpinned = garnish.add_task(&task(unpinned_project.id)).unwrap();
        let pinned = garnish.add_task(&task(pinned_project.id)).unwrap();
        let unavailable = garnish.add_task(&task(unavailable_project.id)).unwrap();
        garnish
            .set_task_route_pin(
                &pinned.id,
                "fake-low",
                "fake",
                "low",
                "preserve account continuity",
            )
            .unwrap();
        garnish
            .set_task_route_pin(
                &unavailable.id,
                "fake-missing",
                "fake",
                "missing",
                "operator pin",
            )
            .unwrap();
        for (account, remaining) in [("low", 40.0), ("high", 90.0)] {
            garnish
                .set_quota(
                    "fake",
                    account,
                    "five_hour",
                    Some(remaining),
                    20.0,
                    None,
                    "fixture",
                    None,
                )
                .unwrap();
        }
        garnish
            .register_scheduler("multi-route", "fixture", 1, now)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("multi-route", now, std::time::Duration::from_secs(60))
            .unwrap();
        let targets = vec![
            RouteTarget {
                adapter: "fake-low".into(),
                provider: "fake".into(),
                account: "low".into(),
            },
            RouteTarget {
                adapter: "fake-high".into(),
                provider: "fake".into(),
                account: "high".into(),
            },
        ];
        let tick = garnish
            .scheduler_tick_candidates_with_limits_at(
                "multi-route",
                leader.generation,
                &targets,
                now,
                3,
                3,
                3,
                std::time::Duration::from_secs(60),
            )
            .unwrap();

        assert_eq!(tick.claims.len(), 2);
        let unpinned_decision = tick
            .decisions
            .iter()
            .find(|decision| decision.task_id == unpinned.id)
            .unwrap();
        assert_eq!(
            unpinned_decision.selected_adapter.as_deref(),
            Some("fake-high")
        );
        assert_eq!(unpinned_decision.selected_account.as_deref(), Some("high"));
        assert_eq!(unpinned_decision.candidates.len(), 2);
        assert!(
            unpinned_decision
                .candidates
                .iter()
                .all(|candidate| candidate.score.is_some())
        );
        let pinned_decision = tick
            .decisions
            .iter()
            .find(|decision| decision.task_id == pinned.id)
            .unwrap();
        assert_eq!(
            pinned_decision.selected_adapter.as_deref(),
            Some("fake-low")
        );
        assert_eq!(pinned_decision.selected_account.as_deref(), Some("low"));
        let unavailable_decision = tick
            .decisions
            .iter()
            .find(|decision| decision.task_id == unavailable.id)
            .unwrap();
        assert!(!unavailable_decision.allowed);
        assert_eq!(unavailable_decision.reason_code, "manual_pin.unavailable");
        let wake = garnish
            .scheduler_wakes()
            .unwrap()
            .into_iter()
            .find(|wake| wake.task_id == unavailable.id)
            .unwrap();
        assert_eq!(wake.reason_code, "manual_pin.unavailable");
    }

    #[cfg(unix)]
    #[test]
    fn supervised_invocation_acknowledges_durable_cancellation_after_process_exit() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
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
        let mut work = task(project.id);
        work.checkpoint_seconds = 1;
        let work = garnish.add_task(&work).unwrap();
        let now = Utc::now();
        prepare_active_run(&mut garnish, &work, "run-cancel-process", now);
        assert!(
            garnish
                .request_run_cancellation("run-cancel-process", "fixture cancellation")
                .unwrap()
        );
        let invocation = Invocation {
            executable: PathBuf::from("/bin/sh"),
            argv: vec!["-c".into(), "sleep 5".into()],
            cwd: source,
            environment: std::collections::BTreeMap::new(),
            stdin: vec![],
            structured_protocol: None,
            timeout: std::time::Duration::from_secs(10),
            output_limit: 1_024,
        };
        let result = garnish
            .supervise_invocation_for_run(
                "run-cancel-process",
                "fake",
                "fake",
                "test",
                &invocation,
                std::sync::Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        assert_eq!(result.outcome.classification, ExitClassification::Cancelled);
        assert!(result.outcome.termination.is_some());
        assert!(result.retry.is_none());
        assert_eq!(
            garnish.task(&work.id).unwrap().status,
            TaskStatus::Cancelled
        );
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
            route_candidates: vec![],
            max_active_claims: 1,
            max_active_per_adapter: 1,
            max_active_per_account: 1,
            poll_interval: std::time::Duration::from_secs(1),
            leader_ttl: std::time::Duration::from_secs(10),
            claim_ttl: std::time::Duration::from_secs(10),
            max_ticks: Some(2),
            execute_fake_claims: false,
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

    #[test]
    fn daemon_can_consume_a_claim_and_complete_quota_free_fake_execution() {
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
            instance_id: "daemon-executor".into(),
            hostname: "fixture".into(),
            adapter: "fake".into(),
            provider: "fake".into(),
            account: "test".into(),
            route_candidates: vec![],
            max_active_claims: 1,
            max_active_per_adapter: 1,
            max_active_per_account: 1,
            poll_interval: std::time::Duration::from_secs(1),
            leader_ttl: std::time::Duration::from_secs(10),
            claim_ttl: std::time::Duration::from_secs(10),
            max_ticks: Some(1),
            execute_fake_claims: true,
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

        assert_eq!(summary.ticks, 1);
        assert_eq!(summary.claims_created, 1);
        assert_eq!(summary.runs_completed, 1);
        assert!(summary.released_task_ids.is_empty());
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Review);
        assert!(
            dir.path()
                .join("data/worktrees/fixture")
                .join(&task.id)
                .join("result.txt")
                .exists()
        );
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
