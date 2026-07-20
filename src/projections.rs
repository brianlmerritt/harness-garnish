use crate::domain::{Project, Task, TaskStatus};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Projector {
    root: PathBuf,
}

impl Projector {
    pub fn new(project: &Project) -> Self {
        Self {
            root: Path::new(&project.root_path).join(".harness-garnish"),
        }
    }

    pub fn initialize(&self, project: &Project) -> Result<()> {
        fs::create_dir_all(self.root.join("agents"))?;
        fs::create_dir_all(self.root.join("runs"))?;
        write_projection(
            &self.root.join("PROJECT.md"),
            &format!(
                "# {}\n\n<!-- harness-garnish:project-id={} schema=1 -->\n\n- Slug: `{}`\n- Root: `{}`\n\n## Purpose\n\nProject purpose is curated through `garnish project` commands.\n",
                project.title, project.id, project.slug, project.root_path
            ),
        )?;
        let memory = self.root.join("MEMORY.md");
        if !memory.exists() {
            write_projection(
                &memory,
                &format!(
                    "# Project memory\n\n<!-- harness-garnish:project-id={} schema=1 -->\n\nCurated durable facts only. Do not store secrets or private chain-of-thought.\n",
                    project.id
                ),
            )?;
        }
        write_projection(
            &self.root.join("DECISIONS.md"),
            "# Decisions\n\nNo project ADRs registered.\n",
        )?;
        write_projection(
            &self.root.join("TASKS.md"),
            "# Tasks\n\nNo tasks registered.\n",
        )?;
        write_projection(
            &self.root.join("HANDOFF.md"),
            "# Current handoff\n\nNo resumable run.\n",
        )?;
        write_projection(
            &self.root.join("agents/AGENTS.md"),
            "# Generated agent context\n\nRead `../PROJECT.md`, `../MEMORY.md`, `../TASKS.md`, and `../HANDOFF.md`. Canonical transactional state is maintained by Harness Garnish. Do not edit policy or claim verification.\n",
        )?;
        write_projection(
            &self.root.join("agents/CLAUDE.md"),
            "# Generated Claude context\n\nRead `../PROJECT.md`, `../MEMORY.md`, `../TASKS.md`, and `../HANDOFF.md`. Canonical transactional state is maintained by Harness Garnish. Do not edit policy or claim verification.\n",
        )?;
        Ok(())
    }

    pub fn tasks(&self, project: &Project, tasks: &[Task]) -> Result<()> {
        let mut content = format!(
            "# Tasks\n\n<!-- harness-garnish:project-id={} schema=1 generated=true -->\n\n",
            project.id
        );
        for status in [
            TaskStatus::Ready,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Blocked,
            TaskStatus::Review,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            let matching: Vec<_> = tasks.iter().filter(|task| task.status == status).collect();
            if matching.is_empty() {
                continue;
            }
            content.push_str(&format!("## {status}\n\n"));
            for task in matching {
                content.push_str(&format!(
                    "- `{}` — {} (priority {}, days {}, checkpoint {}s)\n",
                    task.id, task.title, task.priority, task.day_affinity, task.checkpoint_seconds
                ));
            }
            content.push('\n');
        }
        write_projection(&self.root.join("TASKS.md"), &content)
    }

    pub fn handoff(&self, run_handoff: &Path, task: &Task) -> Result<()> {
        let content = format!(
            "# Current handoff\n\n<!-- harness-garnish:task-id={} schema=1 generated=true -->\n\n- Task: {}\n- Evidence: `{}`\n- Next safe action: review the verification evidence and patch.\n",
            task.id,
            task.title,
            run_handoff.display(),
        );
        write_projection(&self.root.join("HANDOFF.md"), &content)
    }

    pub fn link_run(&self, run_id: &str, evidence_dir: &Path) -> Result<()> {
        let link = self.root.join("runs").join(run_id);
        if link.exists() {
            return Ok(());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(evidence_dir, &link)?;
        #[cfg(not(unix))]
        fs::write(
            &link.with_extension("txt"),
            evidence_dir.to_string_lossy().as_bytes(),
        )?;
        Ok(())
    }
}

fn write_projection(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let digest_path = path.with_extension("sha256");
    if path.exists() && digest_path.exists() {
        let current = fs::read(path)?;
        let current_digest = hex::encode(Sha256::digest(&current));
        let expected_digest = fs::read_to_string(&digest_path)?.trim().to_owned();
        if current_digest != expected_digest {
            anyhow::bail!(
                "projection conflict at {}; file content no longer matches its generated digest",
                path.display()
            );
        }
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)
        .with_context(|| format!("writing projection {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("activating projection {}", path.display()))?;
    let digest = hex::encode(Sha256::digest(content.as_bytes()));
    fs::write(digest_path, format!("{digest}\n"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    #[test]
    fn stale_human_edit_is_reported_without_overwrite() {
        let dir = tempdir().unwrap();
        let project = Project {
            id: "p1".into(),
            slug: "fixture".into(),
            title: "Fixture".into(),
            root_path: dir.path().to_string_lossy().into_owned(),
            scheduler_paused: false,
            scheduler_pause_reason: None,
            created_at: Utc::now(),
        };
        let projector = Projector::new(&project);
        projector.initialize(&project).unwrap();
        let tasks = dir.path().join(".harness-garnish/TASKS.md");
        fs::write(&tasks, "human edit\n").unwrap();
        let error = projector.tasks(&project, &[]).unwrap_err().to_string();
        assert!(error.contains("projection conflict"));
        assert_eq!(fs::read_to_string(tasks).unwrap(), "human edit\n");
    }
}
