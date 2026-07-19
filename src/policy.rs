use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub require_secure_container_for_writes: bool,
    pub reserve_percent: f64,
    pub unknown_quota_unattended: bool,
    pub max_checkpoint_seconds: u64,
    pub allow_local_commit: bool,
    pub allow_branch_changes: bool,
    pub allow_remote_git: bool,
    pub openai_api_enabled: bool,
    pub anthropic_api_enabled: bool,
}

impl Default for EffectivePolicy {
    fn default() -> Self {
        Self {
            require_secure_container_for_writes: true,
            reserve_percent: 20.0,
            unknown_quota_unattended: false,
            max_checkpoint_seconds: 300,
            allow_local_commit: true,
            allow_branch_changes: true,
            allow_remote_git: false,
            openai_api_enabled: false,
            anthropic_api_enabled: false,
        }
    }
}

impl EffectivePolicy {
    pub fn for_garnish_repository() -> Self {
        Self {
            allow_local_commit: false,
            allow_branch_changes: false,
            allow_remote_git: false,
            ..Self::default()
        }
    }

    pub fn hash(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("policy serializes");
        hex::encode(Sha256::digest(bytes))
    }

    pub fn authorize(&self, effect_class: u8, secure_container: bool) -> PolicyDecision {
        match effect_class {
            0 => PolicyDecision::Allow,
            1 if !self.require_secure_container_for_writes || secure_container => {
                PolicyDecision::Allow
            }
            1 => PolicyDecision::Deny("class 1 writes require an attested secure container".into()),
            2 | 3 => PolicyDecision::RequireApproval,
            _ => PolicyDecision::Deny("unknown effect class".into()),
        }
    }

    pub fn api_allowed(&self, provider: &str) -> bool {
        match provider {
            "openai" => self.openai_api_enabled,
            "anthropic" => self.anthropic_api_enabled,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    RequireApproval,
    Deny(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_one_requires_attestation() {
        let policy = EffectivePolicy::default();
        assert!(matches!(
            policy.authorize(1, false),
            PolicyDecision::Deny(_)
        ));
        assert_eq!(policy.authorize(1, true), PolicyDecision::Allow);
    }

    #[test]
    fn api_is_default_deny() {
        let policy = EffectivePolicy::default();
        assert!(!policy.api_allowed("openai"));
        assert!(!policy.api_allowed("anthropic"));
    }

    #[test]
    fn repository_policy_denies_git_mutation() {
        let policy = EffectivePolicy::for_garnish_repository();
        assert!(!policy.allow_branch_changes);
        assert!(!policy.allow_local_commit);
        assert!(!policy.allow_remote_git);
    }
}
