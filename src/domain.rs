use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DayKind {
    #[serde(rename = "W")]
    Work,
    #[serde(rename = "O")]
    Off,
}

impl fmt::Display for DayKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Work => "W",
            Self::Off => "O",
        })
    }
}

impl FromStr for DayKind {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_uppercase().as_str() {
            "W" => Ok(Self::Work),
            "O" => Ok(Self::Off),
            _ => Err(DomainError::InvalidSchedule(
                "day kind must be W or O".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DayAffinity {
    #[serde(rename = "W")]
    Work,
    #[serde(rename = "O")]
    Off,
    #[serde(rename = "B")]
    Both,
}

impl fmt::Display for DayAffinity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Work => "W",
            Self::Off => "O",
            Self::Both => "B",
        })
    }
}

impl FromStr for DayAffinity {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_uppercase().as_str() {
            "W" => Ok(Self::Work),
            "O" => Ok(Self::Off),
            "B" => Ok(Self::Both),
            _ => Err(DomainError::InvalidSchedule(
                "day affinity must be W, O, or B".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarProfile {
    pub id: String,
    pub slug: String,
    pub timezone: String,
    pub weekly_pattern: String,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarException {
    pub profile_id: String,
    pub local_date: chrono::NaiveDate,
    pub day_kind: DayKind,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEvaluation {
    pub profile_id: String,
    pub profile_version: i64,
    pub timezone: String,
    pub evaluated_at: DateTime<Utc>,
    pub local_date: chrono::NaiveDate,
    pub day_kind: DayKind,
    pub day_source: String,
    pub affinity: DayAffinity,
    pub eligible: bool,
    pub reason_code: String,
    pub next_eligible_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Draft,
    Ready,
    Leased,
    Planning,
    AwaitingApproval,
    Running,
    Verifying,
    Review,
    Completed,
    Paused,
    Blocked,
    Failed,
    Cancelled,
    Superseded,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Superseded)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        use TaskStatus::*;
        matches!(
            (self, next),
            (Draft, Ready)
                | (Draft, Cancelled)
                | (Draft, Superseded)
                | (Ready, Leased)
                | (Ready, Paused)
                | (Ready, Cancelled)
                | (Ready, Superseded)
                | (Leased, Planning)
                | (Leased, Paused)
                | (Leased, Failed)
                | (Planning, AwaitingApproval)
                | (Planning, Running)
                | (Planning, Paused)
                | (Planning, Blocked)
                | (Planning, Failed)
                | (AwaitingApproval, Running)
                | (AwaitingApproval, Paused)
                | (AwaitingApproval, Cancelled)
                | (Running, Verifying)
                | (Running, Paused)
                | (Running, Blocked)
                | (Running, Failed)
                | (Running, Cancelled)
                | (Verifying, Review)
                | (Verifying, Failed)
                | (Review, Completed)
                | (Review, Ready)
                | (Review, Cancelled)
                | (Paused, Ready)
                | (Paused, Cancelled)
                | (Blocked, Ready)
                | (Blocked, Cancelled)
                | (Failed, Ready)
                | (Failed, Cancelled)
        )
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| fmt::Error)?;
        f.write_str(value.as_str().ok_or(fmt::Error)?)
    }
}

impl FromStr for TaskStatus {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(value.to_owned()))
            .map_err(|_| DomainError::InvalidTaskStatus(value.to_owned()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub root_path: String,
    pub scheduler_paused: bool,
    pub scheduler_pause_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectLink {
    pub parent_project_id: String,
    pub child_project_id: String,
    pub relationship: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilityProbe {
    pub id: String,
    pub adapter: String,
    pub executable: Option<String>,
    pub version: Option<String>,
    pub health: String,
    pub capabilities: Vec<String>,
    pub failure: Option<String>,
    pub probed_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilityStatus {
    pub adapter: String,
    pub freshness: String,
    pub health: String,
    pub probe: Option<AgentCapabilityProbe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub goal: String,
    pub rationale: String,
    pub scope: Vec<String>,
    pub non_scope: Vec<String>,
    pub acceptance: Vec<String>,
    pub verification_argv: Vec<String>,
    pub priority: i64,
    pub risk_class: u8,
    pub estimated_seconds: u64,
    pub uncertainty_percent: u8,
    pub checkpoint_seconds: u64,
    pub day_affinity: DayAffinity,
    pub deadline_at: Option<DateTime<Utc>>,
    pub required_capabilities: Vec<String>,
    pub pinned_adapter: Option<String>,
    pub pinned_provider: Option<String>,
    pub pinned_account: Option<String>,
    pub fake_write_path: Option<String>,
    pub fake_write_content: Option<String>,
    pub status: TaskStatus,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewTask {
    pub project_id: String,
    pub title: String,
    pub goal: String,
    pub rationale: String,
    pub scope: Vec<String>,
    pub non_scope: Vec<String>,
    pub acceptance: Vec<String>,
    pub verification_argv: Vec<String>,
    pub dependencies: Vec<String>,
    pub priority: i64,
    pub risk_class: u8,
    pub estimated_seconds: u64,
    pub uncertainty_percent: u8,
    pub checkpoint_seconds: u64,
    pub day_affinity: DayAffinity,
    pub deadline_at: Option<DateTime<Utc>>,
    pub required_capabilities: Vec<String>,
    pub pinned_adapter: Option<String>,
    pub pinned_provider: Option<String>,
    pub pinned_account: Option<String>,
    pub fake_write_path: Option<String>,
    pub fake_write_content: Option<String>,
}

impl NewTask {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.title.trim().is_empty() {
            return Err(DomainError::InvalidTask("title is required".into()));
        }
        if self.goal.trim().is_empty() {
            return Err(DomainError::InvalidTask("goal is required".into()));
        }
        if self.acceptance.is_empty() || self.acceptance.iter().any(|v| v.trim().is_empty()) {
            return Err(DomainError::InvalidTask(
                "at least one non-empty acceptance criterion is required".into(),
            ));
        }
        if self.verification_argv.is_empty() {
            return Err(DomainError::InvalidTask(
                "verification argv is required".into(),
            ));
        }
        if self.risk_class > 3 {
            return Err(DomainError::InvalidTask("risk class must be 0..=3".into()));
        }
        if self.estimated_seconds == 0 {
            return Err(DomainError::InvalidTask(
                "estimated seconds must be greater than zero".into(),
            ));
        }
        if self.checkpoint_seconds == 0 || self.checkpoint_seconds > 300 {
            return Err(DomainError::InvalidTask(
                "checkpoint seconds must be in 1..=300".into(),
            ));
        }
        if self
            .required_capabilities
            .iter()
            .any(|value| value.trim().is_empty() || value.chars().any(char::is_whitespace))
        {
            return Err(DomainError::InvalidTask(
                "required capabilities must be non-empty names without whitespace".into(),
            ));
        }
        let pin_values = [
            self.pinned_adapter.as_deref(),
            self.pinned_provider.as_deref(),
            self.pinned_account.as_deref(),
        ];
        if pin_values.iter().filter(|value| value.is_some()).count() != 0
            && pin_values.iter().filter(|value| value.is_some()).count() != pin_values.len()
        {
            return Err(DomainError::InvalidTask(
                "manual pin requires adapter, provider, and account together".into(),
            ));
        }
        if pin_values
            .into_iter()
            .flatten()
            .any(|value| value.trim().is_empty() || value.chars().any(char::is_whitespace))
        {
            return Err(DomainError::InvalidTask(
                "manual pin values must be non-empty and contain no whitespace".into(),
            ));
        }
        match (&self.fake_write_path, &self.fake_write_content) {
            (Some(path), Some(_)) if !path.trim().is_empty() => {}
            (None, None) => {}
            _ => {
                return Err(DomainError::InvalidTask(
                    "fake write path and content must be supplied together".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSurface {
    pub id: String,
    pub provider: String,
    pub account: String,
    pub surface: String,
    pub observed_remaining_percent: Option<f64>,
    pub effective_remaining_percent: Option<f64>,
    pub reserve_percent: f64,
    pub reset_at: Option<DateTime<Utc>>,
    pub source: String,
    pub unknown_reason: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
    pub confidence: String,
    pub collector_contract: Option<String>,
    pub provider_version: Option<String>,
    pub payload_sha256: Option<String>,
    pub override_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaReservation {
    pub id: String,
    pub surface_id: String,
    pub task_id: String,
    pub claim_id: String,
    pub run_id: Option<String>,
    pub reserved_percent: f64,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
    pub release_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaCollectionAttempt {
    pub id: String,
    pub provider: String,
    pub account: String,
    pub collector_contract: String,
    pub status: String,
    pub detail: String,
    pub attempted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaUsageSample {
    pub id: String,
    pub evidence_id: String,
    pub adapter: String,
    pub provider: String,
    pub account: String,
    pub surface: String,
    pub estimated_seconds: u64,
    pub consumed_percent: f64,
    pub source: String,
    pub confidence: String,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiBudget {
    pub id: String,
    pub project_id: String,
    pub provider: String,
    pub account: String,
    pub enabled: bool,
    pub secret_reference: String,
    pub currency: Option<String>,
    pub currency_limit_micros: Option<u64>,
    pub token_limit: Option<u64>,
    pub request_limit: Option<u64>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub allowed_models: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub allowed_roles: Vec<String>,
    pub max_output_tokens: u64,
    pub max_retries: u32,
    pub max_concurrent_requests: u32,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub supersedes_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewApiBudget {
    pub project_id: String,
    pub provider: String,
    pub account: String,
    pub enabled: bool,
    pub secret_reference: String,
    pub currency: Option<String>,
    pub currency_limit_micros: Option<u64>,
    pub token_limit: Option<u64>,
    pub request_limit: Option<u64>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub allowed_models: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub allowed_roles: Vec<String>,
    pub max_output_tokens: u64,
    pub max_retries: u32,
    pub max_concurrent_requests: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiBudgetReservation {
    pub id: String,
    pub budget_id: String,
    pub project_id: String,
    pub task_id: String,
    pub provider: String,
    pub account: String,
    pub model: String,
    pub role: String,
    pub request_digest: String,
    pub reserved_currency_micros: u64,
    pub reserved_input_tokens: u64,
    pub reserved_output_tokens: u64,
    pub reserved_requests: u32,
    pub per_attempt_input_tokens: u64,
    pub per_attempt_output_tokens: u64,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub dispatch_claimed_at: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub release_reason: Option<String>,
    pub claim_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiDispatchAttempt {
    pub id: String,
    pub reservation_id: String,
    pub attempt_number: u32,
    pub status: String,
    pub failure_kind: Option<String>,
    pub retryable: Option<bool>,
    pub response_status: Option<u16>,
    pub provider_request_id_hash: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ApiReservationRequest {
    pub project_id: String,
    pub task_id: String,
    pub provider: String,
    pub account: String,
    pub model: String,
    pub role: String,
    pub request_digest: String,
    pub reserved_currency_micros: u64,
    pub reserved_input_tokens: u64,
    pub reserved_output_tokens: u64,
    pub reserved_attempts: u32,
    pub now: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ApiClaimReservationRequest {
    pub model: String,
    pub role: String,
    pub request_digest: String,
    pub reserved_currency_micros: u64,
    pub reserved_input_tokens: u64,
    pub reserved_output_tokens: u64,
    pub reserved_attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequestPlan {
    pub id: String,
    pub task_id: String,
    pub task_version: i64,
    pub provider: String,
    pub account: String,
    pub enabled: bool,
    pub model: String,
    pub role: String,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_retries: u32,
    pub stream: bool,
    pub template_version: String,
    pub request_digest: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub supersedes_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewApiRequestPlan {
    pub task_id: String,
    pub provider: String,
    pub account: String,
    pub enabled: bool,
    pub model: String,
    pub role: String,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_retries: u32,
    pub stream: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ApiSettlement {
    pub reservation_id: String,
    pub provider_request_id_hash: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    pub cost_micros: u64,
    pub currency: Option<String>,
    pub pricing_evidence_id: Option<String>,
    pub source: String,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiSpend {
    pub id: String,
    pub budget_id: String,
    pub reservation_id: String,
    pub provider_request_id_hash: String,
    pub model: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    pub cost_micros: u64,
    pub currency: Option<String>,
    pub pricing_evidence_id: Option<String>,
    pub source: String,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiModelPrice {
    pub id: String,
    pub provider: String,
    pub account: String,
    pub model: String,
    pub currency: String,
    pub input_micros_per_million: u64,
    pub cached_input_micros_per_million: u64,
    pub cache_creation_input_micros_per_million: u64,
    pub output_micros_per_million: u64,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    pub source: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub supersedes_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewApiModelPrice {
    pub provider: String,
    pub account: String,
    pub model: String,
    pub currency: String,
    pub input_micros_per_million: u64,
    pub cached_input_micros_per_million: u64,
    pub cache_creation_input_micros_per_million: u64,
    pub output_micros_per_million: u64,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    pub source: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageForecast {
    pub adapter: String,
    pub provider: String,
    pub account: String,
    pub estimated_seconds: u64,
    pub uncertainty_percent: u8,
    pub forecast_percent: f64,
    pub source: String,
    pub sample_count: usize,
    pub percentile: Option<u8>,
    pub lookback_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub task_id: String,
    pub effect_class: u8,
    pub action: serde_json::Value,
    pub decision: String,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub decided_by: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub id: String,
    pub task_id: String,
    pub selected_adapter: Option<String>,
    pub selected_provider: Option<String>,
    pub selected_account: Option<String>,
    pub allowed: bool,
    pub reason_code: String,
    pub reason: String,
    pub required_headroom_percent: f64,
    pub quota: Vec<QuotaSurface>,
    pub candidates: Vec<RouteCandidate>,
    pub next_wake_at: Option<DateTime<Utc>>,
    pub schedule: Option<ScheduleEvaluation>,
    pub policy_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteCandidate {
    pub adapter: String,
    pub provider: String,
    pub account: String,
    pub allowed: bool,
    pub reason_code: String,
    pub filter_reason: String,
    pub forecast_percent: f64,
    pub forecast_source: String,
    pub forecast_sample_count: usize,
    pub minimum_effective_remaining_percent: Option<f64>,
    pub score: Option<f64>,
    pub score_components: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTarget {
    pub adapter: String,
    pub provider: String,
    pub account: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerPreview {
    pub evaluated_at: DateTime<Utc>,
    pub adapter: String,
    pub provider: String,
    pub account: String,
    pub decisions: Vec<RouteDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerLeader {
    pub instance_id: String,
    pub generation: i64,
    pub acquired_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerClaim {
    pub id: String,
    pub task_id: String,
    pub instance_id: String,
    pub task_version: i64,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub resource_keys: Vec<String>,
    pub route_decision_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerApiClaim {
    pub claim: SchedulerClaim,
    pub reservation: ApiBudgetReservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedRunStart {
    pub claim_id: String,
    pub task_id: String,
    pub run_id: String,
    pub route_decision_id: String,
    pub action_key: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    ProcessFailed,
    TimedOut,
    Cancelled,
    Signalled,
    AdapterTransient,
    AdapterPermanent,
    Infrastructure,
    Sandbox,
    Verification,
    Policy,
    Quota,
    Unknown,
}

impl FailureCategory {
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::ProcessFailed
                | Self::TimedOut
                | Self::Signalled
                | Self::AdapterTransient
                | Self::Infrastructure
        )
    }
}

impl fmt::Display for FailureCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| fmt::Error)?;
        formatter.write_str(value.as_str().ok_or(fmt::Error)?)
    }
}

impl FromStr for FailureCategory {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(value.to_owned()))
            .map_err(|_| DomainError::InvalidSupervision(format!("failure category: {value}")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointAction {
    Continue,
    ShortenCheckpoint,
    Pause,
    Cancel,
}

impl fmt::Display for CheckpointAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| fmt::Error)?;
        formatter.write_str(value.as_str().ok_or(fmt::Error)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCheckpoint {
    pub id: String,
    pub run_id: String,
    pub sequence: i64,
    pub evaluated_at: DateTime<Utc>,
    pub action: CheckpointAction,
    pub reason_code: String,
    pub next_checkpoint_at: Option<DateTime<Utc>>,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryState {
    pub task_id: String,
    pub retry_limit: u32,
    pub retries_used: u32,
    pub retry_not_before: Option<DateTime<Utc>>,
    pub last_failure_category: Option<FailureCategory>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPlan {
    pub task_id: String,
    pub run_id: String,
    pub scheduled: bool,
    pub reason_code: String,
    pub retry_number: u32,
    pub retry_at: Option<DateTime<Utc>>,
    pub delay_seconds: Option<u64>,
    pub failure_category: FailureCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreaker {
    pub adapter: String,
    pub provider: String,
    pub account: String,
    pub state: String,
    pub consecutive_failures: u32,
    pub last_failure_category: Option<FailureCategory>,
    pub opened_at: Option<DateTime<Utc>>,
    pub next_probe_at: Option<DateTime<Utc>>,
    pub probe_claimed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlState {
    pub pause_new_work: bool,
    pub emergency_stop: bool,
    pub reason: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalNotification {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergencyStopResult {
    pub control: ControlState,
    pub cancellation_requested_run_ids: Vec<String>,
    pub released_task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub path: String,
    pub schema_version: i64,
    pub size_bytes: u64,
    pub sha256: String,
    pub integrity: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerWake {
    pub task_id: String,
    pub reason_code: String,
    pub wake_at: Option<DateTime<Utc>>,
    pub detail: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerTick {
    pub evaluated_at: DateTime<Utc>,
    pub instance_id: String,
    pub leader_generation: i64,
    pub claims: Vec<SchedulerClaim>,
    pub decisions: Vec<RouteDecision>,
    pub active_claims: usize,
    pub capacity: usize,
}

#[derive(Debug, Clone)]
pub struct SchedulerDaemonConfig {
    pub instance_id: String,
    pub hostname: String,
    pub adapter: String,
    pub provider: String,
    pub account: String,
    pub route_candidates: Vec<RouteTarget>,
    pub max_active_claims: usize,
    pub max_active_per_adapter: usize,
    pub max_active_per_account: usize,
    pub poll_interval: std::time::Duration,
    pub leader_ttl: std::time::Duration,
    pub claim_ttl: std::time::Duration,
    pub max_ticks: Option<usize>,
    pub execute_fake_claims: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerDaemonSummary {
    pub instance_id: String,
    pub leader_generation: i64,
    pub started_at: DateTime<Utc>,
    pub stopped_at: DateTime<Utc>,
    pub ticks: usize,
    pub claims_created: usize,
    pub claims_renewed: usize,
    pub runs_completed: usize,
    pub scheduler_claims_recovered: usize,
    pub run_leases_recovered: usize,
    pub released_task_ids: Vec<String>,
    pub shutdown_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub task_id: String,
    pub status: String,
    pub adapter: String,
    pub worktree: String,
    pub branch: String,
    pub base_commit: String,
    pub patch_path: String,
    pub manifest_path: String,
    pub verification_path: String,
    pub handoff_path: String,
    pub route_decision_id: String,
    pub verifier_run_id: String,
    pub verifier_adapter: String,
    pub verifier_route_decision_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub task_id: String,
    pub role: String,
    pub adapter: String,
    pub parent_run_id: Option<String>,
    pub route_decision_id: String,
    pub worktree_path: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid task: {0}")]
    InvalidTask(String),
    #[error("invalid task status: {0}")]
    InvalidTaskStatus(String),
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),
    #[error("illegal task transition: {from} -> {to}")]
    IllegalTransition { from: TaskStatus, to: TaskStatus },
    #[error("dependency cycle detected")]
    DependencyCycle,
    #[error("invalid supervision value: {0}")]
    InvalidSupervision(String),
}

#[derive(Debug, Error)]
pub enum SchedulerClaimRejection {
    #[error("scheduler global concurrency limit reached ({limit})")]
    GlobalCapacity { limit: usize },
    #[error("scheduler adapter concurrency limit reached ({limit})")]
    AdapterCapacity { limit: usize },
    #[error("scheduler account concurrency limit reached ({limit})")]
    AccountCapacity { limit: usize },
    #[error("scheduler resource lock is unavailable: {kind}:{key}")]
    ResourceLocked { kind: String, key: String },
    #[error("quota evidence is unavailable for {provider}:{account}")]
    QuotaUnavailable { provider: String, account: String },
    #[error("quota evidence is stale for surface {surface}")]
    QuotaStale { surface: String },
    #[error(
        "quota reservation would overcommit {surface}: {remaining:.1}% remaining, {required:.1}% required"
    )]
    QuotaCapacity {
        surface: String,
        remaining: f64,
        required: f64,
    },
}

impl SchedulerClaimRejection {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::GlobalCapacity { .. } => "scheduler.capacity",
            Self::AdapterCapacity { .. } => "scheduler.adapter_capacity",
            Self::AccountCapacity { .. } => "scheduler.account_capacity",
            Self::ResourceLocked { .. } => "scheduler.resource_locked",
            Self::QuotaUnavailable { .. } => "quota.unavailable",
            Self::QuotaStale { .. } => "quota.stale",
            Self::QuotaCapacity { .. } => "quota.reservation_conflict",
        }
    }
}
