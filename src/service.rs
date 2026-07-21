use crate::{
    adapters::{
        AgentKind, FakeSandbox, Invocation, ProbeResult, SandboxAttestation, probe_aoe,
        probe_docker, probe_podman, run_invocation_with_tick, safe_write,
    },
    api_providers::{
        ApiFailureKind, ApiOutputItem, ApiProviderResponse, ApiRequestSpec, ApiTerminalStatus,
        ApiToolDefinition, ApiTransport, LiveApiTransport, LiveApiTransportConfig,
        PreparedApiRequest, api_request_conservative_content_token_bound,
        api_request_conservative_input_token_bound, api_request_content_digest, api_request_digest,
        parse_api_transport_response, prepare_api_request,
    },
    db::Database,
    domain::{
        AgentCapabilityProbe, AgentCapabilityStatus, ApiBudget, ApiBudgetReservation,
        ApiClaimReservationRequest, ApiDispatchAttempt, ApiModelPrice, ApiRequestPlan,
        ApiReservationRequest, ApiSettlement, ApiSpend, ApprovalRequest, BackupRecord,
        CalendarException, CalendarProfile, CheckpointAction, CircuitBreaker, ControlState,
        DayKind, EmergencyStopResult, FailureCategory, LocalNotification, McpServerRevision,
        NewApiBudget, NewApiModelPrice, NewApiRequestPlan, NewMcpServerRevision, NewTask, Project,
        ProjectLink, QuotaCollectionAttempt, QuotaReservation, QuotaSurface, QuotaUsageSample,
        RetryPlan, RetryState, RouteCandidate, RouteDecision, RouteTarget, RunCheckpoint,
        RunRecord, RunSummary, ScheduleEvaluation, SchedulerApiClaim, SchedulerClaim,
        SchedulerClaimRejection, SchedulerDaemonConfig, SchedulerDaemonSummary, SchedulerLeader,
        SchedulerPreview, SchedulerTick, SchedulerWake, Task, TaskStatus, UsageForecast,
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
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use ulid::Ulid;

const API_TASK_TEMPLATE_VERSION: &str = "task-v2";
const API_PATCH_CAPABILITY: &str = "agent.patch_submission";
const API_PATCH_TOOL: &str = "submit_patch";
const MAX_API_PATCH_BYTES: usize = 1024 * 1024;
pub const PAID_API_DAEMON_ACKNOWLEDGEMENT: &str = "I_ACCEPT_PAID_API_TASK_EXECUTION";
pub const API_PATCH_DAEMON_ACKNOWLEDGEMENT: &str = "I_ACCEPT_ISOLATED_API_PATCH_EXECUTION";

pub struct Garnish {
    data_dir: PathBuf,
    db: Database,
    policy: EffectivePolicy,
    api_patch_execution_enabled: bool,
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

#[derive(Debug, Clone)]
pub struct ApiExecutionResult {
    pub response: ApiProviderResponse,
    pub spend: ApiSpend,
    pub attempts: Vec<ApiDispatchAttempt>,
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
            api_patch_execution_enabled: false,
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
            schema_version: 20,
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

    pub fn configure_api_budget(&mut self, config: &NewApiBudget) -> Result<ApiBudget> {
        let project = self.db.project(&config.project_id)?;
        let mut config = config.clone();
        config.project_id = project.id;
        self.db.configure_api_budget(&config)
    }

    pub fn configure_mcp_server(
        &mut self,
        config: &NewMcpServerRevision,
    ) -> Result<McpServerRevision> {
        let project = self.db.project(&config.project_id)?;
        let mut config = config.clone();
        config.project_id = project.id;
        config.allowed_tools.sort();
        config.allowed_tools.dedup();
        config.network_hosts = config
            .network_hosts
            .into_iter()
            .map(|host| host.to_ascii_lowercase())
            .collect();
        config.network_hosts.sort();
        config.network_hosts.dedup();
        config.secret_references.sort();
        config.secret_references.dedup();
        self.db.configure_mcp_server(&config)
    }

    pub fn mcp_servers(&self, project: Option<&str>) -> Result<Vec<McpServerRevision>> {
        let project_id = project
            .map(|value| self.db.project(value).map(|p| p.id))
            .transpose()?;
        self.db.list_latest_mcp_servers(project_id.as_deref())
    }

    pub fn api_budgets(&self, project: Option<&str>) -> Result<Vec<ApiBudget>> {
        let project_id = project
            .map(|value| self.db.project(value).map(|project| project.id))
            .transpose()?;
        self.db.list_latest_api_budgets(project_id.as_deref())
    }

    pub fn configure_api_request_plan(
        &mut self,
        config: &NewApiRequestPlan,
    ) -> Result<ApiRequestPlan> {
        self.configure_api_request_plan_at(config, Utc::now())
    }

    pub fn configure_api_request_plan_at(
        &mut self,
        config: &NewApiRequestPlan,
        now: DateTime<Utc>,
    ) -> Result<ApiRequestPlan> {
        let task = self.db.task(&config.task_id)?;
        if task.pinned_adapter.as_deref() != Some("api")
            || task.pinned_provider.as_deref() != Some(config.provider.as_str())
            || task.pinned_account.as_deref() != Some(config.account.as_str())
        {
            bail!("api.explicit_selection_required: task must be pinned to this paid API identity");
        }
        let budget =
            self.db
                .latest_api_budget(&task.project_id, &config.provider, &config.account)?;
        let spec = render_task_api_request(
            &task,
            &config.provider,
            &config.model,
            config.max_output_tokens,
            config.stream,
        )?;
        let conservative_input = if config.enabled {
            if !budget.allowed_roles.contains(&config.role) {
                bail!("api.role_denied: role is not in the project allowlist");
            }
            if config.max_retries > budget.max_retries {
                bail!("api.retry_limit: request plan exceeds the project retry ceiling");
            }
            api_request_conservative_input_token_bound(&budget, &spec, now)?
        } else {
            api_request_conservative_content_token_bound(&spec)?
        };
        if config.max_input_tokens < conservative_input {
            bail!(
                "api.input_limit: request plan reserves {} input tokens but at least {} are required",
                config.max_input_tokens,
                conservative_input
            );
        }
        let digest = if config.enabled {
            api_request_digest(&budget, &spec, now)?
        } else {
            api_request_content_digest(&spec)?
        };
        self.db.configure_api_request_plan(
            config,
            task.version,
            API_TASK_TEMPLATE_VERSION,
            &digest,
            now,
        )
    }

    pub fn api_request_plans(&self, task: Option<&str>) -> Result<Vec<ApiRequestPlan>> {
        let task_id = task
            .map(|value| self.db.task(value).map(|task| task.id))
            .transpose()?;
        self.db.list_latest_api_request_plans(task_id.as_deref())
    }

    fn planned_api_request_at(
        &self,
        task: &Task,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
    ) -> Result<(ApiRequestPlan, ApiRequestSpec, ApiBudget)> {
        self.planned_api_request_for_version_at(task, task.version, provider, account, now)
    }

    fn planned_api_request_for_version_at(
        &self,
        task: &Task,
        request_task_version: i64,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
    ) -> Result<(ApiRequestPlan, ApiRequestSpec, ApiBudget)> {
        let plan = self.db.latest_api_request_plan(&task.id)?;
        if !plan.enabled {
            bail!("api.request_plan_disabled: the latest request plan is disabled");
        }
        if plan.task_version != request_task_version {
            bail!(
                "api.request_plan_stale: plan task version {} differs from request version {}",
                plan.task_version,
                request_task_version
            );
        }
        if plan.template_version != API_TASK_TEMPLATE_VERSION {
            bail!("api.request_plan_template_unknown: request plan template is unsupported");
        }
        if plan.provider != provider || plan.account != account {
            bail!("api.request_plan_identity_mismatch: plan differs from the selected API route");
        }
        let budget = self
            .db
            .latest_api_budget(&task.project_id, provider, account)?;
        if !budget.allowed_roles.contains(&plan.role) {
            bail!("api.role_denied: role is not in the project allowlist");
        }
        if plan.max_retries > budget.max_retries {
            bail!("api.retry_limit: request plan exceeds the project retry ceiling");
        }
        let mut request_task = task.clone();
        request_task.version = request_task_version;
        let spec = render_task_api_request(
            &request_task,
            provider,
            &plan.model,
            plan.max_output_tokens,
            plan.stream,
        )?;
        let conservative_input = api_request_conservative_input_token_bound(&budget, &spec, now)?;
        if conservative_input > plan.max_input_tokens {
            bail!("api.request_plan_input_stale: rendered request exceeds its input reservation");
        }
        let digest = api_request_digest(&budget, &spec, now)?;
        if digest != plan.request_digest {
            bail!("api.request_plan_digest_mismatch: rendered request differs from its plan");
        }
        Ok((plan, spec, budget))
    }

    pub fn api_reservations(&self, project: Option<&str>) -> Result<Vec<ApiBudgetReservation>> {
        let project_id = project
            .map(|value| self.db.project(value).map(|project| project.id))
            .transpose()?;
        self.db.list_api_reservations(project_id.as_deref())
    }

    pub fn api_dispatch_attempts(&self, project: Option<&str>) -> Result<Vec<ApiDispatchAttempt>> {
        let project_id = project
            .map(|value| self.db.project(value).map(|project| project.id))
            .transpose()?;
        self.db.list_api_dispatch_attempts(project_id.as_deref())
    }

    pub fn api_spend(&self, project: Option<&str>) -> Result<Vec<ApiSpend>> {
        let project_id = project
            .map(|value| self.db.project(value).map(|project| project.id))
            .transpose()?;
        self.db.list_api_spend(project_id.as_deref())
    }

    pub fn configure_api_model_price(
        &mut self,
        config: &NewApiModelPrice,
    ) -> Result<ApiModelPrice> {
        self.db.configure_api_model_price(config)
    }

    pub fn api_model_prices(&self) -> Result<Vec<ApiModelPrice>> {
        self.db.list_api_model_prices()
    }

    pub fn reserve_api_budget(
        &mut self,
        request: &ApiReservationRequest,
    ) -> Result<ApiBudgetReservation> {
        if !self.policy.api_allowed(&request.provider) {
            bail!("api.policy_disabled: effective policy disables this API provider");
        }
        self.db.reserve_api_budget(request)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim_exact_api_request_at(
        &mut self,
        instance_id: &str,
        leader_generation: i64,
        task_id: &str,
        provider: &str,
        account: &str,
        role: &str,
        spec: &ApiRequestSpec,
        reserved_input_tokens: u64,
        now: DateTime<Utc>,
        claim_ttl: std::time::Duration,
        max_active_claims: usize,
        max_active_per_adapter: usize,
        max_active_per_account: usize,
    ) -> Result<SchedulerApiClaim> {
        if spec.provider != provider {
            bail!("api.provider_mismatch: request provider differs from its exact route");
        }
        if reserved_input_tokens == 0 {
            bail!("api.input_reservation_required: exact API claim requires input-token headroom");
        }
        let task = self.db.task(task_id)?;
        if task.pinned_adapter.as_deref() != Some("api")
            || task.pinned_provider.as_deref() != Some(provider)
            || task.pinned_account.as_deref() != Some(account)
        {
            bail!("api.explicit_selection_required: task must be pinned to this paid API identity");
        }
        let budget = self
            .db
            .latest_api_budget(&task.project_id, provider, account)?;
        let request_digest = api_request_digest(&budget, spec, now)?;
        let reserved_currency_micros = match budget.currency.as_deref() {
            Some(currency) => {
                let price = self.db.effective_api_model_price(
                    provider,
                    account,
                    &spec.model,
                    currency,
                    now,
                )?;
                worst_case_api_cost_micros(&price, reserved_input_tokens, spec.max_output_tokens)?
            }
            None => 0,
        };
        let decision = self.route_task_at(task_id, "api", provider, account, now)?;
        if !decision.allowed {
            bail!("{}", decision.reason);
        }
        let (claim, reservation) = self.db.claim_task_for_scheduler_with_api_reservation(
            instance_id,
            leader_generation,
            task_id,
            task.version,
            now,
            claim_ttl,
            max_active_claims,
            &decision.id,
            &[],
            provider,
            account,
            max_active_per_adapter,
            max_active_per_account,
            &ApiClaimReservationRequest {
                model: spec.model.clone(),
                role: role.into(),
                request_digest,
                reserved_currency_micros,
                reserved_input_tokens,
                reserved_output_tokens: spec.max_output_tokens,
                reserved_attempts: budget.max_retries + 1,
            },
        )?;
        Ok(SchedulerApiClaim { claim, reservation })
    }

    pub fn prepare_reserved_api_request(
        &self,
        reservation_id: &str,
        spec: &ApiRequestSpec,
        now: DateTime<Utc>,
    ) -> Result<PreparedApiRequest> {
        let reservation = self.db.api_reservation(reservation_id)?;
        if !self.policy.api_allowed(&reservation.provider) {
            bail!("api.policy_disabled: effective policy disables this API provider");
        }
        let retry_resume = if reservation.status == "dispatched" {
            self.db
                .latest_api_dispatch_attempt(reservation_id)?
                .is_some_and(|attempt| {
                    attempt.status == "retryable_failure"
                        && attempt.attempt_number < reservation.reserved_requests
                })
        } else {
            false
        };
        if (reservation.status != "active" && !retry_resume) || reservation.expires_at <= now {
            bail!("api.reservation_inactive: request requires a live dispatchable reservation");
        }
        if reservation.claim_id.is_some() && reservation.run_id.is_none() {
            bail!(
                "api.claim_not_consumed: scheduler-bound request cannot prepare before its run starts"
            );
        }
        if spec.provider != reservation.provider || spec.model != reservation.model {
            bail!("api.reservation_mismatch: provider or model differs from the reservation");
        }
        if spec.max_output_tokens != reservation.per_attempt_output_tokens {
            bail!("api.reservation_mismatch: output maximum differs from the reservation");
        }
        let latest = self.db.latest_api_budget(
            &reservation.project_id,
            &reservation.provider,
            &reservation.account,
        )?;
        if latest.id != reservation.budget_id {
            bail!("api.budget_superseded: reservation budget is no longer the latest revision");
        }
        prepare_api_request(&latest, spec, now, &reservation.request_digest)
    }

    pub fn claim_api_dispatch(
        &mut self,
        reservation_id: &str,
        now: DateTime<Utc>,
    ) -> Result<ApiBudgetReservation> {
        self.db.claim_api_dispatch(reservation_id, now)
    }

    pub fn execute_reserved_api_request<T: ApiTransport + ?Sized>(
        &mut self,
        reservation_id: &str,
        spec: &ApiRequestSpec,
        transport: &mut T,
        now: DateTime<Utc>,
    ) -> Result<ApiExecutionResult> {
        let prepared = self.prepare_reserved_api_request(reservation_id, spec, now)?;
        let active_reservation = self.db.api_reservation(reservation_id)?;
        let budget = self.db.api_budget(&active_reservation.budget_id)?;
        let pricing_evidence = budget
            .currency
            .as_deref()
            .map(|currency| {
                self.db.effective_api_model_price(
                    &active_reservation.provider,
                    &active_reservation.account,
                    &active_reservation.model,
                    currency,
                    now,
                )
            })
            .transpose()?;
        loop {
            let attempt = self.db.begin_api_dispatch_attempt(reservation_id, now)?;
            let transport_response = match transport.send(&prepared) {
                Ok(response) => response,
                Err(_) => {
                    self.db.complete_api_dispatch_attempt_failure(
                        &attempt.id,
                        "uncertain",
                        "transport",
                        false,
                        None,
                        None,
                        now,
                    )?;
                    bail!(
                        "api.transport_uncertain: transport failed after dispatch; automatic replay is denied"
                    );
                }
            };
            let request_id_hash = transport_response.request_id_sha256();
            if !transport_response.is_success() {
                let classification =
                    transport_response.failure_classification(&active_reservation.provider);
                let can_retry = classification.retryable
                    && attempt.attempt_number < active_reservation.reserved_requests;
                self.db.complete_api_dispatch_attempt_failure(
                    &attempt.id,
                    if can_retry {
                        "retryable_failure"
                    } else {
                        "terminal_failure"
                    },
                    api_failure_kind_key(classification.kind),
                    can_retry,
                    Some(transport_response.status_code()),
                    Some(&request_id_hash),
                    now,
                )?;
                if can_retry {
                    continue;
                }
                if classification.retryable {
                    bail!("api.retry_exhausted: all reserved request attempts failed");
                }
                bail!(
                    "api.provider_failure: kind={} retryable=false",
                    api_failure_kind_key(classification.kind)
                );
            }
            let response = match parse_api_transport_response(
                &active_reservation.provider,
                &transport_response,
            ) {
                Ok(response) => response,
                Err(_) => {
                    self.db.complete_api_dispatch_attempt_failure(
                        &attempt.id,
                        "uncertain",
                        "response_invalid",
                        false,
                        Some(transport_response.status_code()),
                        Some(&request_id_hash),
                        now,
                    )?;
                    bail!(
                        "api.response_uncertain: successful transport response was not authoritative; automatic replay is denied"
                    );
                }
            };
            if response.provider != active_reservation.provider
                || response.model != active_reservation.model
            {
                self.db.complete_api_dispatch_attempt_failure(
                    &attempt.id,
                    "uncertain",
                    "response_identity_mismatch",
                    false,
                    Some(transport_response.status_code()),
                    Some(&request_id_hash),
                    now,
                )?;
                bail!("api.response_identity_mismatch: provider response differs from reservation");
            }
            let (cost_micros, pricing_evidence_id) = match pricing_evidence.as_ref() {
                Some(price) => {
                    let cost = crate::api_pricing::calculate_api_cost_micros(
                        price,
                        response.input_tokens,
                        response.cached_input_tokens,
                        response.cache_creation_input_tokens,
                        response.output_tokens,
                    )?;
                    (cost, Some(price.id.clone()))
                }
                None => (0, None),
            };
            let spend = self.db.settle_api_dispatch_attempt(
                &attempt.id,
                transport_response.status_code(),
                &ApiSettlement {
                    reservation_id: active_reservation.id.clone(),
                    provider_request_id_hash: request_id_hash,
                    input_tokens: response.input_tokens,
                    cached_input_tokens: response.cached_input_tokens,
                    cache_creation_input_tokens: response.cache_creation_input_tokens,
                    output_tokens: response.output_tokens,
                    cost_micros,
                    currency: budget.currency.clone(),
                    pricing_evidence_id,
                    source: "provider_reported".into(),
                    observed_at: now,
                },
            )?;
            let mut attempts = self
                .db
                .list_api_dispatch_attempts(None)?
                .into_iter()
                .filter(|entry| entry.reservation_id == reservation_id)
                .collect::<Vec<_>>();
            attempts.sort_by_key(|entry| entry.attempt_number);
            return Ok(ApiExecutionResult {
                response,
                spend,
                attempts,
            });
        }
    }

    pub fn release_api_reservation(
        &mut self,
        reservation_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<ApiBudgetReservation> {
        self.db.release_api_reservation(reservation_id, reason, now)
    }

    pub fn settle_api_reservation(&mut self, settlement: &ApiSettlement) -> Result<ApiSpend> {
        self.db.settle_api_reservation(settlement)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_quota_usage_sample(
        &mut self,
        evidence_id: &str,
        adapter: &str,
        provider: &str,
        account: &str,
        surface: &str,
        estimated_seconds: u64,
        consumed_percent: f64,
        source: &str,
        confidence: &str,
        observed_at: DateTime<Utc>,
    ) -> Result<QuotaUsageSample> {
        self.db.record_quota_usage_sample(
            evidence_id,
            adapter,
            provider,
            account,
            surface,
            estimated_seconds,
            consumed_percent,
            source,
            confidence,
            observed_at,
        )
    }

    pub fn quota_usage_samples(&self, limit: usize) -> Result<Vec<QuotaUsageSample>> {
        self.db.list_quota_usage_samples(limit)
    }

    pub fn usage_forecast(
        &self,
        adapter: &str,
        provider: &str,
        account: &str,
        estimated_seconds: u64,
        uncertainty_percent: u8,
    ) -> Result<UsageForecast> {
        if adapter.trim().is_empty() || provider.trim().is_empty() || account.trim().is_empty() {
            bail!("forecast adapter, provider, and account are required");
        }
        if estimated_seconds == 0 {
            bail!("forecast estimated seconds must be greater than zero");
        }
        const LOOKBACK_LIMIT: usize = 50;
        const MINIMUM_SAMPLES: usize = 5;
        const PERCENTILE: usize = 90;
        let mut predictions = self.db.historical_usage_predictions(
            adapter,
            provider,
            account,
            estimated_seconds,
            LOOKBACK_LIMIT,
        )?;
        predictions.retain(|value| value.is_finite() && *value > 0.0);
        predictions.sort_by(f64::total_cmp);
        let sample_count = predictions.len();
        let (forecast_percent, source, percentile) = if sample_count >= MINIMUM_SAMPLES {
            let rank = (PERCENTILE * sample_count).div_ceil(100);
            let observed = predictions[rank.saturating_sub(1)];
            let adjusted = observed * (1.0 + f64::from(uncertainty_percent) / 100.0);
            (adjusted.clamp(1.0, 100.0), "historical_p90", Some(90))
        } else {
            (
                fallback_forecast_percent(estimated_seconds, uncertainty_percent),
                "conservative_fallback",
                None,
            )
        };
        Ok(UsageForecast {
            adapter: adapter.into(),
            provider: provider.into(),
            account: account.into(),
            estimated_seconds,
            uncertainty_percent,
            forecast_percent,
            source: source.into(),
            sample_count,
            percentile,
            lookback_limit: LOOKBACK_LIMIT,
        })
    }

    fn usage_forecast_for_task(
        &self,
        task: &Task,
        adapter: &str,
        provider: &str,
        account: &str,
    ) -> Result<UsageForecast> {
        self.usage_forecast(
            adapter,
            provider,
            account,
            task.estimated_seconds,
            task.uncertainty_percent,
        )
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

    pub fn run_records(&self, task_id: &str) -> Result<Vec<RunRecord>> {
        self.db.run_records_for_task(task_id)
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
        let adapter = self.db.run_adapter(run_id)?;
        let forecast = self
            .usage_forecast_for_task(&task, &adapter, provider, account)?
            .forecast_percent;
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
        let api_route = adapter == "api";
        let api_provider_allowed = !api_route || matches!(provider, "openai" | "anthropic");
        let api_policy_allowed = !api_route || self.policy.api_allowed(provider);
        let api_capacity = if api_route && api_provider_allowed {
            Some(
                self.db
                    .api_route_capacity(&project.id, provider, account, now)?,
            )
        } else {
            None
        };
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
        let quota: Vec<_> = if api_route {
            vec![]
        } else {
            self.db
                .list_quota()?
                .into_iter()
                .filter(|surface| surface.provider == provider && surface.account == account)
                .collect()
        };
        let usage_forecast = if api_route {
            UsageForecast {
                adapter: adapter.into(),
                provider: provider.into(),
                account: account.into(),
                estimated_seconds: task.estimated_seconds,
                uncertainty_percent: task.uncertainty_percent,
                forecast_percent: 0.0,
                source: "api_budget_capacity".into(),
                sample_count: 0,
                percentile: None,
                lookback_limit: 0,
            }
        } else {
            self.usage_forecast_for_task(&task, adapter, provider, account)?
        };
        let forecast = usage_forecast.forecast_percent;
        let required_headroom = if api_route {
            0.0
        } else {
            quota
                .iter()
                .map(|surface| surface.reserve_percent + forecast)
                .fold(self.policy.reserve_percent + forecast, f64::max)
        };
        let (quota_allowed, quota_reason_code, quota_reason) = if api_route {
            let capacity = api_capacity.as_ref();
            (
                api_provider_allowed
                    && api_policy_allowed
                    && capacity.is_some_and(|capacity| capacity.allowed),
                capacity.map_or("api.provider_denied", |capacity| {
                    capacity.reason_code.as_str()
                }),
                if !api_provider_allowed {
                    "api.provider_denied: api adapter supports only openai or anthropic".into()
                } else if !api_policy_allowed {
                    "api.policy_disabled: effective policy disables this API provider".into()
                } else {
                    capacity.map_or_else(
                        || "api.disabled: project API budget is unavailable".into(),
                        |capacity| capacity.reason.clone(),
                    )
                },
            )
        } else if quota.is_empty() {
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
        } else if api_route && !api_provider_allowed {
            (
                "api.provider_denied".to_owned(),
                "api.provider_denied: api adapter supports only openai or anthropic".into(),
            )
        } else if api_route && !api_policy_allowed {
            (
                "api.policy_disabled".to_owned(),
                "api.policy_disabled: effective policy disables this API provider".into(),
            )
        } else if !quota_allowed {
            (quota_reason_code.to_owned(), quota_reason)
        } else {
            ("route.allowed".to_owned(), quota_reason)
        };
        let selected_adapter = allowed.then(|| adapter.to_owned());
        let selected_provider = allowed.then(|| provider.to_owned());
        let selected_account = allowed.then(|| account.to_owned());
        let quota_wake = if api_route {
            api_capacity
                .as_ref()
                .and_then(|capacity| capacity.next_wake_at)
        } else {
            (!quota_allowed)
                .then(|| quota.iter().filter_map(|surface| surface.reset_at).max())
                .flatten()
        };
        let schedule_wake = (!schedule.eligible)
            .then_some(schedule.next_eligible_at)
            .flatten();
        let retry_wake = (!retry_allowed).then_some(retry.retry_not_before).flatten();
        let next_wake_at = [schedule_wake, quota_wake, retry_wake, circuit_wake]
            .into_iter()
            .flatten()
            .max();
        let minimum_effective_remaining_percent = if api_route {
            api_capacity
                .as_ref()
                .and_then(|capacity| capacity.remaining_percent)
        } else {
            quota
                .iter()
                .filter_map(|surface| surface.effective_remaining_percent)
                .min_by(f64::total_cmp)
        };
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
                forecast_source: usage_forecast.source,
                forecast_sample_count: usage_forecast.sample_count,
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
        validate_api_runtime_config(config)?;
        if config.execute_api_claims {
            let mut transport = LiveApiTransport::new(LiveApiTransportConfig {
                network_enabled: true,
                ..LiveApiTransportConfig::default()
            })?;
            return self.run_scheduler_daemon_with(
                config,
                shutdown,
                Some(&mut transport),
                Utc::now,
                std::thread::sleep,
            );
        }
        self.run_scheduler_daemon_with(config, shutdown, None, Utc::now, std::thread::sleep)
    }

    fn run_scheduler_daemon_with<N, S>(
        &mut self,
        config: &SchedulerDaemonConfig,
        shutdown: &AtomicBool,
        mut api_transport: Option<&mut dyn ApiTransport>,
        mut now: N,
        mut sleep: S,
    ) -> Result<SchedulerDaemonSummary>
    where
        N: FnMut() -> DateTime<Utc>,
        S: FnMut(std::time::Duration),
    {
        self.api_patch_execution_enabled = false;
        validate_api_runtime_config(config)?;
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
        if config.execute_api_claims {
            if api_transport.is_none() {
                bail!(
                    "api.transport_unavailable: paid API execution requires an explicit transport"
                );
            }
            let mut found_api_target = false;
            for target in &route_targets {
                if target.adapter != "api" {
                    continue;
                }
                found_api_target = true;
                match target.provider.as_str() {
                    "openai" => self.policy.openai_api_enabled = true,
                    "anthropic" => self.policy.anthropic_api_enabled = true,
                    _ => bail!("api.provider_denied: unsupported API provider"),
                }
            }
            if !found_api_target {
                bail!("api.route_missing: --execute-api requires an api route candidate");
            }
        } else if config.paid_api_acknowledgement.is_some() {
            bail!(
                "api.execution_disabled: paid API acknowledgement is invalid without --execute-api"
            );
        }

        self.api_patch_execution_enabled = config.execute_api_patches;
        let daemon_result = (|| -> Result<SchedulerDaemonSummary> {
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
                    if config.execute_fake_claims || config.execute_api_claims {
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
                            let selected_adapter =
                                route.selected_adapter.clone().ok_or_else(|| {
                                    anyhow::anyhow!("allowed route has no selected adapter")
                                })?;
                            if selected_adapter.starts_with("fake") && config.execute_fake_claims {
                                self.run_scheduler_claim(
                                    claim,
                                    &config.instance_id,
                                    leader.generation,
                                    &selected_adapter,
                                    route,
                                    tick_at,
                                )?;
                                runs_completed += 1;
                            } else if selected_adapter == "api" && config.execute_api_claims {
                                let transport = api_transport.as_deref_mut().ok_or_else(|| {
                                anyhow::anyhow!(
                                    "api.transport_unavailable: paid API execution requires an explicit transport"
                                )
                            })?;
                                self.run_scheduler_api_claim(
                                    claim,
                                    &config.instance_id,
                                    leader.generation,
                                    route,
                                    transport,
                                    tick_at,
                                )?;
                                runs_completed += 1;
                            }
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
        })();
        self.api_patch_execution_enabled = false;
        daemon_result
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

        let pin = task_route_pin(&task)?;
        for (target, decision) in &mut preliminary {
            let exact_api_pin = pin.as_ref().is_some_and(|pin| {
                pin.adapter == "api"
                    && pin.provider == target.provider
                    && pin.account == target.account
            });
            if target.adapter == "api" && !exact_api_pin {
                decision.allowed = false;
                decision.selected_adapter = None;
                decision.selected_provider = None;
                decision.selected_account = None;
                decision.reason_code = "api.explicit_selection_required".into();
                decision.reason = "api.explicit_selection_required: every paid API scheduler candidate requires an exact task pin and cannot be selected as fallback".into();
                decision.candidates[0].allowed = false;
                decision.candidates[0].reason_code = decision.reason_code.clone();
                decision.candidates[0].filter_reason = decision.reason.clone();
            }
        }

        for (target, decision) in &mut preliminary {
            if target.adapter == "api"
                && decision.allowed
                && let Err(error) =
                    self.planned_api_request_at(&task, &target.provider, &target.account, now)
            {
                let reason = error.to_string();
                let reason_code = api_error_reason_code(&reason);
                decision.allowed = false;
                decision.selected_adapter = None;
                decision.selected_provider = None;
                decision.selected_account = None;
                decision.reason_code = reason_code.into();
                decision.reason = reason.clone();
                decision.candidates[0].allowed = false;
                decision.candidates[0].reason_code = reason_code.into();
                decision.candidates[0].filter_reason = reason;
            }
            if target.adapter == "api"
                && decision.allowed
                && let Err(error) = api_patch_mode(&task, self.api_patch_execution_enabled)
            {
                let reason = error.to_string();
                let reason_code = api_error_reason_code(&reason);
                decision.allowed = false;
                decision.selected_adapter = None;
                decision.selected_provider = None;
                decision.selected_account = None;
                decision.reason_code = reason_code.into();
                decision.reason = reason;
                decision.candidates[0].allowed = false;
                decision.candidates[0].reason_code = decision.reason_code.clone();
                decision.candidates[0].filter_reason = decision.reason.clone();
            }
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
            let (freshness, health, capabilities) = if target.adapter == "api" {
                (
                    ProbeFreshness::Fresh,
                    AdapterHealth::Healthy,
                    api_adapter_capabilities(),
                )
            } else if target.adapter.starts_with("fake") {
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
            let remaining_percent = if target.adapter == "api" {
                preliminary
                    .iter()
                    .find(|(candidate, _)| candidate == target)
                    .and_then(|(_, decision)| {
                        decision.candidates[0].minimum_effective_remaining_percent
                    })
            } else {
                surfaces
                    .iter()
                    .filter_map(|surface| surface.effective_remaining_percent)
                    .min_by(|left, right| left.total_cmp(right))
            };
            let reserve_percent = if target.adapter == "api" {
                0.0
            } else {
                surfaces
                    .iter()
                    .map(|surface| surface.reserve_percent)
                    .fold(self.policy.reserve_percent, f64::max)
            };
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
                forecast_percent: preliminary
                    .iter()
                    .find(|(candidate, _)| candidate == target)
                    .map(|(_, decision)| decision.candidates[0].forecast_percent)
                    .expect("allowed target has a preliminary decision"),
                historical_success_percent: None,
                continuity: false,
                preference: 0.0,
            });
        }
        let selection = select_candidate(
            &RoutingRequest {
                required_capabilities: task.required_capabilities.clone(),
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
                        forecast_percent: preliminary_decision.candidates[0].forecast_percent,
                        forecast_source: preliminary_decision.candidates[0].forecast_source.clone(),
                        forecast_sample_count: preliminary_decision.candidates[0]
                            .forecast_sample_count,
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
            let claim_result = if selected_target.adapter == "api" {
                (|| -> Result<SchedulerClaim> {
                    if task.pinned_adapter.as_deref() != Some("api")
                        || task.pinned_provider.as_deref()
                            != Some(selected_target.provider.as_str())
                        || task.pinned_account.as_deref() != Some(selected_target.account.as_str())
                    {
                        bail!(
                            "api.explicit_selection_required: task must be pinned to this paid API identity"
                        );
                    }
                    let (plan, spec, budget) = self.planned_api_request_at(
                        &task,
                        &selected_target.provider,
                        &selected_target.account,
                        now,
                    )?;
                    if plan.role != "implementer" {
                        bail!(
                            "api.role_denied: scheduler task execution requires the implementer role"
                        );
                    }
                    let reserved_currency_micros = match budget.currency.as_deref() {
                        Some(currency) => {
                            let price = self.db.effective_api_model_price(
                                &selected_target.provider,
                                &selected_target.account,
                                &spec.model,
                                currency,
                                now,
                            )?;
                            worst_case_api_cost_micros(
                                &price,
                                plan.max_input_tokens,
                                plan.max_output_tokens,
                            )?
                        }
                        None => 0,
                    };
                    let (claim, _) = self.db.claim_task_for_scheduler_with_api_reservation(
                        instance_id,
                        leader_generation,
                        &task.id,
                        task.version,
                        now,
                        claim_ttl,
                        max_active_claims,
                        &decision.id,
                        &[],
                        &selected_target.provider,
                        &selected_target.account,
                        max_active_per_adapter,
                        max_active_per_account,
                        &ApiClaimReservationRequest {
                            model: plan.model,
                            role: plan.role,
                            request_digest: plan.request_digest,
                            reserved_currency_micros,
                            reserved_input_tokens: plan.max_input_tokens,
                            reserved_output_tokens: plan.max_output_tokens,
                            reserved_attempts: plan.max_retries + 1,
                        },
                    )?;
                    Ok(claim)
                })()
            } else {
                self.db.claim_task_for_scheduler_with_route_limits(
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
                    decision
                        .candidates
                        .iter()
                        .find(|candidate| {
                            candidate.adapter == selected_target.adapter
                                && candidate.provider == selected_target.provider
                                && candidate.account == selected_target.account
                        })
                        .map(|candidate| candidate.forecast_percent)
                        .ok_or_else(|| {
                            anyhow::anyhow!("selected route has no forecast evidence")
                        })?,
                )
            };
            match claim_result {
                Ok(claim) => claims.push(claim),
                Err(error) => {
                    let message = error.to_string();
                    let reason_code = error.downcast_ref::<SchedulerClaimRejection>().map_or_else(
                        || {
                            if message.starts_with("api.") {
                                api_error_reason_code(&message)
                            } else {
                                "scheduler.claim_conflict"
                            }
                        },
                        |rejection| rejection.reason_code(),
                    );
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
        self.finish_agent_run(
            task,
            project,
            adapter,
            route,
            worktree,
            run_id,
            sandbox,
            true,
            vec!["fake backend attestation is deterministic test evidence".into()],
        )
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
        self.finish_agent_run(
            task,
            project,
            adapter,
            route,
            worktree,
            run_id,
            sandbox,
            true,
            vec!["fake backend attestation is deterministic test evidence".into()],
        )
    }

    fn run_scheduler_api_claim<T: ApiTransport + ?Sized>(
        &mut self,
        claim: &SchedulerClaim,
        instance_id: &str,
        leader_generation: i64,
        route: RouteDecision,
        transport: &mut T,
        now: DateTime<Utc>,
    ) -> Result<RunSummary> {
        if claim.route_decision_id.as_deref() != Some(route.id.as_str()) {
            bail!("scheduler claim and route decision do not match");
        }
        if route.selected_adapter.as_deref() != Some("api") {
            bail!("api.route_mismatch: scheduler route does not select the API adapter");
        }
        let provider = route
            .selected_provider
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("api.route_mismatch: API route has no provider"))?;
        let account = route
            .selected_account
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("api.route_mismatch: API route has no account"))?;
        let task = self.db.task(&claim.task_id)?;
        if task.status != TaskStatus::Leased {
            bail!(
                "scheduler claim task must be leased; current status is {}",
                task.status
            );
        }
        let patch_mode = api_patch_mode(&task, self.api_patch_execution_enabled)?;
        let allowed_patch_paths = if patch_mode {
            Some(validated_patch_scope(&task)?)
        } else {
            None
        };
        let (plan, spec, _) = self.planned_api_request_for_version_at(
            &task,
            claim.task_version,
            provider,
            account,
            now,
        )?;
        if plan.role != "implementer" {
            bail!("api.role_denied: scheduler task execution requires the implementer role");
        }
        let reservation = self.db.api_reservation_for_claim(&claim.id)?;
        if reservation.task_id != task.id
            || reservation.provider != provider
            || reservation.account != account
            || reservation.model != plan.model
            || reservation.role != plan.role
            || reservation.request_digest != plan.request_digest
        {
            bail!("api.claim_reservation_mismatch: scheduler claim and request plan differ");
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
        let sandbox = direct_api_attestation(provider, Path::new(&worktree.path), patch_mode);
        let policy_decision = if patch_mode {
            self.policy.authorize_isolated_patch(task.risk_class, true)
        } else {
            self.policy
                .authorize(task.risk_class, sandbox.secure_container)
        };
        match policy_decision {
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
            "api",
            &worktree.path,
            &worktree.branch,
            &worktree.base_commit,
            now,
            std::time::Duration::from_secs(task.checkpoint_seconds),
        )?;
        self.db.append_run_event(
            &task.id,
            &run_id,
            "api.execution_started",
            "control_plane",
            &serde_json::json!({
                "reservation_id": reservation.id,
                "provider": provider,
                "account": account,
                "model": plan.model,
                "plan_id": plan.id,
                "reserved_attempts": reservation.reserved_requests,
            }),
        )?;

        let execution =
            match self.execute_reserved_api_request(&reservation.id, &spec, transport, now) {
                Ok(execution) => execution,
                Err(error) => {
                    let error_message = error.to_string();
                    let reason_code = api_error_reason_code(&error_message);
                    if let Err(finish_error) =
                        self.fail_scheduler_api_run(&task, &run_id, reason_code, now)
                    {
                        return Err(error.context(format!(
                            "also failed to terminalise API run: {finish_error:#}"
                        )));
                    }
                    return Err(error);
                }
            };
        self.db.append_run_event(
            &task.id,
            &run_id,
            "api.execution_completed",
            "control_plane",
            &serde_json::json!({
                "provider": execution.response.provider,
                "model": execution.response.model,
                "terminal_status": execution.response.terminal_status,
                "output_item_count": execution.response.output.len(),
                "attempt_count": execution.attempts.len(),
                "spend_id": execution.spend.id,
                "input_tokens": execution.spend.input_tokens,
                "cached_input_tokens": execution.spend.cached_input_tokens,
                "cache_creation_input_tokens": execution.spend.cache_creation_input_tokens,
                "output_tokens": execution.spend.output_tokens,
                "cost_micros": execution.spend.cost_micros,
                "currency": execution.spend.currency,
            }),
        )?;
        let expected_status = if patch_mode {
            ApiTerminalStatus::ToolUse
        } else {
            ApiTerminalStatus::Completed
        };
        if execution.response.terminal_status != expected_status {
            self.fail_scheduler_api_run(&task, &run_id, "api.non_completed_response", now)?;
            bail!(
                "api.non_completed_response: provider returned {:?}; expected {:?}",
                execution.response.terminal_status,
                expected_status
            );
        }
        if patch_mode {
            let apply_result = (|| {
                let patch = extract_api_patch(&execution.response)?;
                git::apply_untrusted_patch(Path::new(&worktree.path), patch.as_bytes())?;
                let changed_files = git::changed_files(Path::new(&worktree.path))?;
                validate_applied_patch_paths(
                    Path::new(&worktree.path),
                    &changed_files,
                    allowed_patch_paths
                        .as_ref()
                        .expect("patch scope was validated before execution"),
                )?;
                self.db.append_run_event(
                    &task.id,
                    &run_id,
                    "api.patch_applied",
                    "control_plane",
                    &serde_json::json!({
                        "tool": API_PATCH_TOOL,
                        "patch_bytes": patch.len(),
                        "patch_sha256": hex::encode(Sha256::digest(patch.as_bytes())),
                        "changed_files": changed_files,
                    }),
                )?;
                Ok::<(), anyhow::Error>(())
            })();
            if let Err(error) = apply_result {
                let reason_code = api_error_reason_code(&error.to_string()).to_owned();
                if let Err(finish_error) =
                    self.fail_scheduler_api_run(&task, &run_id, &reason_code, now)
                {
                    return Err(error.context(format!(
                        "also failed to terminalise rejected API patch run: {finish_error:#}"
                    )));
                }
                return Err(error);
            }
        }
        self.finish_agent_run(
            task,
            project,
            "api",
            route,
            worktree,
            run_id,
            sandbox,
            false,
            if patch_mode {
                vec![
                    "the control plane applied one validated submit_patch result to the isolated task worktree".into(),
                    "independent verification evaluates the resulting patch in a separate detached worktree".into(),
                ]
            } else {
                vec![
                    "direct API response content is not persisted or applied to the worktree".into(),
                    "independent verification evaluates the unchanged isolated worktree".into(),
                ]
            },
        )
    }

    fn fail_scheduler_api_run(
        &mut self,
        task: &Task,
        run_id: &str,
        reason_code: &str,
        _now: DateTime<Utc>,
    ) -> Result<()> {
        self.db.append_run_event(
            &task.id,
            run_id,
            "api.execution_failed",
            "control_plane",
            &serde_json::json!({"reason_code": reason_code}),
        )?;
        self.db.transition_task(
            &task.id,
            TaskStatus::Running,
            TaskStatus::Failed,
            reason_code,
        )?;
        self.db.finish_run(run_id, "failed", None, 1)?;
        self.db.enqueue_notification(
            "failure",
            "error",
            Some(&task.id),
            Some(run_id),
            "Paid API task failed",
            &format!("{} failed with {}.", task.title, reason_code),
            Utc::now(),
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_agent_run(
        &mut self,
        task: Task,
        project: Project,
        adapter: &str,
        route: RouteDecision,
        worktree: crate::git::Worktree,
        run_id: String,
        sandbox: SandboxAttestation,
        apply_fake_effects: bool,
        assumptions: Vec<String>,
    ) -> Result<RunSummary> {
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

        if apply_fake_effects
            && let (Some(relative), Some(content)) =
                (&task.fake_write_path, &task.fake_write_content)
        {
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
        let implementer_target = RouteTarget {
            adapter: adapter.to_owned(),
            provider: route
                .selected_provider
                .clone()
                .ok_or_else(|| anyhow::anyhow!("implementer route has no provider"))?,
            account: route
                .selected_account
                .clone()
                .ok_or_else(|| anyhow::anyhow!("implementer route has no account"))?,
        };
        let verifier_target = RouteTarget {
            adapter: "garnish-command-verifier".into(),
            provider: "local".into(),
            account: "default".into(),
        };
        let verifier_route = build_verifier_route(
            &self.policy,
            &task.id,
            &implementer_target,
            std::slice::from_ref(&verifier_target),
            Utc::now(),
        );
        self.db.record_route(&verifier_route)?;
        if !verifier_route.allowed {
            bail!("no independent verifier passed policy separation");
        }
        let verifier_destination = self.data_dir.join("verifiers").join(&run_id);
        let verifier = git::create_verification_worktree(
            Path::new(&project.root_path),
            &verifier_destination,
            &worktree.base_commit,
            &patch,
        )?;
        let verifier_sandbox = FakeSandbox::attest(Path::new(&verifier.path));
        let verifier_run_id = Ulid::new().to_string();
        let verifier_evidence = RunEvidence::create(&self.data_dir, &verifier_run_id)?;
        let started = Utc::now();
        self.db.create_verifier_run(
            &verifier_run_id,
            &run_id,
            &task.id,
            &verifier_target.adapter,
            &verifier_route.id,
            &verifier.path,
            &worktree.base_commit,
            started,
        )?;
        verifier_evidence.write_manifest(&RunManifest {
            schema_version: 1,
            run_id: verifier_run_id.clone(),
            task_id: task.id.clone(),
            project_id: project.id.clone(),
            adapter: verifier_target.adapter.clone(),
            base_commit: worktree.base_commit.clone(),
            worktree: verifier.path.clone(),
            branch: "(detached verifier)".into(),
            policy_hash: self.policy.hash(),
            route_decision_id: verifier_route.id.clone(),
            created_at: started.to_rfc3339(),
            sandbox: verifier_sandbox.clone(),
        })?;
        verifier_evidence.write_route(&verifier_route)?;
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
        verifier_evidence.write_process_output(&output.stdout, &output.stderr)?;
        verifier_evidence.write_verification(&verification)?;
        self.db.finish_verifier_run(
            &verifier_run_id,
            &run_id,
            verification.passed,
            exit_code,
            verifier_evidence
                .verification_path
                .to_string_lossy()
                .as_ref(),
            ended,
        )?;
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
            decisions: vec![
                format!("implementation route {}: {}", route.id, route.reason),
                format!(
                    "verifier run {} route {} selected {}:{}:{}",
                    verifier_run_id,
                    verifier_route.id,
                    verifier_target.adapter,
                    verifier_target.provider,
                    verifier_target.account
                ),
            ],
            assumptions,
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
            verifier_run_id,
            verifier_adapter: verifier_target.adapter,
            verifier_route_decision_id: verifier_route.id,
        })
    }

    pub fn recover(&mut self) -> Result<Vec<String>> {
        self.recover_at(Utc::now())
    }

    pub fn recover_at(&mut self, now: DateTime<Utc>) -> Result<Vec<String>> {
        self.db.recover_expired_api_reservations(now)?;
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

fn render_task_api_request(
    task: &Task,
    provider: &str,
    model: &str,
    max_output_tokens: u64,
    stream: bool,
) -> Result<ApiRequestSpec> {
    let patch_mode = task_requests_api_patch(task);
    let input = serde_json::to_string(&serde_json::json!({
        "task_id": task.id,
        "task_version": task.version,
        "title": task.title,
        "goal": task.goal,
        "rationale": task.rationale,
        "scope": task.scope,
        "non_scope": task.non_scope,
        "acceptance": task.acceptance,
        "verification_argv": task.verification_argv,
        "risk_class": task.risk_class,
        "required_capabilities": task.required_capabilities,
    }))?;
    let tools = if patch_mode {
        vec![ApiToolDefinition {
            name: API_PATCH_TOOL.into(),
            description: "Submit exactly one UTF-8 git patch for deterministic validation and application to the isolated task worktree. Do not include binary data, links, submodules, renames, or out-of-scope files.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "A complete unified git diff beginning with diff --git"
                    }
                },
                "required": ["patch"],
                "additionalProperties": false
            }),
        }]
    } else {
        vec![]
    };
    Ok(ApiRequestSpec {
        provider: provider.into(),
        model: model.into(),
        instructions: if patch_mode {
            "Execute only the supplied canonical task. Return exactly one submit_patch tool call containing a UTF-8 git diff. Change only exact paths listed in scope; do not call any other tool or return prose.".into()
        } else {
            "Execute only the supplied canonical task. Respect scope and return a concise implementation result with verification evidence.".into()
        },
        input,
        max_output_tokens,
        tools,
        stream,
    })
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

fn build_verifier_route(
    policy: &EffectivePolicy,
    task_id: &str,
    implementer: &RouteTarget,
    candidates: &[RouteTarget],
    now: DateTime<Utc>,
) -> RouteDecision {
    let mut candidates = candidates.to_vec();
    candidates.sort_by(|left, right| {
        (&left.adapter, &left.provider, &left.account).cmp(&(
            &right.adapter,
            &right.provider,
            &right.account,
        ))
    });
    candidates.dedup();
    let evaluations = candidates
        .into_iter()
        .map(|candidate| {
            let (allowed, reason_code, filter_reason) = if &candidate == implementer {
                (
                    false,
                    "verifier.same_identity",
                    "verifier identity must differ from the implementer".to_owned(),
                )
            } else if policy.verifier_must_differ_adapter
                && candidate.adapter == implementer.adapter
            {
                (
                    false,
                    "verifier.same_adapter",
                    "policy requires a verifier using a different adapter".to_owned(),
                )
            } else if policy.verifier_must_differ_provider
                && candidate.provider == implementer.provider
            {
                (
                    false,
                    "verifier.same_provider",
                    "policy requires a verifier using a different provider".to_owned(),
                )
            } else {
                (
                    true,
                    "verifier.allowed",
                    "candidate satisfies independent-verifier separation policy".to_owned(),
                )
            };
            RouteCandidate {
                adapter: candidate.adapter,
                provider: candidate.provider,
                account: candidate.account,
                allowed,
                reason_code: reason_code.into(),
                filter_reason,
                forecast_percent: 0.0,
                forecast_source: "quota_free_local_verification".into(),
                forecast_sample_count: 0,
                minimum_effective_remaining_percent: None,
                score: allowed.then_some(0.0),
                score_components: allowed.then(|| serde_json::json!({"separation": true})),
            }
        })
        .collect::<Vec<_>>();
    let selected = evaluations.iter().find(|candidate| candidate.allowed);
    let allowed = selected.is_some();
    RouteDecision {
        id: Ulid::new().to_string(),
        task_id: task_id.into(),
        selected_adapter: selected.map(|candidate| candidate.adapter.clone()),
        selected_provider: selected.map(|candidate| candidate.provider.clone()),
        selected_account: selected.map(|candidate| candidate.account.clone()),
        allowed,
        reason_code: if allowed {
            "verifier.allowed".into()
        } else {
            "verifier.unavailable".into()
        },
        reason: if allowed {
            "an independent verifier passed separation policy".into()
        } else {
            "no verifier candidate passed separation policy".into()
        },
        required_headroom_percent: 0.0,
        quota: vec![],
        candidates: evaluations,
        next_wake_at: None,
        schedule: None,
        policy_hash: policy.hash(),
        created_at: now,
    }
}

fn fallback_forecast_percent(estimated_seconds: u64, uncertainty_percent: u8) -> f64 {
    let baseline = (estimated_seconds as f64 / 2700.0) * 20.0;
    (baseline * (1.0 + f64::from(uncertainty_percent) / 100.0)).clamp(1.0, 50.0)
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
    if adapter == "api" {
        let capabilities = api_adapter_capabilities();
        let missing = required
            .iter()
            .filter(|required| !capabilities.contains(required))
            .cloned()
            .collect::<Vec<_>>();
        return if missing.is_empty() {
            (
                true,
                "API adapter satisfies every required capability".into(),
            )
        } else {
            (
                false,
                format!(
                    "capability.missing: API adapter lacks {}",
                    missing.join(",")
                ),
            )
        };
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

fn api_adapter_capabilities() -> Vec<String> {
    vec![
        "agent.headless".into(),
        "agent.tool_policy".into(),
        API_PATCH_CAPABILITY.into(),
    ]
}

fn direct_api_attestation(provider: &str, worktree: &Path, patch_mode: bool) -> SandboxAttestation {
    SandboxAttestation {
        backend: "host-direct-api".into(),
        secure_container: false,
        image: "not-applicable".into(),
        writable_mounts: if patch_mode {
            vec![worktree.to_string_lossy().into_owned()]
        } else {
            vec![]
        },
        network: format!("fixed-https:{provider}"),
        user: "current-host-user".into(),
        container_socket_mounted: false,
        host_home_mounted: true,
        cpu_limit: "host-process".into(),
        memory_limit: "bounded-request-and-response".into(),
        pids_limit: 1,
        rootless: None,
        user_namespace: None,
        effective_capabilities: None,
        capability_evidence_source: Some("compiled-direct-transport".into()),
        inherited_proxy_environment: vec![],
        reasons: if patch_mode {
            vec![
                "provider receives only the bounded request and exposes no shell, filesystem, or network tool".into(),
                "the deterministic control plane may apply one validated patch to the named isolated worktree".into(),
            ]
        } else {
            vec![
                "provider receives only the bounded request; no worktree or repository tool is exposed".into(),
                "response content is not persisted or applied to the isolated worktree".into(),
            ]
        },
    }
}

fn validate_api_runtime_config(config: &SchedulerDaemonConfig) -> Result<()> {
    if config.execute_api_claims {
        if config.paid_api_acknowledgement.as_deref() != Some(PAID_API_DAEMON_ACKNOWLEDGEMENT) {
            bail!(
                "api.execution_acknowledgement_required: --execute-api requires the exact paid API acknowledgement"
            );
        }
    } else if config.paid_api_acknowledgement.is_some() {
        bail!("api.execution_disabled: paid API acknowledgement is invalid without --execute-api");
    }
    if config.execute_api_patches {
        if !config.execute_api_claims {
            bail!("api.patch_execution_disabled: --execute-api-patches requires --execute-api");
        }
        if config.api_patch_acknowledgement.as_deref() != Some(API_PATCH_DAEMON_ACKNOWLEDGEMENT) {
            bail!(
                "api.patch_acknowledgement_required: --execute-api-patches requires the exact isolated patch acknowledgement"
            );
        }
    } else if config.api_patch_acknowledgement.is_some() {
        bail!(
            "api.patch_execution_disabled: patch acknowledgement is invalid without --execute-api-patches"
        );
    }
    Ok(())
}

fn task_requests_api_patch(task: &Task) -> bool {
    task.required_capabilities
        .iter()
        .any(|capability| capability == API_PATCH_CAPABILITY)
}

fn api_patch_mode(task: &Task, enabled: bool) -> Result<bool> {
    match (task.risk_class, task_requests_api_patch(task), enabled) {
        (0, false, _) => Ok(false),
        (1, true, true) => Ok(true),
        (1, true, false) => bail!(
            "api.patch_execution_disabled: risk-class 1 API patch tasks require the separately acknowledged patch runtime"
        ),
        (0, true, _) => {
            bail!("api.patch_risk_mismatch: agent.patch_submission requires risk class 1")
        }
        (1, false, _) => bail!(
            "api.execution_tools_unavailable: risk-class 1 API tasks must explicitly require agent.patch_submission"
        ),
        (2 | 3, _, _) => {
            bail!("api.approval_required: API patch execution is limited to risk class 1")
        }
        _ => bail!("api.risk_denied: unsupported task risk class"),
    }
}

fn validated_patch_scope(task: &Task) -> Result<BTreeSet<String>> {
    if task.scope.is_empty() {
        bail!("api.patch_scope_invalid: patch tasks require at least one exact path");
    }
    let mut allowed = BTreeSet::new();
    for relative in &task.scope {
        let path = Path::new(relative);
        let valid = !relative.is_empty()
            && !relative.contains('\\')
            && path
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
            && !matches!(relative.as_str(), ".git" | ".harness-garnish")
            && !relative.starts_with(".git/")
            && !relative.starts_with(".harness-garnish/");
        if !valid {
            bail!(
                "api.patch_scope_invalid: scope entries must be exact safe repository-relative paths"
            );
        }
        allowed.insert(relative.clone());
    }
    Ok(allowed)
}

fn extract_api_patch(response: &ApiProviderResponse) -> Result<String> {
    let [
        ApiOutputItem::ToolCall {
            name, arguments, ..
        },
    ] = response.output.as_slice()
    else {
        bail!("api.patch_tool_result_invalid: expected exactly one tool call");
    };
    if name != API_PATCH_TOOL {
        bail!("api.patch_tool_denied: provider called an unexpected tool");
    }
    let object = arguments.as_object().ok_or_else(|| {
        anyhow::anyhow!("api.patch_arguments_invalid: submit_patch arguments must be an object")
    })?;
    if object.len() != 1 || !object.contains_key("patch") {
        bail!("api.patch_arguments_invalid: submit_patch requires only the patch field");
    }
    let patch = object["patch"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("api.patch_arguments_invalid: patch must be a string"))?;
    if patch.is_empty() {
        bail!("api.patch_empty: submit_patch returned an empty patch");
    }
    if patch.len() > MAX_API_PATCH_BYTES {
        bail!("api.patch_too_large: patch exceeds the 1 MiB limit");
    }
    Ok(patch.to_owned())
}

fn validate_applied_patch_paths(
    worktree: &Path,
    changed_files: &[String],
    allowed: &BTreeSet<String>,
) -> Result<()> {
    if changed_files.is_empty() {
        bail!("api.patch_empty: applied patch changed no files");
    }
    for relative in changed_files {
        if !allowed.contains(relative) {
            bail!("api.patch_scope_denied: patch changed out-of-scope path {relative}");
        }
        let target = worktree.join(relative);
        if target
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            bail!("api.patch_type_denied: patch created or changed a symbolic link");
        }
    }
    Ok(())
}

fn api_error_reason_code(message: &str) -> &str {
    let candidate = message.split_once(':').map_or(message, |(code, _)| code);
    if candidate.starts_with("api.") {
        candidate
    } else {
        "api.request_plan_invalid"
    }
}

fn api_failure_kind_key(kind: ApiFailureKind) -> &'static str {
    match kind {
        ApiFailureKind::Authentication => "authentication",
        ApiFailureKind::Permission => "permission",
        ApiFailureKind::RateLimited => "rate_limited",
        ApiFailureKind::UsageExhausted => "usage_exhausted",
        ApiFailureKind::InvalidRequest => "invalid_request",
        ApiFailureKind::Transient => "transient",
        ApiFailureKind::Provider => "provider",
        ApiFailureKind::Unknown => "unknown",
    }
}

fn worst_case_api_cost_micros(
    price: &ApiModelPrice,
    input_tokens: u64,
    output_tokens: u64,
) -> Result<u64> {
    let maximum_input_rate = price
        .input_micros_per_million
        .max(price.cached_input_micros_per_million)
        .max(price.cache_creation_input_micros_per_million);
    let (cached_input_tokens, cache_creation_input_tokens) =
        if maximum_input_rate == price.input_micros_per_million {
            (0, 0)
        } else if maximum_input_rate == price.cached_input_micros_per_million {
            (input_tokens, 0)
        } else {
            (0, input_tokens)
        };
    crate::api_pricing::calculate_api_cost_micros(
        price,
        input_tokens,
        cached_input_tokens,
        cache_creation_input_tokens,
        output_tokens,
    )
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
    fn verifier_selection_is_deterministic_and_enforces_policy_separation() {
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let implementer = RouteTarget {
            adapter: "codex".into(),
            provider: "openai".into(),
            account: "primary".into(),
        };
        let candidates = vec![
            implementer.clone(),
            RouteTarget {
                adapter: "codex".into(),
                provider: "local".into(),
                account: "review".into(),
            },
            RouteTarget {
                adapter: "claude".into(),
                provider: "openai".into(),
                account: "review".into(),
            },
            RouteTarget {
                adapter: "antigravity".into(),
                provider: "google".into(),
                account: "review".into(),
            },
        ];
        let default = build_verifier_route(
            &EffectivePolicy::default(),
            "task",
            &implementer,
            &candidates,
            now,
        );
        assert!(default.allowed);
        assert_eq!(default.selected_adapter.as_deref(), Some("antigravity"));
        assert_eq!(default.candidates[0].reason_code, "verifier.allowed");
        assert_eq!(default.candidates[2].reason_code, "verifier.same_adapter");

        let strict = EffectivePolicy {
            verifier_must_differ_provider: true,
            ..EffectivePolicy::default()
        };
        let strict = build_verifier_route(&strict, "task", &implementer, &candidates, now);
        assert_eq!(strict.selected_adapter.as_deref(), Some("antigravity"));
        let same_provider = strict
            .candidates
            .iter()
            .find(|candidate| candidate.adapter == "claude")
            .unwrap();
        assert_eq!(same_provider.reason_code, "verifier.same_provider");
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

    fn enable_fixture_api_route(garnish: &mut Garnish, project_id: &str, now: DateTime<Utc>) {
        garnish.policy.openai_api_enabled = true;
        garnish
            .configure_api_budget(&NewApiBudget {
                project_id: project_id.into(),
                provider: "openai".into(),
                account: "paid".into(),
                enabled: true,
                secret_reference: "env:FIXTURE_API_KEY".into(),
                currency: Some("USD".into()),
                currency_limit_micros: Some(1_000_000),
                token_limit: Some(100_000),
                request_limit: Some(10),
                period_start: now - Duration::minutes(1),
                period_end: now + Duration::days(1),
                allowed_models: vec!["model-fixture".into()],
                allowed_tools: vec![],
                allowed_roles: vec!["implementer".into()],
                max_output_tokens: 1_000,
                max_retries: 0,
                max_concurrent_requests: 1,
                reason: "explicit paid routing fixture".into(),
            })
            .unwrap();
        garnish
            .configure_api_model_price(&NewApiModelPrice {
                provider: "openai".into(),
                account: "paid".into(),
                model: "model-fixture".into(),
                currency: "USD".into(),
                input_micros_per_million: 1_000_000,
                cached_input_micros_per_million: 500_000,
                cache_creation_input_micros_per_million: 1_500_000,
                output_micros_per_million: 2_000_000,
                effective_from: now - Duration::minutes(1),
                effective_to: Some(now + Duration::days(1)),
                source: "fixture price evidence".into(),
                reason: "explicit paid routing fixture".into(),
            })
            .unwrap();
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
    fn historical_usage_forecast_is_durable_deduplicated_and_identity_scoped() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(&data_dir).unwrap();
        for index in 1..=4 {
            garnish
                .record_quota_usage_sample(
                    &format!("run-{index}"),
                    "codex",
                    "codex",
                    "personal",
                    "five_hour",
                    600,
                    f64::from(index),
                    "fixture-adapter",
                    "collector_measured",
                    now + Duration::seconds(index.into()),
                )
                .unwrap();
        }
        let fallback = garnish
            .usage_forecast("codex", "codex", "personal", 600, 20)
            .unwrap();
        assert_eq!(fallback.source, "conservative_fallback");
        assert_eq!(fallback.sample_count, 4);

        garnish
            .record_quota_usage_sample(
                "run-5",
                "codex",
                "codex",
                "personal",
                "five_hour",
                600,
                5.0,
                "fixture-adapter",
                "collector_measured",
                now + Duration::seconds(5),
            )
            .unwrap();
        assert!(
            garnish
                .record_quota_usage_sample(
                    "run-5",
                    "codex",
                    "codex",
                    "personal",
                    "five_hour",
                    600,
                    5.0,
                    "fixture-adapter",
                    "collector_measured",
                    now + Duration::seconds(5),
                )
                .is_err()
        );
        drop(garnish);

        let garnish = Garnish::open(&data_dir).unwrap();
        let historical = garnish
            .usage_forecast("codex", "codex", "personal", 600, 20)
            .unwrap();
        assert_eq!(historical.source, "historical_p90");
        assert_eq!(historical.sample_count, 5);
        assert_eq!(historical.percentile, Some(90));
        assert_eq!(historical.forecast_percent, 6.0);
        let other_account = garnish
            .usage_forecast("codex", "codex", "work", 600, 20)
            .unwrap();
        assert_eq!(other_account.source, "conservative_fallback");
        assert_eq!(other_account.sample_count, 0);
        assert_eq!(garnish.quota_usage_samples(100).unwrap().len(), 5);
    }

    #[test]
    fn api_budget_is_default_deny_and_dispatch_settlement_are_single_use() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = Utc::now();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();
        let budget = garnish
            .configure_api_budget(&NewApiBudget {
                project_id: project.id.clone(),
                provider: "openai".into(),
                account: "default".into(),
                enabled: true,
                secret_reference: "env:OPENAI_API_KEY".into(),
                currency: Some("USD".into()),
                currency_limit_micros: Some(1_000),
                token_limit: Some(100),
                request_limit: Some(1),
                period_start: now - Duration::minutes(1),
                period_end: now + Duration::days(30),
                allowed_models: vec!["gpt-fixture".into()],
                allowed_tools: vec![],
                allowed_roles: vec!["planner".into()],
                max_output_tokens: 50,
                max_retries: 0,
                max_concurrent_requests: 1,
                reason: "explicit fixture budget".into(),
            })
            .unwrap();
        assert!(budget.enabled);
        let request = ApiReservationRequest {
            project_id: project.id,
            task_id: task.id,
            provider: "openai".into(),
            account: "default".into(),
            model: "gpt-fixture".into(),
            role: "planner".into(),
            request_digest: "a".repeat(64),
            reserved_currency_micros: 400,
            reserved_input_tokens: 10,
            reserved_output_tokens: 20,
            reserved_attempts: 1,
            now,
            expires_at: now + Duration::minutes(5),
        };
        let denied = garnish.reserve_api_budget(&request).unwrap_err();
        assert!(denied.to_string().contains("api.policy_disabled"));

        garnish.policy.openai_api_enabled = true;
        let reservation = garnish.reserve_api_budget(&request).unwrap();
        assert_eq!(reservation.status, "active");
        assert!(garnish.reserve_api_budget(&request).is_err());
        let dispatched = garnish
            .claim_api_dispatch(&reservation.id, now + Duration::seconds(1))
            .unwrap();
        assert_eq!(dispatched.status, "dispatched");
        assert!(
            garnish
                .claim_api_dispatch(&reservation.id, now + Duration::seconds(2))
                .is_err()
        );
        assert!(
            garnish
                .release_api_reservation(
                    &reservation.id,
                    "unsafe after dispatch",
                    now + Duration::seconds(2),
                )
                .is_err()
        );
        let price = garnish
            .configure_api_model_price(&NewApiModelPrice {
                provider: "openai".into(),
                account: "default".into(),
                model: "gpt-fixture".into(),
                currency: "USD".into(),
                input_micros_per_million: 1_000_000,
                cached_input_micros_per_million: 500_000,
                cache_creation_input_micros_per_million: 1_500_000,
                output_micros_per_million: 1_000_000,
                effective_from: now,
                effective_to: Some(now + Duration::days(30)),
                source: "fixture price evidence".into(),
                reason: "quota-free accounting fixture".into(),
            })
            .unwrap();
        let replacement_price = garnish
            .configure_api_model_price(&NewApiModelPrice {
                provider: "openai".into(),
                account: "default".into(),
                model: "gpt-fixture".into(),
                currency: "USD".into(),
                input_micros_per_million: 1_000_000,
                cached_input_micros_per_million: 500_000,
                cache_creation_input_micros_per_million: 1_500_000,
                output_micros_per_million: 1_000_000,
                effective_from: now,
                effective_to: Some(now + Duration::days(30)),
                source: "replacement fixture price evidence".into(),
                reason: "verify superseded evidence fails closed".into(),
            })
            .unwrap();
        let settlement = ApiSettlement {
            reservation_id: reservation.id,
            provider_request_id_hash: "b".repeat(64),
            input_tokens: 8,
            cached_input_tokens: 2,
            cache_creation_input_tokens: 1,
            output_tokens: 18,
            cost_micros: 26,
            currency: Some("USD".into()),
            pricing_evidence_id: Some(replacement_price.id),
            source: "provider_reported".into(),
            observed_at: now + Duration::seconds(3),
        };
        let mut superseded = settlement.clone();
        superseded.pricing_evidence_id = Some(price.id);
        assert!(
            garnish
                .settle_api_reservation(&superseded)
                .unwrap_err()
                .to_string()
                .contains("api.pricing_evidence_superseded")
        );
        let mut incorrect = settlement.clone();
        incorrect.cost_micros += 1;
        assert!(
            garnish
                .settle_api_reservation(&incorrect)
                .unwrap_err()
                .to_string()
                .contains("api.cost_mismatch")
        );
        let spend = garnish.settle_api_reservation(&settlement).unwrap();
        assert_eq!(spend.cost_micros, 26);
        assert!(garnish.settle_api_reservation(&settlement).is_err());
        assert_eq!(garnish.api_spend(Some("fixture")).unwrap().len(), 1);

        let mut second = request;
        second.request_digest = "c".repeat(64);
        second.now = now + Duration::seconds(4);
        second.expires_at = now + Duration::minutes(5);
        let denied = garnish.reserve_api_budget(&second).unwrap_err();
        assert!(denied.to_string().contains("api.request_budget_exhausted"));
    }

    #[cfg(unix)]
    #[test]
    fn fake_api_lifecycle_prepares_dispatches_prices_and_settles_once() {
        use crate::api_providers::{ApiTransportResponse, api_request_digest};
        use std::{io::Write, os::unix::fs::OpenOptionsExt};

        struct FakeTransport {
            sends: usize,
            terminal: bool,
            transport_error: bool,
        }
        impl ApiTransport for FakeTransport {
            fn send(&mut self, request: &PreparedApiRequest) -> Result<ApiTransportResponse> {
                self.sends += 1;
                request.with_sensitive_parts(|endpoint, _, _, _, secret, body| {
                    assert_eq!(endpoint, "https://api.openai.com/v1/responses");
                    assert_eq!(secret, b"fixture-secret-never-persist");
                    let body: serde_json::Value = serde_json::from_slice(body).unwrap();
                    assert_eq!(body["model"], "gpt-fixture");
                });
                if self.transport_error {
                    bail!("transport-secret-canary");
                }
                if self.terminal {
                    return ApiTransportResponse::new(
                        401,
                        "provider-request-auth-failure".into(),
                        br#"{"error":{"type":"authentication_error","code":"invalid_api_key"}}"#
                            .to_vec(),
                        false,
                    );
                }
                if self.sends == 1 {
                    return ApiTransportResponse::new(
                        429,
                        "provider-request-retry-1".into(),
                        br#"{"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"}}"#
                            .to_vec(),
                        false,
                    );
                }
                if self.sends == 2 {
                    return ApiTransportResponse::new(
                        503,
                        "provider-request-retry-2".into(),
                        br#"{"error":{"type":"server_error","code":"unavailable"}}"#.to_vec(),
                        false,
                    );
                }
                ApiTransportResponse::new(
                    200,
                    "provider-request-fixture".into(),
                    br#"{
                        "id":"response-fixture","object":"response","status":"completed",
                        "model":"gpt-fixture",
                        "output":[{"type":"message","status":"completed","role":"assistant",
                          "content":[{"type":"output_text","text":"sensitive output","annotations":[]}]}],
                        "usage":{"input_tokens":8,
                          "input_tokens_details":{"cached_tokens":2,"cache_write_tokens":1},
                          "output_tokens":18,"total_tokens":26}
                    }"#
                    .to_vec(),
                    false,
                )
            }
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let secret_path = dir.path().join("provider-key");
        let mut secret_file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&secret_path)
            .unwrap();
        writeln!(secret_file, "fixture-secret-never-persist").unwrap();
        drop(secret_file);

        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        garnish.policy.openai_api_enabled = true;
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let initial_task = garnish.add_task(&task(project.id.clone())).unwrap();
        let budget = garnish
            .configure_api_budget(&NewApiBudget {
                project_id: project.id.clone(),
                provider: "openai".into(),
                account: "default".into(),
                enabled: true,
                secret_reference: format!("file:{}", secret_path.display()),
                currency: Some("USD".into()),
                currency_limit_micros: Some(1_000),
                token_limit: Some(1_000),
                request_limit: Some(10),
                period_start: now - Duration::minutes(1),
                period_end: now + Duration::days(1),
                allowed_models: vec!["gpt-fixture".into()],
                allowed_tools: vec![],
                allowed_roles: vec!["planner".into(), "implementer".into()],
                max_output_tokens: 18,
                max_retries: 2,
                max_concurrent_requests: 3,
                reason: "fake lifecycle".into(),
            })
            .unwrap();
        let price = garnish
            .configure_api_model_price(&NewApiModelPrice {
                provider: "openai".into(),
                account: "default".into(),
                model: "gpt-fixture".into(),
                currency: "USD".into(),
                input_micros_per_million: 1_000_000,
                cached_input_micros_per_million: 500_000,
                cache_creation_input_micros_per_million: 1_500_000,
                output_micros_per_million: 1_000_000,
                effective_from: now - Duration::minutes(1),
                effective_to: Some(now + Duration::days(1)),
                source: "fixture evidence".into(),
                reason: "fake lifecycle".into(),
            })
            .unwrap();
        let spec = ApiRequestSpec {
            provider: "openai".into(),
            model: "gpt-fixture".into(),
            instructions: "sensitive instructions".into(),
            input: "sensitive prompt".into(),
            max_output_tokens: 18,
            tools: vec![],
            stream: false,
        };
        let digest = api_request_digest(&budget, &spec, now).unwrap();
        let reservation = garnish
            .reserve_api_budget(&ApiReservationRequest {
                project_id: project.id.clone(),
                task_id: initial_task.id.clone(),
                provider: "openai".into(),
                account: "default".into(),
                model: "gpt-fixture".into(),
                role: "planner".into(),
                request_digest: digest,
                reserved_currency_micros: 100,
                reserved_input_tokens: 8,
                reserved_output_tokens: 18,
                reserved_attempts: 3,
                now,
                expires_at: now + Duration::minutes(5),
            })
            .unwrap();
        let mut transport = FakeTransport {
            sends: 0,
            terminal: false,
            transport_error: false,
        };
        let result = garnish
            .execute_reserved_api_request(&reservation.id, &spec, &mut transport, now)
            .unwrap();
        assert_eq!(transport.sends, 3);
        assert_eq!(result.attempts.len(), 3);
        assert_eq!(result.attempts[0].status, "retryable_failure");
        assert_eq!(result.attempts[1].status, "retryable_failure");
        assert_eq!(result.attempts[2].status, "succeeded");
        assert_eq!(
            result.attempts[0].failure_kind.as_deref(),
            Some("rate_limited")
        );
        assert_eq!(
            result.attempts[1].failure_kind.as_deref(),
            Some("transient")
        );
        assert_eq!(result.spend.cost_micros, 26);
        assert_eq!(result.spend.cached_input_tokens, 2);
        assert_eq!(result.spend.cache_creation_input_tokens, 1);
        assert_eq!(
            result.spend.pricing_evidence_id.as_deref(),
            Some(price.id.as_str())
        );
        assert_ne!(
            result.spend.provider_request_id_hash,
            "provider-request-fixture"
        );
        assert!(
            garnish
                .execute_reserved_api_request(&reservation.id, &spec, &mut transport, now)
                .is_err()
        );
        assert_eq!(transport.sends, 3);

        let terminal_task = garnish.add_task(&task(project.id.clone())).unwrap();
        let terminal_spec = ApiRequestSpec {
            input: "different bounded terminal prompt".into(),
            ..spec.clone()
        };
        let terminal_digest = api_request_digest(&budget, &terminal_spec, now).unwrap();
        let terminal_reservation = garnish
            .reserve_api_budget(&ApiReservationRequest {
                project_id: project.id,
                task_id: terminal_task.id,
                provider: "openai".into(),
                account: "default".into(),
                model: "gpt-fixture".into(),
                role: "planner".into(),
                request_digest: terminal_digest,
                reserved_currency_micros: 100,
                reserved_input_tokens: 8,
                reserved_output_tokens: 18,
                reserved_attempts: 3,
                now,
                expires_at: now + Duration::minutes(5),
            })
            .unwrap();
        let mut terminal_transport = FakeTransport {
            sends: 0,
            terminal: true,
            transport_error: false,
        };
        let terminal_error = garnish
            .execute_reserved_api_request(
                &terminal_reservation.id,
                &terminal_spec,
                &mut terminal_transport,
                now,
            )
            .unwrap_err();
        assert!(terminal_error.to_string().contains("authentication"));
        assert_eq!(terminal_transport.sends, 1);
        let terminal_attempts = garnish
            .api_dispatch_attempts(Some("fixture"))
            .unwrap()
            .into_iter()
            .filter(|attempt| attempt.reservation_id == terminal_reservation.id)
            .collect::<Vec<_>>();
        assert_eq!(terminal_attempts.len(), 1);
        assert_eq!(terminal_attempts[0].status, "terminal_failure");
        assert_eq!(
            terminal_attempts[0].failure_kind.as_deref(),
            Some("authentication")
        );

        let uncertain_task = garnish
            .add_task(&task(terminal_reservation.project_id.clone()))
            .unwrap();
        let uncertain_spec = ApiRequestSpec {
            input: "different bounded uncertain prompt".into(),
            ..spec.clone()
        };
        let uncertain_digest = api_request_digest(&budget, &uncertain_spec, now).unwrap();
        let uncertain_reservation = garnish
            .reserve_api_budget(&ApiReservationRequest {
                project_id: terminal_reservation.project_id.clone(),
                task_id: uncertain_task.id,
                provider: "openai".into(),
                account: "default".into(),
                model: "gpt-fixture".into(),
                role: "planner".into(),
                request_digest: uncertain_digest,
                reserved_currency_micros: 100,
                reserved_input_tokens: 8,
                reserved_output_tokens: 18,
                reserved_attempts: 3,
                now,
                expires_at: now + Duration::minutes(5),
            })
            .unwrap();
        let mut uncertain_transport = FakeTransport {
            sends: 0,
            terminal: false,
            transport_error: true,
        };
        let uncertain_error = garnish
            .execute_reserved_api_request(
                &uncertain_reservation.id,
                &uncertain_spec,
                &mut uncertain_transport,
                now,
            )
            .unwrap_err()
            .to_string();
        assert!(uncertain_error.contains("api.transport_uncertain"));
        assert!(!uncertain_error.contains("transport-secret-canary"));
        assert_eq!(uncertain_transport.sends, 1);
        let uncertain_attempts = garnish
            .api_dispatch_attempts(Some("fixture"))
            .unwrap()
            .into_iter()
            .filter(|attempt| attempt.reservation_id == uncertain_reservation.id)
            .collect::<Vec<_>>();
        assert_eq!(uncertain_attempts.len(), 1);
        assert_eq!(uncertain_attempts[0].status, "uncertain");
        assert_eq!(
            uncertain_attempts[0].failure_kind.as_deref(),
            Some("transport")
        );
        let capacity = garnish
            .db
            .api_route_capacity(&terminal_reservation.project_id, "openai", "default", now)
            .unwrap();
        assert!(capacity.allowed);
        assert_eq!(capacity.remaining_percent, Some(10.0));

        let data_dir = garnish.data_dir().to_path_buf();
        drop(garnish);
        let mut garnish = Garnish::open(&data_dir).unwrap();
        garnish.policy.openai_api_enabled = true;
        let mut replay_transport = FakeTransport {
            sends: 0,
            terminal: false,
            transport_error: false,
        };
        assert!(
            garnish
                .execute_reserved_api_request(
                    &uncertain_reservation.id,
                    &uncertain_spec,
                    &mut replay_transport,
                    now,
                )
                .is_err()
        );
        assert_eq!(replay_transport.sends, 0);
        let backup_path = dir.path().join("state-backup.db");
        let backup = garnish.create_backup(Some(&backup_path)).unwrap();
        let mut state_artifacts = fs::read_dir(garnish.data_dir())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| fs::read(entry.path()).unwrap())
            .collect::<Vec<_>>();
        state_artifacts.push(fs::read(backup.path).unwrap());
        for canary in [
            "fixture-secret-never-persist",
            "sensitive instructions",
            "sensitive prompt",
            "sensitive output",
            "provider-request-fixture",
            "provider-request-retry-1",
            "provider-request-retry-2",
            "provider-request-auth-failure",
            "transport-secret-canary",
        ] {
            assert!(
                state_artifacts
                    .iter()
                    .all(|artifact| !String::from_utf8_lossy(artifact).contains(canary))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "real-paid-api: requires the explicit scripts/test-real-api-smoke opt-in"]
    fn real_paid_api_smoke_is_explicit_and_accounted() {
        use crate::api_providers::{
            LiveApiTransport, LiveApiTransportConfig, api_request_conservative_input_token_bound,
            api_request_digest,
        };
        use std::time::Duration as StdDuration;

        const ACKNOWLEDGEMENT: &str = "I_ACCEPT_ONE_PAID_API_REQUEST";
        assert_eq!(
            std::env::var("GARNISH_ACKNOWLEDGE_PAID_API").as_deref(),
            Ok(ACKNOWLEDGEMENT),
            "the paid API acknowledgement is missing or incorrect"
        );
        let provider = std::env::var("GARNISH_REAL_API_PROVIDER")
            .expect("GARNISH_REAL_API_PROVIDER is required");
        assert!(
            matches!(provider.as_str(), "openai" | "anthropic"),
            "GARNISH_REAL_API_PROVIDER must be openai or anthropic"
        );
        let model =
            std::env::var("GARNISH_REAL_API_MODEL").expect("GARNISH_REAL_API_MODEL is required");
        let secret_reference = std::env::var("GARNISH_REAL_API_SECRET_REFERENCE")
            .expect("GARNISH_REAL_API_SECRET_REFERENCE is required");

        let directory = tempdir().unwrap();
        let source = directory.path().join("source");
        fixture_repo(&source);
        let now = Utc::now();
        let mut garnish = Garnish::open(directory.path().join("data")).unwrap();
        match provider.as_str() {
            "openai" => garnish.policy.openai_api_enabled = true,
            "anthropic" => garnish.policy.anthropic_api_enabled = true,
            _ => unreachable!(),
        }
        let project = garnish
            .add_project("real-api-smoke", "Real API smoke", &source)
            .unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();
        let budget = garnish
            .configure_api_budget(&NewApiBudget {
                project_id: project.id.clone(),
                provider: provider.clone(),
                account: "smoke".into(),
                enabled: true,
                secret_reference,
                currency: None,
                currency_limit_micros: None,
                token_limit: Some(65_536),
                request_limit: Some(1),
                period_start: now - Duration::minutes(1),
                period_end: now + Duration::minutes(10),
                allowed_models: vec![model.clone()],
                allowed_tools: vec![],
                allowed_roles: vec!["planner".into()],
                max_output_tokens: 32,
                max_retries: 0,
                max_concurrent_requests: 1,
                reason: "explicit one-request paid API smoke test".into(),
            })
            .unwrap();
        let spec = ApiRequestSpec {
            provider: provider.clone(),
            model: model.clone(),
            instructions: "Return only the requested text. Do not call tools.".into(),
            input: "Reply with exactly: OK".into(),
            max_output_tokens: 32,
            tools: vec![],
            stream: false,
        };
        let reserved_input_tokens =
            api_request_conservative_input_token_bound(&budget, &spec, now).unwrap();
        let reservation = garnish
            .reserve_api_budget(&ApiReservationRequest {
                project_id: project.id,
                task_id: task.id,
                provider,
                account: "smoke".into(),
                model,
                role: "planner".into(),
                request_digest: api_request_digest(&budget, &spec, now).unwrap(),
                reserved_currency_micros: 0,
                reserved_input_tokens,
                reserved_output_tokens: 32,
                reserved_attempts: 1,
                now,
                expires_at: now + Duration::minutes(5),
            })
            .unwrap();
        let mut transport = LiveApiTransport::new(LiveApiTransportConfig {
            network_enabled: true,
            connect_timeout: StdDuration::from_secs(10),
            request_timeout: StdDuration::from_secs(120),
            max_response_bytes: 1024 * 1024,
        })
        .unwrap();
        let result = garnish
            .execute_reserved_api_request(&reservation.id, &spec, &mut transport, now)
            .unwrap();
        assert_eq!(result.attempts.len(), 1);
        assert_eq!(result.attempts[0].status, "succeeded");
        assert_eq!(result.spend.reservation_id, reservation.id);
        assert!(result.spend.input_tokens <= reserved_input_tokens);
        assert!(result.spend.output_tokens <= 32);
        assert_eq!(result.spend.cost_micros, 0);
        assert_eq!(result.spend.currency, None);
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "real-paid-api-patch: requires the explicit scripts/test-real-api-patch-smoke opt-in"]
    fn real_paid_api_patch_smoke_is_explicit_scoped_and_verified() {
        use std::time::Duration as StdDuration;

        const ACKNOWLEDGEMENT: &str = "I_ACCEPT_ONE_PAID_API_PATCH_REQUEST";
        assert_eq!(
            std::env::var("GARNISH_ACKNOWLEDGE_PAID_API_PATCH").as_deref(),
            Ok(ACKNOWLEDGEMENT),
            "the paid API patch acknowledgement is missing or incorrect"
        );
        let provider = std::env::var("GARNISH_REAL_API_PROVIDER")
            .expect("GARNISH_REAL_API_PROVIDER is required");
        assert!(matches!(provider.as_str(), "openai" | "anthropic"));
        let model =
            std::env::var("GARNISH_REAL_API_MODEL").expect("GARNISH_REAL_API_MODEL is required");
        let secret_reference = std::env::var("GARNISH_REAL_API_SECRET_REFERENCE")
            .expect("GARNISH_REAL_API_SECRET_REFERENCE is required");

        let directory = tempdir().unwrap();
        let source = directory.path().join("source");
        fixture_repo(&source);
        let now = Utc::now();
        let data_dir = directory.path().join("data");
        let mut garnish = Garnish::open(&data_dir).unwrap();
        let project = garnish
            .add_project("real-api-patch-smoke", "Real API patch smoke", &source)
            .unwrap();
        let mut new_task = task(project.id.clone());
        new_task.title = "Create the exact scoped smoke-test file".into();
        new_task.goal = "Create result.txt containing exactly one line: done".into();
        new_task.required_capabilities = vec![API_PATCH_CAPABILITY.into()];
        new_task.fake_write_path = None;
        new_task.fake_write_content = None;
        let task = garnish.add_task(&new_task).unwrap();
        garnish
            .configure_api_budget(&NewApiBudget {
                project_id: project.id,
                provider: provider.clone(),
                account: "smoke".into(),
                enabled: true,
                secret_reference,
                currency: None,
                currency_limit_micros: None,
                token_limit: Some(100_000),
                request_limit: Some(1),
                period_start: now - Duration::minutes(1),
                period_end: now + Duration::minutes(10),
                allowed_models: vec![model.clone()],
                allowed_tools: vec![API_PATCH_TOOL.into()],
                allowed_roles: vec!["implementer".into()],
                max_output_tokens: 512,
                max_retries: 0,
                max_concurrent_requests: 1,
                reason: "explicit one-request paid API patch smoke test".into(),
            })
            .unwrap();
        garnish
            .set_task_route_pin(
                &task.id,
                "api",
                &provider,
                "smoke",
                "explicit paid API patch smoke",
            )
            .unwrap();
        garnish
            .configure_api_request_plan_at(
                &NewApiRequestPlan {
                    task_id: task.id.clone(),
                    provider: provider.clone(),
                    account: "smoke".into(),
                    enabled: true,
                    model: model.clone(),
                    role: "implementer".into(),
                    max_input_tokens: 65_536,
                    max_output_tokens: 512,
                    max_retries: 0,
                    stream: false,
                    reason: "exact one-request paid API patch smoke".into(),
                },
                now,
            )
            .unwrap();
        let config = SchedulerDaemonConfig {
            instance_id: "real-api-patch-smoke".into(),
            hostname: "local".into(),
            adapter: "api".into(),
            provider: provider.clone(),
            account: "smoke".into(),
            route_candidates: vec![],
            max_active_claims: 1,
            max_active_per_adapter: 1,
            max_active_per_account: 1,
            poll_interval: StdDuration::from_secs(1),
            leader_ttl: StdDuration::from_secs(30),
            claim_ttl: StdDuration::from_secs(300),
            max_ticks: Some(1),
            execute_fake_claims: false,
            execute_api_claims: true,
            paid_api_acknowledgement: Some(PAID_API_DAEMON_ACKNOWLEDGEMENT.into()),
            execute_api_patches: true,
            api_patch_acknowledgement: Some(API_PATCH_DAEMON_ACKNOWLEDGEMENT.into()),
        };
        let mut transport = LiveApiTransport::new(LiveApiTransportConfig {
            network_enabled: true,
            connect_timeout: StdDuration::from_secs(10),
            request_timeout: StdDuration::from_secs(120),
            max_response_bytes: 1024 * 1024,
        })
        .unwrap();
        let summary = garnish
            .run_scheduler_daemon_with(
                &config,
                &AtomicBool::new(false),
                Some(&mut transport),
                Utc::now,
                |_| {},
            )
            .unwrap();
        assert_eq!(summary.claims_created, 1);
        assert_eq!(summary.runs_completed, 1);
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Review);
        assert_eq!(
            fs::read_to_string(
                data_dir
                    .join("worktrees/real-api-patch-smoke")
                    .join(&task.id)
                    .join("result.txt")
            )
            .unwrap(),
            "done\n"
        );
        assert!(!source.join("result.txt").exists());
        assert_eq!(
            garnish
                .api_reservations(Some("real-api-patch-smoke"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            garnish
                .api_spend(Some("real-api-patch-smoke"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn api_route_uses_project_budget_not_subscription_quota_and_fails_closed() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();

        let denied = garnish
            .route_task_at(&task.id, "api", "openai", "paid", now)
            .unwrap();
        assert!(!denied.allowed);
        assert_eq!(denied.reason_code, "api.policy_disabled");
        assert!(denied.quota.is_empty());

        enable_fixture_api_route(&mut garnish, &project.id, now);
        let allowed = garnish
            .route_task_at(&task.id, "api", "openai", "paid", now)
            .unwrap();
        assert!(allowed.allowed);
        assert_eq!(allowed.reason_code, "route.allowed");
        assert!(allowed.quota.is_empty());
        assert_eq!(allowed.candidates[0].forecast_percent, 0.0);
        assert_eq!(allowed.candidates[0].forecast_source, "api_budget_capacity");
        assert_eq!(
            allowed.candidates[0].minimum_effective_remaining_percent,
            Some(100.0)
        );
        garnish
            .reserve_api_budget(&ApiReservationRequest {
                project_id: project.id,
                task_id: task.id.clone(),
                provider: "openai".into(),
                account: "paid".into(),
                model: "model-fixture".into(),
                role: "implementer".into(),
                request_digest: "d".repeat(64),
                reserved_currency_micros: 100,
                reserved_input_tokens: 10,
                reserved_output_tokens: 10,
                reserved_attempts: 1,
                now,
                expires_at: now + Duration::minutes(5),
            })
            .unwrap();
        let saturated = garnish
            .route_task_at(&task.id, "api", "openai", "paid", now)
            .unwrap();
        assert!(!saturated.allowed);
        assert_eq!(saturated.reason_code, "api.concurrency_limit");
    }

    #[test]
    fn low_subscription_quota_never_selects_a_paid_api_fallback() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();
        enable_fixture_api_route(&mut garnish, &project.id, now);
        garnish
            .set_quota(
                "fake",
                "subscription",
                "five_hour",
                Some(5.0),
                20.0,
                None,
                "fixture",
                None,
            )
            .unwrap();
        let targets = vec![
            RouteTarget {
                adapter: "fake".into(),
                provider: "fake".into(),
                account: "subscription".into(),
            },
            RouteTarget {
                adapter: "api".into(),
                provider: "openai".into(),
                account: "paid".into(),
            },
        ];
        let (decision, selected) = garnish
            .route_task_candidates_at(&task.id, &targets, now)
            .unwrap();
        assert!(!decision.allowed);
        assert!(selected.is_none());
        let api = decision
            .candidates
            .iter()
            .find(|candidate| candidate.adapter == "api")
            .unwrap();
        assert_eq!(api.reason_code, "api.explicit_selection_required");
        let subscription = decision
            .candidates
            .iter()
            .find(|candidate| candidate.adapter == "fake")
            .unwrap();
        assert_eq!(subscription.reason_code, "quota.insufficient");
    }

    #[test]
    fn explicit_api_pin_requires_a_durable_exact_request_plan() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();
        enable_fixture_api_route(&mut garnish, &project.id, now);
        let (unpinned, selected) = garnish
            .route_task_candidates_at(
                &task.id,
                &[RouteTarget {
                    adapter: "api".into(),
                    provider: "openai".into(),
                    account: "paid".into(),
                }],
                now,
            )
            .unwrap();
        assert!(!unpinned.allowed);
        assert!(selected.is_none());
        assert_eq!(unpinned.reason_code, "api.explicit_selection_required");
        garnish
            .set_task_route_pin(&task.id, "api", "openai", "paid", "explicit paid API route")
            .unwrap();
        let (decision, selected) = garnish
            .route_task_candidates_at(
                &task.id,
                &[
                    RouteTarget {
                        adapter: "fake".into(),
                        provider: "fake".into(),
                        account: "subscription".into(),
                    },
                    RouteTarget {
                        adapter: "api".into(),
                        provider: "openai".into(),
                        account: "paid".into(),
                    },
                ],
                now,
            )
            .unwrap();
        assert!(!decision.allowed);
        assert!(selected.is_none());
        assert_eq!(decision.reason_code, "api.request_plan_missing");
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Ready);
        assert!(
            garnish
                .api_reservations(Some("fixture"))
                .unwrap()
                .is_empty()
        );

        let disabled = garnish
            .configure_api_request_plan_at(
                &NewApiRequestPlan {
                    task_id: task.id.clone(),
                    provider: "openai".into(),
                    account: "paid".into(),
                    enabled: false,
                    model: "model-fixture".into(),
                    role: "implementer".into(),
                    max_input_tokens: 10_000,
                    max_output_tokens: 50,
                    max_retries: 0,
                    stream: false,
                    reason: "keep paid execution disabled".into(),
                },
                now,
            )
            .unwrap();
        assert!(!disabled.enabled);
        let (decision, selected) = garnish
            .route_task_candidates_at(
                &task.id,
                &[RouteTarget {
                    adapter: "api".into(),
                    provider: "openai".into(),
                    account: "paid".into(),
                }],
                now,
            )
            .unwrap();
        assert!(!decision.allowed);
        assert!(selected.is_none());
        assert_eq!(decision.reason_code, "api.request_plan_disabled");
    }

    #[test]
    fn response_only_api_scheduler_denies_write_risk_before_claim() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();
        enable_fixture_api_route(&mut garnish, &project.id, now);
        garnish
            .set_task_route_pin(&task.id, "api", "openai", "paid", "explicit paid API route")
            .unwrap();
        garnish
            .configure_api_request_plan_at(
                &NewApiRequestPlan {
                    task_id: task.id.clone(),
                    provider: "openai".into(),
                    account: "paid".into(),
                    enabled: true,
                    model: "model-fixture".into(),
                    role: "implementer".into(),
                    max_input_tokens: 10_000,
                    max_output_tokens: 50,
                    max_retries: 0,
                    stream: false,
                    reason: "write-risk denial fixture".into(),
                },
                now,
            )
            .unwrap();

        let (decision, selected) = garnish
            .route_task_candidates_at(
                &task.id,
                &[RouteTarget {
                    adapter: "api".into(),
                    provider: "openai".into(),
                    account: "paid".into(),
                }],
                now,
            )
            .unwrap();
        assert!(!decision.allowed);
        assert!(selected.is_none());
        assert_eq!(decision.reason_code, "api.execution_tools_unavailable");
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Ready);
        assert!(
            garnish
                .api_reservations(Some("fixture"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn scheduler_uses_current_request_plan_and_reserves_every_retry_attempt_atomically() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let mut api_task = task(project.id.clone());
        api_task.risk_class = 0;
        let task = garnish.add_task(&api_task).unwrap();
        enable_fixture_api_route(&mut garnish, &project.id, now);
        garnish
            .configure_api_budget(&NewApiBudget {
                project_id: project.id.clone(),
                provider: "openai".into(),
                account: "paid".into(),
                enabled: true,
                secret_reference: "env:FIXTURE_API_KEY".into(),
                currency: Some("USD".into()),
                currency_limit_micros: Some(1_000_000),
                token_limit: Some(100_000),
                request_limit: Some(10),
                period_start: now - Duration::minutes(1),
                period_end: now + Duration::days(1),
                allowed_models: vec!["model-fixture".into()],
                allowed_tools: vec![],
                allowed_roles: vec!["implementer".into()],
                max_output_tokens: 1_000,
                max_retries: 2,
                max_concurrent_requests: 1,
                reason: "retry-aware plan fixture".into(),
            })
            .unwrap();
        garnish
            .set_task_route_pin(&task.id, "api", "openai", "paid", "explicit paid API route")
            .unwrap();
        let plan = garnish
            .configure_api_request_plan_at(
                &NewApiRequestPlan {
                    task_id: task.id.clone(),
                    provider: "openai".into(),
                    account: "paid".into(),
                    enabled: true,
                    model: "model-fixture".into(),
                    role: "implementer".into(),
                    max_input_tokens: 10_000,
                    max_output_tokens: 50,
                    max_retries: 2,
                    stream: false,
                    reason: "exact scheduler request fixture".into(),
                },
                now,
            )
            .unwrap();
        assert_eq!(plan.task_version, garnish.task(&task.id).unwrap().version);
        assert_eq!(garnish.api_request_plans(Some(&task.id)).unwrap().len(), 1);

        garnish
            .register_scheduler("planned-api", "fixture", 1, now)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("planned-api", now, std::time::Duration::from_secs(60))
            .unwrap();
        let tick = garnish
            .scheduler_tick_candidates_with_limits_at(
                "planned-api",
                leader.generation,
                &[RouteTarget {
                    adapter: "api".into(),
                    provider: "openai".into(),
                    account: "paid".into(),
                }],
                now,
                1,
                1,
                1,
                std::time::Duration::from_secs(60),
            )
            .unwrap();
        assert_eq!(tick.claims.len(), 1);
        let reservations = garnish.api_reservations(Some("fixture")).unwrap();
        assert_eq!(reservations.len(), 1);
        let reservation = &reservations[0];
        assert_eq!(reservation.reserved_requests, 3);
        assert_eq!(reservation.per_attempt_input_tokens, 10_000);
        assert_eq!(reservation.per_attempt_output_tokens, 50);
        assert_eq!(reservation.reserved_input_tokens, 30_000);
        assert_eq!(reservation.reserved_output_tokens, 150);
        assert_eq!(reservation.reserved_currency_micros, 45_300);
        assert_eq!(
            reservation.claim_id.as_deref(),
            Some(tick.claims[0].id.as_str())
        );
    }

    #[test]
    fn exact_api_claim_atomically_binds_budget_and_releases_on_scheduler_stop() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let data_dir = dir.path().join("data");
        let mut garnish = Garnish::open(&data_dir).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();
        enable_fixture_api_route(&mut garnish, &project.id, now);
        garnish
            .set_task_route_pin(&task.id, "api", "openai", "paid", "explicit paid API route")
            .unwrap();
        garnish
            .register_scheduler("api-claim", "fixture", 1, now)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("api-claim", now, std::time::Duration::from_secs(60))
            .unwrap();
        let spec = ApiRequestSpec {
            provider: "openai".into(),
            model: "model-fixture".into(),
            instructions: "bounded fixture instructions".into(),
            input: "bounded fixture input".into(),
            max_output_tokens: 50,
            tools: vec![],
            stream: false,
        };

        let denied = garnish
            .claim_exact_api_request_at(
                "api-claim",
                leader.generation,
                &task.id,
                "openai",
                "paid",
                "implementer",
                &spec,
                100_000,
                now,
                std::time::Duration::from_secs(60),
                1,
                1,
                1,
            )
            .unwrap_err();
        assert!(denied.to_string().contains("api.token_budget_exhausted"));
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Ready);
        assert_eq!(garnish.db.active_scheduler_claim_count(now).unwrap(), 0);
        assert!(
            garnish
                .api_reservations(Some("fixture"))
                .unwrap()
                .is_empty()
        );

        let bound = garnish
            .claim_exact_api_request_at(
                "api-claim",
                leader.generation,
                &task.id,
                "openai",
                "paid",
                "implementer",
                &spec,
                100,
                now,
                std::time::Duration::from_secs(60),
                1,
                1,
                1,
            )
            .unwrap();
        assert_eq!(
            bound.reservation.claim_id.as_deref(),
            Some(bound.claim.id.as_str())
        );
        assert_eq!(bound.reservation.run_id, None);
        assert_eq!(bound.reservation.reserved_currency_micros, 250);
        assert_eq!(
            bound.reservation.request_digest,
            api_request_digest(
                &garnish
                    .db
                    .latest_api_budget(&project.id, "openai", "paid")
                    .unwrap(),
                &spec,
                now,
            )
            .unwrap()
        );
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Leased);
        assert!(
            garnish
                .prepare_reserved_api_request(&bound.reservation.id, &spec, now)
                .unwrap_err()
                .to_string()
                .contains("api.claim_not_consumed")
        );
        assert!(
            garnish
                .release_api_reservation(&bound.reservation.id, "manual bypass", now)
                .is_err()
        );
        drop(garnish);
        let mut garnish = Garnish::open(&data_dir).unwrap();
        let durable = garnish.db.api_reservation(&bound.reservation.id).unwrap();
        assert_eq!(durable.claim_id.as_deref(), Some(bound.claim.id.as_str()));
        garnish
            .db
            .heartbeat_scheduler_claims(
                "api-claim",
                leader.generation,
                now + Duration::seconds(10),
                std::time::Duration::from_secs(120),
            )
            .unwrap();
        assert_eq!(
            garnish
                .api_reservations(Some("fixture"))
                .unwrap()
                .first()
                .unwrap()
                .expires_at,
            now + Duration::seconds(130)
        );

        assert_eq!(
            garnish
                .stop_scheduler("api-claim", now + Duration::seconds(11))
                .unwrap(),
            vec![task.id.clone()]
        );
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Ready);
        let released = garnish.api_reservations(Some("fixture")).unwrap();
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].status, "released");
        assert_eq!(
            released[0].release_reason.as_deref(),
            Some("scheduler_stopped")
        );
    }

    #[test]
    fn dispatched_scheduler_bound_api_reservation_is_retained_after_run_failure() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();
        enable_fixture_api_route(&mut garnish, &project.id, now);
        garnish
            .set_task_route_pin(&task.id, "api", "openai", "paid", "explicit paid API route")
            .unwrap();
        garnish
            .register_scheduler("api-run", "fixture", 1, now)
            .unwrap();
        let leader = garnish
            .acquire_scheduler_leader("api-run", now, std::time::Duration::from_secs(60))
            .unwrap();
        let spec = ApiRequestSpec {
            provider: "openai".into(),
            model: "model-fixture".into(),
            instructions: "bounded fixture instructions".into(),
            input: "bounded fixture input".into(),
            max_output_tokens: 50,
            tools: vec![],
            stream: false,
        };
        let bound = garnish
            .claim_exact_api_request_at(
                "api-run",
                leader.generation,
                &task.id,
                "openai",
                "paid",
                "implementer",
                &spec,
                100,
                now,
                std::time::Duration::from_secs(60),
                1,
                1,
                1,
            )
            .unwrap();
        assert!(
            garnish
                .claim_api_dispatch(&bound.reservation.id, now + Duration::seconds(1))
                .is_err()
        );
        let run_id = Ulid::new().to_string();
        garnish
            .db
            .begin_claimed_run(
                &bound.claim.id,
                "api-run",
                leader.generation,
                &run_id,
                "api",
                "/fixture/worktree",
                "fixture-branch",
                "fixture-base",
                now + Duration::seconds(1),
                std::time::Duration::from_secs(120),
            )
            .unwrap();
        let running = garnish.db.api_reservation(&bound.reservation.id).unwrap();
        assert_eq!(running.run_id.as_deref(), Some(run_id.as_str()));
        garnish
            .claim_api_dispatch(&bound.reservation.id, now + Duration::seconds(2))
            .unwrap();
        garnish.db.finish_run(&run_id, "failed", None, 1).unwrap();
        let retained = garnish.db.api_reservation(&bound.reservation.id).unwrap();
        assert_eq!(retained.status, "dispatched");
        assert_eq!(retained.release_reason, None);
    }

    #[test]
    fn invalid_api_budget_cannot_become_enabled() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = Utc::now();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let result = garnish.configure_api_budget(&NewApiBudget {
            project_id: project.id,
            provider: "openai".into(),
            account: "default".into(),
            enabled: true,
            secret_reference: "sk-not-a-reference".into(),
            currency: None,
            currency_limit_micros: None,
            token_limit: None,
            request_limit: None,
            period_start: now,
            period_end: now + Duration::days(30),
            allowed_models: vec!["gpt-fixture".into()],
            allowed_tools: vec![],
            allowed_roles: vec!["planner".into()],
            max_output_tokens: 100,
            max_retries: 0,
            max_concurrent_requests: 1,
            reason: "invalid fixture".into(),
        });
        assert!(result.is_err());
        assert!(garnish.api_budgets(Some("fixture")).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn api_secret_canary_never_enters_state_backup_diagnostics_or_ui() {
        use crate::{secrets::SecretReference, web_ui::operator_snapshot};
        use std::{io::Write, os::unix::fs::OpenOptionsExt};

        const CANARY: &str = "secret-canary-never-persist-1d37a4b2";
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let secret_path = dir.path().join("provider-api-key");
        let mut secret_file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&secret_path)
            .unwrap();
        writeln!(secret_file, "{CANARY}").unwrap();
        drop(secret_file);

        let data_dir = dir.path().join("state");
        let now = Utc::now();
        let mut garnish = Garnish::open(&data_dir).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let locator = format!("file:{}", secret_path.display());
        garnish
            .configure_api_budget(&NewApiBudget {
                project_id: project.id,
                provider: "openai".into(),
                account: "default".into(),
                enabled: true,
                secret_reference: locator.clone(),
                currency: Some("USD".into()),
                currency_limit_micros: Some(1_000),
                token_limit: Some(1_000),
                request_limit: Some(1),
                period_start: now - Duration::minutes(1),
                period_end: now + Duration::days(1),
                allowed_models: vec!["gpt-fixture".into()],
                allowed_tools: vec![],
                allowed_roles: vec!["planner".into()],
                max_output_tokens: 100,
                max_retries: 0,
                max_concurrent_requests: 1,
                reason: "secret canary fixture".into(),
            })
            .unwrap();
        let secret = SecretReference::parse(&locator).unwrap().resolve().unwrap();
        assert!(secret.expose(|bytes| bytes == CANARY.as_bytes()));
        assert!(!format!("{secret:?}").contains(CANARY));
        drop(secret);

        let diagnostics = serde_json::to_vec(&garnish.diagnostics().unwrap()).unwrap();
        let snapshot = serde_json::to_vec(&operator_snapshot(&garnish).unwrap()).unwrap();
        assert!(
            !diagnostics
                .windows(CANARY.len())
                .any(|part| part == CANARY.as_bytes())
        );
        assert!(
            !snapshot
                .windows(CANARY.len())
                .any(|part| part == CANARY.as_bytes())
        );
        garnish.create_backup(None).unwrap();
        drop(garnish);

        fn assert_tree_has_no_canary(path: &Path) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    assert_tree_has_no_canary(&path);
                } else {
                    let bytes = fs::read(path).unwrap();
                    assert!(
                        !bytes
                            .windows(CANARY.len())
                            .any(|part| part == CANARY.as_bytes())
                    );
                }
            }
        }
        assert_tree_has_no_canary(&data_dir);
    }

    #[cfg(unix)]
    #[test]
    fn api_request_preparation_requires_policy_live_exact_reservation_and_latest_budget() {
        use crate::api_providers::{ApiRequestSpec, ApiToolDefinition, api_request_digest};
        use std::{io::Write, os::unix::fs::OpenOptionsExt};

        const SECRET: &str = "reserved-request-secret-canary-a103";
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let secret_path = dir.path().join("provider-api-key");
        let mut secret_file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&secret_path)
            .unwrap();
        writeln!(secret_file, "{SECRET}").unwrap();
        drop(secret_file);

        let now = Utc::now();
        let mut garnish = Garnish::open(dir.path().join("state")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id.clone())).unwrap();
        let config = NewApiBudget {
            project_id: project.id.clone(),
            provider: "openai".into(),
            account: "default".into(),
            enabled: true,
            secret_reference: format!("file:{}", secret_path.display()),
            currency: Some("USD".into()),
            currency_limit_micros: Some(1_000),
            token_limit: Some(1_000),
            request_limit: Some(10),
            period_start: now - Duration::minutes(1),
            period_end: now + Duration::days(1),
            allowed_models: vec!["gpt-fixture".into()],
            allowed_tools: vec!["read_fixture".into()],
            allowed_roles: vec!["planner".into()],
            max_output_tokens: 20,
            max_retries: 0,
            max_concurrent_requests: 1,
            reason: "request preparation fixture".into(),
        };
        let budget = garnish.configure_api_budget(&config).unwrap();
        let spec = ApiRequestSpec {
            provider: "openai".into(),
            model: "gpt-fixture".into(),
            instructions: "bounded fixture instructions".into(),
            input: "bounded fixture input".into(),
            max_output_tokens: 20,
            tools: vec![ApiToolDefinition {
                name: "read_fixture".into(),
                description: "Read one fixture".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }],
            stream: false,
        };
        let digest = api_request_digest(&budget, &spec, now).unwrap();
        garnish.policy.openai_api_enabled = true;
        let reservation = garnish
            .reserve_api_budget(&ApiReservationRequest {
                project_id: project.id,
                task_id: task.id,
                provider: "openai".into(),
                account: "default".into(),
                model: "gpt-fixture".into(),
                role: "planner".into(),
                request_digest: digest,
                reserved_currency_micros: 100,
                reserved_input_tokens: 10,
                reserved_output_tokens: 20,
                reserved_attempts: 1,
                now,
                expires_at: now + Duration::minutes(5),
            })
            .unwrap();

        garnish.policy.openai_api_enabled = false;
        let error = garnish
            .prepare_reserved_api_request(&reservation.id, &spec, now)
            .unwrap_err()
            .to_string();
        assert!(error.contains("api.policy_disabled"));
        assert!(!error.contains(SECRET));

        garnish.policy.openai_api_enabled = true;
        let prepared = garnish
            .prepare_reserved_api_request(&reservation.id, &spec, now)
            .unwrap();
        assert_eq!(prepared.endpoint(), "https://api.openai.com/v1/responses");
        assert!(!format!("{prepared:?}").contains(SECRET));

        let mut changed = spec.clone();
        changed.input.push_str(" changed");
        let error = garnish
            .prepare_reserved_api_request(&reservation.id, &changed, now)
            .unwrap_err()
            .to_string();
        assert!(error.contains("api.request_digest_mismatch"));
        assert!(!error.contains(SECRET));

        garnish.configure_api_budget(&config).unwrap();
        let error = garnish
            .prepare_reserved_api_request(&reservation.id, &spec, now)
            .unwrap_err()
            .to_string();
        assert!(error.contains("api.budget_superseded"));
    }

    #[test]
    fn route_records_and_gates_on_the_exact_historical_forecast() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = Utc::now();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id)).unwrap();
        for index in 1..=5 {
            garnish
                .record_quota_usage_sample(
                    &format!("route-run-{index}"),
                    "fake",
                    "fake",
                    "test",
                    "five_hour",
                    60,
                    f64::from(index),
                    "fixture-adapter",
                    "collector_measured",
                    now + Duration::seconds(index.into()),
                )
                .unwrap();
        }
        garnish
            .set_quota(
                "fake",
                "test",
                "five_hour",
                Some(25.0),
                20.0,
                None,
                "fixture",
                None,
            )
            .unwrap();
        let denied = garnish
            .route_task_at(&task.id, "fake", "fake", "test", now)
            .unwrap();
        assert!(!denied.allowed);
        assert_eq!(denied.reason_code, "quota.insufficient");
        assert_eq!(denied.candidates[0].forecast_source, "historical_p90");
        assert_eq!(denied.candidates[0].forecast_sample_count, 5);
        assert_eq!(denied.candidates[0].forecast_percent, 6.0);
        assert_eq!(denied.required_headroom_percent, 26.0);
    }

    #[test]
    fn recovery_orphans_a_verifier_with_its_expired_implementer_once() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let now = Utc::now();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let task = garnish.add_task(&task(project.id)).unwrap();
        prepare_active_run(&mut garnish, &task, "implementer-recovery", now);
        garnish
            .db
            .transition_task(
                &task.id,
                TaskStatus::Running,
                TaskStatus::Verifying,
                "fixture",
            )
            .unwrap();
        let route = build_verifier_route(
            &garnish.policy,
            &task.id,
            &RouteTarget {
                adapter: "fake".into(),
                provider: "fake".into(),
                account: "test".into(),
            },
            &[RouteTarget {
                adapter: "garnish-command-verifier".into(),
                provider: "local".into(),
                account: "default".into(),
            }],
            now,
        );
        garnish.db.record_route(&route).unwrap();
        garnish
            .db
            .create_verifier_run(
                "verifier-recovery",
                "implementer-recovery",
                &task.id,
                "garnish-command-verifier",
                &route.id,
                "/fixture/verifier",
                "0123456789abcdef",
                now,
            )
            .unwrap();

        assert_eq!(
            garnish.recover_at(now + Duration::minutes(11)).unwrap(),
            vec![task.id.clone()]
        );
        assert!(
            garnish
                .recover_at(now + Duration::minutes(12))
                .unwrap()
                .is_empty()
        );
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Paused);
        let runs = garnish.db.run_records_for_task(&task.id).unwrap();
        assert!(runs.iter().all(|run| run.status == "orphaned"));
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
        assert_eq!(summary.verifier_adapter, "garnish-command-verifier");
        assert_ne!(summary.run_id, summary.verifier_run_id);
        assert!(Path::new(&summary.patch_path).exists());
        let runs = garnish.db.run_records_for_task(&task.id).unwrap();
        assert_eq!(runs.len(), 2);
        let implementer = runs.iter().find(|run| run.role == "implementer").unwrap();
        let verifier = runs.iter().find(|run| run.role == "verifier").unwrap();
        assert_eq!(
            verifier.parent_run_id.as_deref(),
            Some(implementer.id.as_str())
        );
        assert_eq!(verifier.status, "passed");
        assert_ne!(verifier.worktree_path, implementer.worktree_path);
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
            execute_api_claims: false,
            paid_api_acknowledgement: None,
            execute_api_patches: false,
            api_patch_acknowledgement: None,
        };
        let shutdown = AtomicBool::new(false);
        let mut instant = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let summary = garnish
            .run_scheduler_daemon_with(
                &config,
                &shutdown,
                None,
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
            execute_api_claims: false,
            paid_api_acknowledgement: None,
            execute_api_patches: false,
            api_patch_acknowledgement: None,
        };
        let shutdown = AtomicBool::new(false);
        let mut instant = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let summary = garnish
            .run_scheduler_daemon_with(
                &config,
                &shutdown,
                None,
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
    fn daemon_executes_only_an_explicitly_pinned_planned_api_task_with_fake_transport() {
        use crate::api_providers::ApiTransportResponse;
        use std::{io::Write, os::unix::fs::OpenOptionsExt};

        struct SchedulerApiTransport {
            sends: usize,
            uncertain: bool,
        }

        impl ApiTransport for SchedulerApiTransport {
            fn send(&mut self, request: &PreparedApiRequest) -> Result<ApiTransportResponse> {
                self.sends += 1;
                request.with_sensitive_parts(|endpoint, _, _, _, secret, body| {
                    assert_eq!(endpoint, "https://api.openai.com/v1/responses");
                    assert_eq!(secret, b"scheduler-api-secret-canary");
                    let body: serde_json::Value = serde_json::from_slice(body).unwrap();
                    assert_eq!(body["model"], "model-fixture");
                    assert_eq!(body["store"], false);
                });
                if self.uncertain {
                    bail!("scheduler-transport-error-canary");
                }
                ApiTransportResponse::new(
                    200,
                    "scheduler-provider-request-canary".into(),
                    br#"{
                        "id":"scheduler-response-id-canary",
                        "object":"response",
                        "status":"completed",
                        "model":"model-fixture",
                        "output":[{"type":"message","status":"completed","role":"assistant",
                          "content":[{"type":"output_text","text":"scheduler-output-canary","annotations":[]}]}],
                        "usage":{"input_tokens":8,"output_tokens":2,"total_tokens":10}
                    }"#
                    .to_vec(),
                    false,
                )
            }
        }

        fn collect_file_bytes(path: &Path, output: &mut Vec<Vec<u8>>) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let file_type = entry.file_type().unwrap();
                if file_type.is_dir() {
                    collect_file_bytes(&entry.path(), output);
                } else if file_type.is_file() {
                    output.push(fs::read(entry.path()).unwrap());
                }
            }
        }

        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let secret_path = dir.path().join("scheduler-provider-key");
        let mut secret_file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&secret_path)
            .unwrap();
        writeln!(secret_file, "scheduler-api-secret-canary").unwrap();
        drop(secret_file);

        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 20, 12, 0, 0).unwrap();
        let data_dir = dir.path().join("data");
        let mut garnish = Garnish::open(&data_dir).unwrap();
        let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
        let mut new_task = task(project.id.clone());
        new_task.title = "Verify an API-only result".into();
        new_task.goal = "Return a bounded provider result without repository writes".into();
        new_task.acceptance = vec!["the isolated worktree remains unchanged".into()];
        new_task.verification_argv = vec!["git".into(), "diff".into(), "--quiet".into()];
        new_task.risk_class = 0;
        new_task.fake_write_path = None;
        new_task.fake_write_content = None;
        let task = garnish.add_task(&new_task).unwrap();
        garnish
            .configure_api_budget(&NewApiBudget {
                project_id: project.id.clone(),
                provider: "openai".into(),
                account: "paid".into(),
                enabled: true,
                secret_reference: format!("file:{}", secret_path.display()),
                currency: Some("USD".into()),
                currency_limit_micros: Some(1_000_000),
                token_limit: Some(100_000),
                request_limit: Some(10),
                period_start: now - Duration::minutes(1),
                period_end: now + Duration::days(1),
                allowed_models: vec!["model-fixture".into()],
                allowed_tools: vec![],
                allowed_roles: vec!["implementer".into()],
                max_output_tokens: 100,
                max_retries: 0,
                max_concurrent_requests: 1,
                reason: "explicit scheduler API fixture".into(),
            })
            .unwrap();
        garnish
            .configure_api_model_price(&NewApiModelPrice {
                provider: "openai".into(),
                account: "paid".into(),
                model: "model-fixture".into(),
                currency: "USD".into(),
                input_micros_per_million: 1_000_000,
                cached_input_micros_per_million: 500_000,
                cache_creation_input_micros_per_million: 1_500_000,
                output_micros_per_million: 2_000_000,
                effective_from: now - Duration::minutes(1),
                effective_to: Some(now + Duration::days(1)),
                source: "fixture price evidence".into(),
                reason: "explicit scheduler API fixture".into(),
            })
            .unwrap();
        garnish
            .set_task_route_pin(
                &task.id,
                "api",
                "openai",
                "paid",
                "explicit paid API execution",
            )
            .unwrap();
        garnish
            .configure_api_request_plan_at(
                &NewApiRequestPlan {
                    task_id: task.id.clone(),
                    provider: "openai".into(),
                    account: "paid".into(),
                    enabled: true,
                    model: "model-fixture".into(),
                    role: "implementer".into(),
                    max_input_tokens: 10_000,
                    max_output_tokens: 50,
                    max_retries: 0,
                    stream: false,
                    reason: "exact paid scheduler request".into(),
                },
                now,
            )
            .unwrap();

        let config = SchedulerDaemonConfig {
            instance_id: "daemon-api-executor".into(),
            hostname: "fixture".into(),
            adapter: "api".into(),
            provider: "openai".into(),
            account: "paid".into(),
            route_candidates: vec![],
            max_active_claims: 1,
            max_active_per_adapter: 1,
            max_active_per_account: 1,
            poll_interval: std::time::Duration::from_secs(1),
            leader_ttl: std::time::Duration::from_secs(10),
            claim_ttl: std::time::Duration::from_secs(10),
            max_ticks: Some(1),
            execute_fake_claims: false,
            execute_api_claims: true,
            paid_api_acknowledgement: Some(PAID_API_DAEMON_ACKNOWLEDGEMENT.into()),
            execute_api_patches: false,
            api_patch_acknowledgement: None,
        };
        let shutdown = AtomicBool::new(false);
        let mut transport = SchedulerApiTransport {
            sends: 0,
            uncertain: false,
        };
        let mut instant = now;
        let summary = garnish
            .run_scheduler_daemon_with(
                &config,
                &shutdown,
                Some(&mut transport),
                || {
                    let current = instant;
                    instant += Duration::seconds(1);
                    current
                },
                |_| {},
            )
            .unwrap();

        assert_eq!(transport.sends, 1);
        assert_eq!(summary.claims_created, 1);
        assert_eq!(summary.runs_completed, 1);
        assert!(summary.released_task_ids.is_empty());
        assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Review);
        let runs = garnish.db.run_records_for_task(&task.id).unwrap();
        assert_eq!(runs.len(), 2);
        let implementer = runs.iter().find(|run| run.adapter == "api").unwrap();
        assert!(runs.iter().any(|run| run.role == "verifier"));
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(
                data_dir
                    .join("runs")
                    .join(&implementer.id)
                    .join("manifest.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["sandbox"]["backend"], "host-direct-api");
        assert_eq!(manifest["sandbox"]["secure_container"], false);
        assert_eq!(manifest["sandbox"]["host_home_mounted"], true);
        assert_eq!(
            manifest["sandbox"]["writable_mounts"],
            serde_json::json!([])
        );
        let reservations = garnish.api_reservations(Some("fixture")).unwrap();
        assert_eq!(reservations.len(), 1);
        assert_eq!(reservations[0].status, "settled");
        assert_eq!(garnish.api_spend(Some("fixture")).unwrap().len(), 1);
        assert!(
            Command::new("git")
                .args(["diff", "--quiet", "HEAD", "--", "README.md"])
                .current_dir(&source)
                .status()
                .unwrap()
                .success()
        );
        assert!(!source.join("result.txt").exists());

        let mut uncertain_task = new_task;
        uncertain_task.title = "Retain uncertain API dispatch".into();
        uncertain_task.goal = "Prove an uncertain dispatch cannot replay".into();
        let uncertain_task = garnish.add_task(&uncertain_task).unwrap();
        garnish
            .set_task_route_pin(
                &uncertain_task.id,
                "api",
                "openai",
                "paid",
                "explicit uncertain API fixture",
            )
            .unwrap();
        let uncertain_at = now + Duration::seconds(10);
        garnish
            .configure_api_request_plan_at(
                &NewApiRequestPlan {
                    task_id: uncertain_task.id.clone(),
                    provider: "openai".into(),
                    account: "paid".into(),
                    enabled: true,
                    model: "model-fixture".into(),
                    role: "implementer".into(),
                    max_input_tokens: 10_000,
                    max_output_tokens: 50,
                    max_retries: 0,
                    stream: false,
                    reason: "exact uncertain scheduler request".into(),
                },
                uncertain_at,
            )
            .unwrap();
        let mut uncertain_config = config.clone();
        uncertain_config.instance_id = "daemon-api-uncertain".into();
        let mut uncertain_transport = SchedulerApiTransport {
            sends: 0,
            uncertain: true,
        };
        let mut uncertain_instant = uncertain_at;
        let error = garnish
            .run_scheduler_daemon_with(
                &uncertain_config,
                &shutdown,
                Some(&mut uncertain_transport),
                || {
                    let current = uncertain_instant;
                    uncertain_instant += Duration::seconds(1);
                    current
                },
                |_| {},
            )
            .unwrap_err();
        assert!(error.to_string().contains("api.transport_uncertain"));
        assert!(
            !error
                .to_string()
                .contains("scheduler-transport-error-canary")
        );
        assert_eq!(uncertain_transport.sends, 1);
        assert_eq!(
            garnish.task(&uncertain_task.id).unwrap().status,
            TaskStatus::Failed
        );
        let uncertain_reservation = garnish
            .api_reservations(Some("fixture"))
            .unwrap()
            .into_iter()
            .find(|reservation| reservation.task_id == uncertain_task.id)
            .unwrap();
        assert_eq!(uncertain_reservation.status, "dispatched");

        drop(garnish);
        let mut garnish = Garnish::open(&data_dir).unwrap();
        let mut restart_config = config;
        restart_config.instance_id = "daemon-api-restart".into();
        let mut restart_transport = SchedulerApiTransport {
            sends: 0,
            uncertain: false,
        };
        let mut restart_instant = uncertain_at + Duration::seconds(10);
        let restart = garnish
            .run_scheduler_daemon_with(
                &restart_config,
                &shutdown,
                Some(&mut restart_transport),
                || {
                    let current = restart_instant;
                    restart_instant += Duration::seconds(1);
                    current
                },
                |_| {},
            )
            .unwrap();
        assert_eq!(restart.claims_created, 0);
        assert_eq!(restart_transport.sends, 0);
        assert_eq!(
            garnish
                .api_reservations(Some("fixture"))
                .unwrap()
                .into_iter()
                .find(|reservation| reservation.task_id == uncertain_task.id)
                .unwrap()
                .status,
            "dispatched"
        );

        let mut artifacts = Vec::new();
        collect_file_bytes(&data_dir, &mut artifacts);
        for canary in [
            "scheduler-api-secret-canary",
            "scheduler-provider-request-canary",
            "scheduler-response-id-canary",
            "scheduler-output-canary",
            "scheduler-transport-error-canary",
        ] {
            assert!(
                artifacts
                    .iter()
                    .all(|artifact| !String::from_utf8_lossy(artifact).contains(canary)),
                "sensitive API material entered a durable artifact: {canary}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn daemon_applies_exactly_scoped_api_patches_for_both_providers_with_fake_transports() {
        use crate::api_providers::ApiTransportResponse;
        use std::{io::Write, os::unix::fs::OpenOptionsExt};

        struct PatchTransport {
            provider: &'static str,
            patch_path: &'static str,
            sends: usize,
        }

        impl ApiTransport for PatchTransport {
            fn send(&mut self, request: &PreparedApiRequest) -> Result<ApiTransportResponse> {
                self.sends += 1;
                request.with_sensitive_parts(|endpoint, _, _, _, secret, body| {
                    assert_eq!(secret, b"patch-api-secret-canary");
                    assert_eq!(
                        endpoint,
                        match self.provider {
                            "openai" => "https://api.openai.com/v1/responses",
                            "anthropic" => "https://api.anthropic.com/v1/messages",
                            _ => unreachable!(),
                        }
                    );
                    let body: serde_json::Value = serde_json::from_slice(body).unwrap();
                    assert_eq!(body["tools"][0]["name"], API_PATCH_TOOL);
                });
                let patch = format!(
                    "diff --git a/{0} b/{0}\nnew file mode 100644\n--- /dev/null\n+++ b/{0}\n@@ -0,0 +1 @@\n+done\n",
                    self.patch_path
                );
                let body = match self.provider {
                    "openai" => serde_json::json!({
                        "id": "response_patch_fixture",
                        "object": "response",
                        "status": "completed",
                        "model": "model-fixture",
                        "output": [{
                            "type": "function_call",
                            "status": "completed",
                            "call_id": "call_patch_fixture",
                            "name": API_PATCH_TOOL,
                            "arguments": serde_json::to_string(&serde_json::json!({"patch": patch})).unwrap()
                        }],
                        "usage": {"input_tokens": 8, "output_tokens": 2, "total_tokens": 10}
                    }),
                    "anthropic" => serde_json::json!({
                        "id": "message_patch_fixture",
                        "type": "message",
                        "role": "assistant",
                        "model": "model-fixture",
                        "content": [{
                            "type": "tool_use",
                            "id": "tool_patch_fixture",
                            "name": API_PATCH_TOOL,
                            "input": {"patch": patch}
                        }],
                        "stop_reason": "tool_use",
                        "stop_sequence": null,
                        "usage": {"input_tokens": 8, "output_tokens": 2}
                    }),
                    _ => unreachable!(),
                };
                ApiTransportResponse::new(
                    200,
                    "request_patch_fixture".into(),
                    serde_json::to_vec(&body).unwrap(),
                    false,
                )
            }
        }

        for provider in ["openai", "anthropic"] {
            let dir = tempdir().unwrap();
            let source = dir.path().join("source");
            fixture_repo(&source);
            let secret_path = dir.path().join("provider-key");
            let mut secret_file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&secret_path)
                .unwrap();
            writeln!(secret_file, "patch-api-secret-canary").unwrap();
            drop(secret_file);

            let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 7, 21, 12, 0, 0).unwrap();
            let data_dir = dir.path().join("data");
            let mut garnish = Garnish::open(&data_dir).unwrap();
            match provider {
                "openai" => garnish.policy.openai_api_enabled = true,
                "anthropic" => garnish.policy.anthropic_api_enabled = true,
                _ => unreachable!(),
            }
            let project = garnish.add_project("fixture", "Fixture", &source).unwrap();
            let mut new_task = task(project.id.clone());
            new_task.required_capabilities = vec![API_PATCH_CAPABILITY.into()];
            new_task.fake_write_path = None;
            new_task.fake_write_content = None;
            let task = garnish.add_task(&new_task).unwrap();
            garnish
                .configure_api_budget(&NewApiBudget {
                    project_id: project.id.clone(),
                    provider: provider.into(),
                    account: "paid".into(),
                    enabled: true,
                    secret_reference: format!("file:{}", secret_path.display()),
                    currency: Some("USD".into()),
                    currency_limit_micros: Some(1_000_000),
                    token_limit: Some(100_000),
                    request_limit: Some(1),
                    period_start: now - Duration::minutes(1),
                    period_end: now + Duration::days(1),
                    allowed_models: vec!["model-fixture".into()],
                    allowed_tools: vec![API_PATCH_TOOL.into()],
                    allowed_roles: vec!["implementer".into()],
                    max_output_tokens: 1_000,
                    max_retries: 0,
                    max_concurrent_requests: 1,
                    reason: "explicit fixture patch budget".into(),
                })
                .unwrap();
            garnish
                .configure_api_model_price(&NewApiModelPrice {
                    provider: provider.into(),
                    account: "paid".into(),
                    model: "model-fixture".into(),
                    currency: "USD".into(),
                    input_micros_per_million: 1_000_000,
                    cached_input_micros_per_million: 500_000,
                    cache_creation_input_micros_per_million: 1_500_000,
                    output_micros_per_million: 2_000_000,
                    effective_from: now - Duration::minutes(1),
                    effective_to: Some(now + Duration::days(1)),
                    source: "fixture price evidence".into(),
                    reason: "explicit fixture patch price".into(),
                })
                .unwrap();
            garnish
                .set_task_route_pin(&task.id, "api", provider, "paid", "explicit API patch")
                .unwrap();
            garnish
                .configure_api_request_plan_at(
                    &NewApiRequestPlan {
                        task_id: task.id.clone(),
                        provider: provider.into(),
                        account: "paid".into(),
                        enabled: true,
                        model: "model-fixture".into(),
                        role: "implementer".into(),
                        max_input_tokens: 10_000,
                        max_output_tokens: 500,
                        max_retries: 0,
                        stream: false,
                        reason: "exact API patch request".into(),
                    },
                    now,
                )
                .unwrap();
            let config = SchedulerDaemonConfig {
                instance_id: format!("daemon-{provider}-patch"),
                hostname: "fixture".into(),
                adapter: "api".into(),
                provider: provider.into(),
                account: "paid".into(),
                route_candidates: vec![],
                max_active_claims: 1,
                max_active_per_adapter: 1,
                max_active_per_account: 1,
                poll_interval: std::time::Duration::from_secs(1),
                leader_ttl: std::time::Duration::from_secs(10),
                claim_ttl: std::time::Duration::from_secs(10),
                max_ticks: Some(1),
                execute_fake_claims: false,
                execute_api_claims: true,
                paid_api_acknowledgement: Some(PAID_API_DAEMON_ACKNOWLEDGEMENT.into()),
                execute_api_patches: true,
                api_patch_acknowledgement: Some(API_PATCH_DAEMON_ACKNOWLEDGEMENT.into()),
            };
            let mut transport = PatchTransport {
                provider,
                patch_path: "result.txt",
                sends: 0,
            };
            let mut instant = now;
            let summary = garnish
                .run_scheduler_daemon_with(
                    &config,
                    &AtomicBool::new(false),
                    Some(&mut transport),
                    || {
                        let current = instant;
                        instant += Duration::seconds(1);
                        current
                    },
                    |_| {},
                )
                .unwrap();

            assert_eq!(transport.sends, 1);
            assert_eq!(summary.runs_completed, 1);
            assert!(!garnish.api_patch_execution_enabled);
            assert_eq!(garnish.task(&task.id).unwrap().status, TaskStatus::Review);
            let result = data_dir
                .join("worktrees/fixture")
                .join(&task.id)
                .join("result.txt");
            assert_eq!(fs::read_to_string(result).unwrap(), "done\n");
            assert!(!source.join("result.txt").exists());
            let implementer = garnish
                .db
                .run_records_for_task(&task.id)
                .unwrap()
                .into_iter()
                .find(|run| run.adapter == "api")
                .unwrap();
            let manifest: serde_json::Value = serde_json::from_slice(
                &fs::read(
                    data_dir
                        .join("runs")
                        .join(implementer.id)
                        .join("manifest.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(manifest["sandbox"]["backend"], "host-direct-api");
            assert_eq!(manifest["sandbox"]["secure_container"], false);
            assert_eq!(
                manifest["sandbox"]["writable_mounts"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                garnish.api_reservations(Some("fixture")).unwrap()[0].status,
                "settled"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn exact_scope_gate_rejects_an_out_of_scope_patch_in_the_isolated_worktree() {
        // The dual-provider fixture above proves the accepted path. This assertion exercises the
        // final exact-scope gate directly after a patch has been safely confined to a worktree.
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fixture_repo(&source);
        let worktree =
            git::create_task_worktree(&source, &dir.path().join("worktree"), "01OUTOFSCOPE")
                .unwrap();
        let patch = b"diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-fixture\n+changed\n";
        git::apply_untrusted_patch(Path::new(&worktree.path), patch).unwrap();
        let changed = git::changed_files(Path::new(&worktree.path)).unwrap();
        let allowed = BTreeSet::from(["result.txt".to_owned()]);
        let error = validate_applied_patch_paths(Path::new(&worktree.path), &changed, &allowed)
            .unwrap_err();
        assert!(error.to_string().contains("api.patch_scope_denied"));
        assert_eq!(
            fs::read_to_string(source.join("README.md")).unwrap(),
            "fixture\n"
        );
    }

    #[test]
    fn paid_api_daemon_requires_exact_runtime_acknowledgement_before_claiming() {
        let dir = tempdir().unwrap();
        let mut garnish = Garnish::open(dir.path().join("data")).unwrap();
        let config = SchedulerDaemonConfig {
            instance_id: "daemon-api-denied".into(),
            hostname: "fixture".into(),
            adapter: "api".into(),
            provider: "openai".into(),
            account: "paid".into(),
            route_candidates: vec![],
            max_active_claims: 1,
            max_active_per_adapter: 1,
            max_active_per_account: 1,
            poll_interval: std::time::Duration::from_secs(1),
            leader_ttl: std::time::Duration::from_secs(10),
            claim_ttl: std::time::Duration::from_secs(10),
            max_ticks: Some(1),
            execute_fake_claims: false,
            execute_api_claims: true,
            paid_api_acknowledgement: Some("NOT_ACCEPTED".into()),
            execute_api_patches: false,
            api_patch_acknowledgement: None,
        };
        let shutdown = AtomicBool::new(false);
        let error = garnish
            .run_scheduler_daemon_with(&config, &shutdown, None, Utc::now, |_| unreachable!())
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("api.execution_acknowledgement_required")
        );
        assert!(garnish.scheduler_wakes().unwrap().is_empty());
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
