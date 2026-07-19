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
    pub override_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub id: String,
    pub task_id: String,
    pub selected_adapter: Option<String>,
    pub allowed: bool,
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
    pub filter_reason: String,
    pub forecast_percent: f64,
    pub minimum_effective_remaining_percent: Option<f64>,
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
}
