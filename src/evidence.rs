use crate::{
    adapters::SandboxAttestation,
    domain::{RouteDecision, Task},
};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub argv: Vec<String>,
    pub started_at: String,
    pub ended_at: String,
    pub exit_code: i32,
    pub passed: bool,
    pub sandbox: SandboxAttestation,
    pub worktree: String,
    pub stdout_sha256: String,
    pub stderr_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    pub schema_version: u32,
    pub run_id: String,
    pub task_id: String,
    pub project_id: String,
    pub adapter: String,
    pub base_commit: String,
    pub worktree: String,
    pub branch: String,
    pub policy_hash: String,
    pub route_decision_id: String,
    pub created_at: String,
    pub sandbox: SandboxAttestation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub schema_version: u32,
    pub task_id: String,
    pub run_id: String,
    pub goal: String,
    pub acceptance: Vec<String>,
    pub base_commit: String,
    pub head_commit: String,
    pub worktree: String,
    pub changed_files: Vec<String>,
    pub commands: Vec<VerificationEvidence>,
    pub decisions: Vec<String>,
    pub assumptions: Vec<String>,
    pub blocker: Option<String>,
    pub artifacts: Vec<String>,
    pub next_safe_action: String,
    pub unverified_facts: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct RunEvidence {
    pub directory: PathBuf,
    pub manifest_path: PathBuf,
    pub events_path: PathBuf,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub patch_path: PathBuf,
    pub verification_path: PathBuf,
    pub handoff_json_path: PathBuf,
    pub summary_path: PathBuf,
}

impl RunEvidence {
    pub fn create(data_dir: &Path, run_id: &str) -> Result<Self> {
        let directory = data_dir.join("runs").join(run_id);
        fs::create_dir_all(&directory)
            .with_context(|| format!("creating run evidence directory {}", directory.display()))?;
        Ok(Self {
            manifest_path: directory.join("manifest.json"),
            events_path: directory.join("events.jsonl"),
            stdout_path: directory.join("stdout.log"),
            stderr_path: directory.join("stderr.log"),
            patch_path: directory.join("changes.patch"),
            verification_path: directory.join("verification.json"),
            handoff_json_path: directory.join("handoff.json"),
            summary_path: directory.join("summary.md"),
            directory,
        })
    }

    pub fn write_manifest(&self, manifest: &RunManifest) -> Result<()> {
        write_json(&self.manifest_path, manifest)
    }

    pub fn write_route(&self, route: &RouteDecision) -> Result<PathBuf> {
        let path = self.directory.join("route.json");
        write_json(&path, route)?;
        Ok(path)
    }

    pub fn write_events(&self, events: &[serde_json::Value]) -> Result<()> {
        let mut content = Vec::new();
        for event in events {
            serde_json::to_writer(&mut content, event)?;
            content.push(b'\n');
        }
        fs::write(&self.events_path, content)?;
        Ok(())
    }

    pub fn write_process_output(&self, stdout: &[u8], stderr: &[u8]) -> Result<()> {
        fs::write(&self.stdout_path, bounded(stdout, 2 * 1024 * 1024))?;
        fs::write(&self.stderr_path, bounded(stderr, 2 * 1024 * 1024))?;
        Ok(())
    }

    pub fn write_patch(&self, patch: &[u8]) -> Result<()> {
        fs::write(&self.patch_path, bounded(patch, 10 * 1024 * 1024))?;
        Ok(())
    }

    pub fn write_verification(&self, verification: &VerificationEvidence) -> Result<()> {
        write_json(&self.verification_path, verification)
    }

    pub fn write_handoff(&self, handoff: &Handoff) -> Result<()> {
        write_json(&self.handoff_json_path, handoff)
    }

    pub fn write_summary(
        &self,
        task: &Task,
        adapter: &str,
        changed_files: &[String],
        verification: &VerificationEvidence,
    ) -> Result<()> {
        let changed = if changed_files.is_empty() {
            "- None\n".to_owned()
        } else {
            changed_files
                .iter()
                .map(|path| format!("- `{path}`\n"))
                .collect()
        };
        let content = format!(
            "# Run summary\n\n- Task: `{}` — {}\n- Adapter: `{adapter}`\n- Verification: {} (exit {})\n\n## Changed files\n\n{}\n## Integration\n\nNo push, merge, PR, or source-checkout promotion was performed.\n",
            task.id,
            task.title,
            if verification.passed {
                "passed"
            } else {
                "failed"
            },
            verification.exit_code,
            changed,
        );
        fs::write(&self.summary_path, content)?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn verification_evidence(
    argv: Vec<String>,
    started_at: String,
    ended_at: String,
    exit_code: i32,
    stdout: &[u8],
    stderr: &[u8],
    sandbox: SandboxAttestation,
    worktree: String,
) -> VerificationEvidence {
    VerificationEvidence {
        argv,
        started_at,
        ended_at,
        exit_code,
        passed: exit_code == 0,
        sandbox,
        worktree,
        stdout_sha256: hex::encode(Sha256::digest(stdout)),
        stderr_sha256: hex::encode(Sha256::digest(stderr)),
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut content = serde_json::to_vec_pretty(value)?;
    content.push(b'\n');
    fs::write(path, content)?;
    Ok(())
}

fn bounded(bytes: &[u8], limit: usize) -> Vec<u8> {
    if bytes.len() <= limit {
        return bytes.to_vec();
    }
    let mut result = bytes[..limit].to_vec();
    result.extend_from_slice(
        format!(
            "\n[truncated {} bytes at {}]\n",
            bytes.len() - limit,
            Utc::now()
        )
        .as_bytes(),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_bounded() {
        let value = vec![b'x'; 20];
        let bounded = bounded(&value, 10);
        assert!(bounded.starts_with(&[b'x'; 10]));
        assert!(String::from_utf8_lossy(&bounded).contains("truncated 10 bytes"));
    }

    #[test]
    fn handoff_has_no_thought_process_field() {
        let handoff = Handoff {
            schema_version: 1,
            task_id: "t".into(),
            run_id: "r".into(),
            goal: "g".into(),
            acceptance: vec!["a".into()],
            base_commit: "b".into(),
            head_commit: "h".into(),
            worktree: "/w".into(),
            changed_files: vec![],
            commands: vec![],
            decisions: vec![],
            assumptions: vec![],
            blocker: None,
            artifacts: vec![],
            next_safe_action: "verify".into(),
            unverified_facts: vec![],
            created_at: Utc::now().to_rfc3339(),
        };
        let json = serde_json::to_value(handoff).unwrap();
        assert!(json.get("thought_process").is_none());
        assert!(json.get("chain_of_thought").is_none());
    }
}
