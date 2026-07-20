use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeFreshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterHealth {
    Healthy,
    Missing,
    Unsupported,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateIdentity {
    pub adapter: String,
    pub provider: String,
    pub account: String,
}

impl CandidateIdentity {
    fn key(&self) -> (&str, &str, &str) {
        (&self.adapter, &self.provider, &self.account)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingCandidateInput {
    pub identity: CandidateIdentity,
    pub freshness: ProbeFreshness,
    pub health: AdapterHealth,
    pub capabilities: Vec<String>,
    pub remaining_percent: Option<f64>,
    pub reserve_percent: f64,
    pub historical_success_percent: Option<f64>,
    pub continuity: bool,
    pub preference: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRequest {
    pub required_capabilities: Vec<String>,
    pub forecast_percent: f64,
    pub pin: Option<CandidateIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreComponents {
    pub quota_margin: f64,
    pub reliability: f64,
    pub continuity_bonus: f64,
    pub preference: f64,
    pub total: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateEvaluation {
    pub identity: CandidateIdentity,
    pub allowed: bool,
    pub reason_code: String,
    pub reason: String,
    pub required_headroom_percent: f64,
    pub score: Option<ScoreComponents>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingSelection {
    pub selected: Option<CandidateIdentity>,
    pub allowed: bool,
    pub reason_code: String,
    pub evaluations: Vec<CandidateEvaluation>,
}

pub fn select_candidate(
    request: &RoutingRequest,
    candidates: &[RoutingCandidateInput],
) -> Result<RoutingSelection> {
    validate_request(request)?;
    for candidate in candidates {
        validate_candidate(candidate)?;
    }

    let mut evaluations = candidates
        .iter()
        .map(|candidate| evaluate_candidate(request, candidate))
        .collect::<Vec<_>>();
    evaluations.sort_by(|left, right| {
        right
            .allowed
            .cmp(&left.allowed)
            .then_with(|| {
                let left_score = left
                    .score
                    .as_ref()
                    .map_or(f64::NEG_INFINITY, |score| score.total);
                let right_score = right
                    .score
                    .as_ref()
                    .map_or(f64::NEG_INFINITY, |score| score.total);
                right_score.total_cmp(&left_score)
            })
            .then_with(|| left.identity.key().cmp(&right.identity.key()))
    });
    let selected = evaluations
        .first()
        .filter(|evaluation| evaluation.allowed)
        .map(|evaluation| evaluation.identity.clone());
    let allowed = selected.is_some();
    let reason_code = if allowed {
        "route.allowed"
    } else if let Some(pin) = request.pin.as_ref() {
        evaluations
            .iter()
            .find(|evaluation| &evaluation.identity == pin)
            .map_or("manual_pin.unavailable", |evaluation| {
                evaluation.reason_code.as_str()
            })
    } else {
        "route.no_candidate"
    };
    Ok(RoutingSelection {
        selected,
        allowed,
        reason_code: reason_code.into(),
        evaluations,
    })
}

fn evaluate_candidate(
    request: &RoutingRequest,
    candidate: &RoutingCandidateInput,
) -> CandidateEvaluation {
    let required_headroom = candidate.reserve_percent + request.forecast_percent;
    let denied = |reason_code: &str, reason: String| CandidateEvaluation {
        identity: candidate.identity.clone(),
        allowed: false,
        reason_code: reason_code.into(),
        reason,
        required_headroom_percent: required_headroom,
        score: None,
    };

    if request
        .pin
        .as_ref()
        .is_some_and(|pin| pin != &candidate.identity)
    {
        return denied(
            "manual_pin.mismatch",
            "candidate does not match the task's exact manual pin".into(),
        );
    }
    match candidate.freshness {
        ProbeFreshness::Unknown => {
            return denied(
                "capability.unknown",
                "candidate has no capability probe evidence".into(),
            );
        }
        ProbeFreshness::Stale => {
            return denied(
                "capability.stale",
                "candidate capability probe evidence has expired".into(),
            );
        }
        ProbeFreshness::Fresh => {}
    }
    match candidate.health {
        AdapterHealth::Healthy => {}
        AdapterHealth::Missing => {
            return denied("adapter.missing", "candidate executable is missing".into());
        }
        AdapterHealth::Unsupported => {
            return denied(
                "adapter.unsupported",
                "candidate version is outside the supported range".into(),
            );
        }
        AdapterHealth::Unhealthy => {
            return denied("adapter.unhealthy", "candidate health check failed".into());
        }
        AdapterHealth::Unknown => {
            return denied("adapter.unknown", "candidate health is unknown".into());
        }
    }
    let missing = request
        .required_capabilities
        .iter()
        .filter(|required| !candidate.capabilities.contains(required))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return denied(
            "capability.missing",
            format!(
                "candidate lacks required capabilities: {}",
                missing.join(",")
            ),
        );
    }
    let Some(remaining_percent) = candidate.remaining_percent else {
        return denied(
            "quota.unknown",
            "candidate quota remaining percentage is unknown".into(),
        );
    };
    if remaining_percent < required_headroom {
        return denied(
            "quota.insufficient",
            format!(
                "candidate has {remaining_percent:.1}% remaining but {required_headroom:.1}% is required"
            ),
        );
    }

    let quota_margin = remaining_percent - required_headroom;
    let reliability = candidate.historical_success_percent.unwrap_or(50.0) * 0.25;
    let continuity_bonus = if candidate.continuity { 10.0 } else { 0.0 };
    let total = quota_margin + reliability + continuity_bonus + candidate.preference;
    CandidateEvaluation {
        identity: candidate.identity.clone(),
        allowed: true,
        reason_code: "route.allowed".into(),
        reason: "candidate passed every hard routing gate".into(),
        required_headroom_percent: required_headroom,
        score: Some(ScoreComponents {
            quota_margin,
            reliability,
            continuity_bonus,
            preference: candidate.preference,
            total,
        }),
    }
}

fn validate_request(request: &RoutingRequest) -> Result<()> {
    validate_percent("forecast percent", request.forecast_percent)?;
    if request
        .required_capabilities
        .iter()
        .any(|capability| capability.trim().is_empty())
    {
        bail!("required capabilities must not contain empty values");
    }
    Ok(())
}

fn validate_candidate(candidate: &RoutingCandidateInput) -> Result<()> {
    if candidate.identity.adapter.trim().is_empty()
        || candidate.identity.provider.trim().is_empty()
        || candidate.identity.account.trim().is_empty()
    {
        bail!("candidate adapter, provider, and account are required");
    }
    validate_percent("reserve percent", candidate.reserve_percent)?;
    if let Some(remaining) = candidate.remaining_percent {
        validate_percent("remaining percent", remaining)?;
    }
    if let Some(success) = candidate.historical_success_percent {
        validate_percent("historical success percent", success)?;
    }
    if !candidate.preference.is_finite() {
        bail!("candidate preference must be finite");
    }
    Ok(())
}

fn validate_percent(label: &str, value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        bail!("{label} must be a finite percentage in 0..=100");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(adapter: &str, account: &str) -> CandidateIdentity {
        CandidateIdentity {
            adapter: adapter.into(),
            provider: adapter.into(),
            account: account.into(),
        }
    }

    fn candidate(
        adapter: &str,
        account: &str,
        remaining_percent: Option<f64>,
    ) -> RoutingCandidateInput {
        RoutingCandidateInput {
            identity: identity(adapter, account),
            freshness: ProbeFreshness::Fresh,
            health: AdapterHealth::Healthy,
            capabilities: vec!["agent.headless".into()],
            remaining_percent,
            reserve_percent: 20.0,
            historical_success_percent: Some(80.0),
            continuity: false,
            preference: 0.0,
        }
    }

    fn request() -> RoutingRequest {
        RoutingRequest {
            required_capabilities: vec!["agent.headless".into()],
            forecast_percent: 10.0,
            pin: None,
        }
    }

    #[test]
    fn deterministic_score_and_lexical_tie_break_ignore_input_order() {
        let a = candidate("claude", "primary", Some(80.0));
        let b = candidate("codex", "primary", Some(80.0));
        for candidates in [vec![a.clone(), b.clone()], vec![b, a]] {
            let selection = select_candidate(&request(), &candidates).unwrap();
            assert!(selection.allowed);
            assert_eq!(selection.selected.unwrap().adapter, "claude");
            assert_eq!(selection.evaluations[0].score.as_ref().unwrap().total, 70.0);
            assert_eq!(selection.evaluations[0].reason_code, "route.allowed");
        }
    }

    #[test]
    fn hard_gates_keep_unknown_stale_health_capability_and_quota_distinct() {
        let mut unknown = candidate("unknown", "one", Some(90.0));
        unknown.freshness = ProbeFreshness::Unknown;
        let mut stale = candidate("stale", "one", Some(90.0));
        stale.freshness = ProbeFreshness::Stale;
        let mut unsupported = candidate("unsupported", "one", Some(90.0));
        unsupported.health = AdapterHealth::Unsupported;
        let mut missing_capability = candidate("missing-cap", "one", Some(90.0));
        missing_capability.capabilities.clear();
        let unknown_quota = candidate("unknown-quota", "one", None);
        let insufficient = candidate("insufficient", "one", Some(29.9));

        let selection = select_candidate(
            &request(),
            &[
                unknown,
                stale,
                unsupported,
                missing_capability,
                unknown_quota,
                insufficient,
            ],
        )
        .unwrap();
        assert!(!selection.allowed);
        let codes = selection
            .evaluations
            .iter()
            .map(|evaluation| evaluation.reason_code.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            codes,
            std::collections::BTreeSet::from([
                "adapter.unsupported",
                "capability.missing",
                "capability.stale",
                "capability.unknown",
                "quota.insufficient",
                "quota.unknown",
            ])
        );
    }

    #[test]
    fn exact_manual_pin_never_bypasses_hard_gates() {
        let codex = candidate("codex", "primary", Some(90.0));
        let claude = candidate("claude", "primary", Some(25.0));
        let mut pinned_request = request();
        pinned_request.pin = Some(claude.identity.clone());
        let selection = select_candidate(&pinned_request, &[codex, claude]).unwrap();
        assert!(!selection.allowed);
        assert_eq!(selection.reason_code, "quota.insufficient");
        let pinned = selection
            .evaluations
            .iter()
            .find(|evaluation| evaluation.identity.adapter == "claude")
            .unwrap();
        assert_eq!(pinned.reason_code, "quota.insufficient");
        let unpinned = selection
            .evaluations
            .iter()
            .find(|evaluation| evaluation.identity.adapter == "codex")
            .unwrap();
        assert_eq!(unpinned.reason_code, "manual_pin.mismatch");

        pinned_request.pin = Some(identity("antigravity", "primary"));
        let unavailable = select_candidate(&pinned_request, &[]).unwrap();
        assert_eq!(unavailable.reason_code, "manual_pin.unavailable");
    }

    #[test]
    fn invalid_numeric_inputs_fail_closed() {
        let mut invalid = candidate("codex", "primary", Some(90.0));
        invalid.preference = f64::NAN;
        assert!(select_candidate(&request(), &[invalid]).is_err());
        let mut invalid_request = request();
        invalid_request.forecast_percent = 101.0;
        assert!(select_candidate(&invalid_request, &[]).is_err());
    }
}
