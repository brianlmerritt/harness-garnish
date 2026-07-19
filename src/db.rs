use crate::domain::{NewTask, Project, ProjectLink, QuotaSurface, RouteDecision, Task, TaskStatus};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};
use ulid::Ulid;

const SCHEMA_VERSION: i64 = 1;

pub struct Database {
    path: PathBuf,
    conn: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating data directory {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("opening database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut db = Self { path, conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&mut self) -> Result<()> {
        let current: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current > SCHEMA_VERSION {
            bail!("database schema {current} is newer than supported schema {SCHEMA_VERSION}");
        }
        if current == SCHEMA_VERSION {
            return Ok(());
        }
        if current > 0 {
            self.backup_before_migration(current)?;
        }
        let tx = self.conn.transaction()?;
        if current < 1 {
            tx.execute_batch(MIGRATION_1)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn backup_before_migration(&self, version: i64) -> Result<PathBuf> {
        let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
        let backup = self
            .path
            .with_extension(format!("v{version}.{stamp}.backup.db"));
        self.conn
            .execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
        let check = Connection::open(&backup)?;
        let integrity: String = check.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            bail!("pre-migration backup failed integrity check: {integrity}");
        }
        Ok(backup)
    }

    pub fn add_project(&mut self, slug: &str, title: &str, root: &Path) -> Result<Project> {
        let id = Ulid::new().to_string();
        let now = Utc::now();
        let root_path = root
            .canonicalize()
            .with_context(|| format!("canonicalizing project path {}", root.display()))?
            .to_string_lossy()
            .into_owned();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO projects(id, slug, title, root_path, created_at, updated_at, version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, 1)",
            params![id, slug, title, root_path, now.to_rfc3339()],
        )?;
        append_event_tx(
            &tx,
            Some(&id),
            None,
            None,
            "project.registered",
            "user",
            &serde_json::json!({"slug": slug, "root_path": root_path}),
        )?;
        tx.commit()?;
        Ok(Project {
            id,
            slug: slug.to_owned(),
            title: title.to_owned(),
            root_path,
            created_at: now,
        })
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, slug, title, root_path, created_at FROM projects ORDER BY slug")?;
        let rows = stmt.query_map([], map_project)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn project(&self, id_or_slug: &str) -> Result<Project> {
        self.conn
            .query_row(
                "SELECT id, slug, title, root_path, created_at
                 FROM projects WHERE id = ?1 OR slug = ?1",
                [id_or_slug],
                map_project,
            )
            .optional()?
            .ok_or_else(|| anyhow!("project not found: {id_or_slug}"))
    }

    pub fn link_projects(
        &mut self,
        parent: &str,
        child: &str,
        relationship: &str,
    ) -> Result<ProjectLink> {
        if relationship.trim().is_empty() {
            bail!("project relationship is required");
        }
        let parent = self.project(parent)?;
        let child = self.project(child)?;
        if parent.id == child.id {
            bail!("a project cannot link to itself");
        }
        let now = Utc::now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO project_links(parent_project_id, child_project_id, relationship, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![parent.id, child.id, relationship, now.to_rfc3339()],
        )?;
        append_event_tx(
            &tx,
            Some(&parent.id),
            None,
            None,
            "project.linked",
            "user",
            &serde_json::json!({
                "child_project_id": child.id,
                "relationship": relationship,
            }),
        )?;
        tx.commit()?;
        Ok(ProjectLink {
            parent_project_id: parent.id,
            child_project_id: child.id,
            relationship: relationship.into(),
            created_at: now,
        })
    }

    pub fn list_project_links(&self) -> Result<Vec<ProjectLink>> {
        let mut stmt = self.conn.prepare(
            "SELECT parent_project_id, child_project_id, relationship, created_at
             FROM project_links ORDER BY parent_project_id, child_project_id, relationship",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ProjectLink {
                parent_project_id: row.get(0)?,
                child_project_id: row.get(1)?,
                relationship: row.get(2)?,
                created_at: parse_time(row.get(3)?)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn add_task(&mut self, new: &NewTask) -> Result<Task> {
        new.validate()?;
        let project = self.project(&new.project_id)?;
        let estimated_seconds = i64::try_from(new.estimated_seconds)
            .context("estimated seconds exceeds SQLite integer range")?;
        let checkpoint_seconds = i64::try_from(new.checkpoint_seconds)
            .context("checkpoint seconds exceeds SQLite integer range")?;
        let id = Ulid::new().to_string();
        let now = Utc::now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO tasks(
                id, project_id, title, goal, rationale, scope_json, non_scope_json,
                acceptance_json, verification_argv_json, priority, risk_class,
                estimated_seconds, uncertainty_percent, checkpoint_seconds,
                fake_write_path, fake_write_content, status, version, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, 'draft', 1, ?17, ?17
             )",
            params![
                id,
                project.id,
                new.title,
                new.goal,
                new.rationale,
                to_json(&new.scope)?,
                to_json(&new.non_scope)?,
                to_json(&new.acceptance)?,
                to_json(&new.verification_argv)?,
                new.priority,
                new.risk_class,
                estimated_seconds,
                new.uncertainty_percent,
                checkpoint_seconds,
                new.fake_write_path,
                new.fake_write_content,
                now.to_rfc3339(),
            ],
        )?;
        for dependency in &new.dependencies {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
                [dependency],
                |row| row.get(0),
            )?;
            if !exists {
                bail!("dependency task not found: {dependency}");
            }
            tx.execute(
                "INSERT INTO task_dependencies(task_id, depends_on_task_id, created_at)
                 VALUES (?1, ?2, ?3)",
                params![id, dependency, now.to_rfc3339()],
            )?;
        }
        ensure_acyclic_tx(&tx)?;
        append_event_tx(
            &tx,
            Some(&project.id),
            Some(&id),
            None,
            "task.created",
            "user",
            &serde_json::json!({"title": new.title, "dependencies": new.dependencies}),
        )?;
        let dependencies_satisfied: bool = tx.query_row(
            "SELECT NOT EXISTS(
                SELECT 1 FROM task_dependencies d
                JOIN tasks t ON t.id = d.depends_on_task_id
                WHERE d.task_id = ?1 AND t.status != 'completed'
             )",
            [&id],
            |row| row.get(0),
        )?;
        if dependencies_satisfied {
            transition_task_tx(
                &tx,
                &id,
                TaskStatus::Draft,
                TaskStatus::Ready,
                "contract_validated",
            )?;
        } else {
            append_event_tx(
                &tx,
                Some(&project.id),
                Some(&id),
                None,
                "task.waiting_dependencies",
                "control_plane",
                &serde_json::json!({"status": "draft"}),
            )?;
        }
        tx.commit()?;
        self.task(&id)
    }

    pub fn add_dependency(&mut self, task_id: &str, depends_on_task_id: &str) -> Result<Task> {
        let task = self.task(task_id)?;
        self.task(depends_on_task_id)?;
        if !matches!(
            task.status,
            TaskStatus::Draft | TaskStatus::Ready | TaskStatus::Paused
        ) {
            bail!("dependencies can only be edited before a task is leased");
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO task_dependencies(task_id, depends_on_task_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![task_id, depends_on_task_id, Utc::now().to_rfc3339()],
        )?;
        ensure_acyclic_tx(&tx)?;
        if task.status == TaskStatus::Ready {
            transition_task_tx(
                &tx,
                task_id,
                TaskStatus::Ready,
                TaskStatus::Paused,
                "dependency_added",
            )?;
        }
        append_event_tx(
            &tx,
            Some(&task.project_id),
            Some(task_id),
            None,
            "task.dependency_added",
            "user",
            &serde_json::json!({"depends_on_task_id": depends_on_task_id}),
        )?;
        tx.commit()?;
        self.task(task_id)
    }

    pub fn complete_review(&mut self, task_id: &str) -> Result<Vec<Task>> {
        self.task(task_id)?;
        let tx = self.conn.transaction()?;
        transition_task_tx(
            &tx,
            task_id,
            TaskStatus::Review,
            TaskStatus::Completed,
            "user_accepted_review",
        )?;
        let mut stmt = tx.prepare(
            "SELECT DISTINCT t.id, t.status
             FROM tasks t
             JOIN task_dependencies d ON d.task_id = t.id
             WHERE d.depends_on_task_id = ?1 AND t.status IN ('draft', 'paused')
               AND NOT EXISTS (
                   SELECT 1 FROM task_dependencies pending
                   JOIN tasks dependency ON dependency.id = pending.depends_on_task_id
                   WHERE pending.task_id = t.id AND dependency.status != 'completed'
               )",
        )?;
        let candidates = stmt
            .query_map([task_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        let mut promoted_ids = Vec::new();
        for (id, status) in candidates {
            let expected = TaskStatus::from_str(&status)?;
            transition_task_tx(
                &tx,
                &id,
                expected,
                TaskStatus::Ready,
                "dependencies_completed",
            )?;
            promoted_ids.push(id);
        }
        tx.commit()?;
        promoted_ids
            .iter()
            .map(|id| self.task(id))
            .collect::<Result<Vec<_>>>()
    }

    pub fn task(&self, id: &str) -> Result<Task> {
        self.conn
            .query_row(TASK_SELECT_BY_ID, [id], map_task)
            .optional()?
            .ok_or_else(|| anyhow!("task not found: {id}"))
    }

    pub fn list_tasks(&self, project_id: Option<&str>) -> Result<Vec<Task>> {
        let (sql, argument) = if let Some(project_id) = project_id {
            (
                format!(
                    "{} WHERE project_id = ?1 ORDER BY priority DESC, created_at",
                    TASK_SELECT
                ),
                Some(project_id),
            )
        } else {
            (
                format!("{} ORDER BY priority DESC, created_at", TASK_SELECT),
                None,
            )
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let mut result = Vec::new();
        if let Some(argument) = argument {
            let rows = stmt.query_map([argument], map_task)?;
            for row in rows {
                result.push(row?);
            }
        } else {
            let rows = stmt.query_map([], map_task)?;
            for row in rows {
                result.push(row?);
            }
        }
        Ok(result)
    }

    pub fn dependencies_satisfied(&self, task_id: &str) -> Result<bool> {
        let missing: i64 = self.conn.query_row(
            "SELECT COUNT(*)
             FROM task_dependencies d
             JOIN tasks t ON t.id = d.depends_on_task_id
             WHERE d.task_id = ?1 AND t.status != 'completed'",
            [task_id],
            |row| row.get(0),
        )?;
        Ok(missing == 0)
    }

    pub fn transition_task(
        &mut self,
        task_id: &str,
        expected: TaskStatus,
        next: TaskStatus,
        reason: &str,
    ) -> Result<Task> {
        if !expected.can_transition_to(next) {
            bail!("illegal task transition: {expected} -> {next}");
        }
        let tx = self.conn.transaction()?;
        transition_task_tx(&tx, task_id, expected, next, reason)?;
        tx.commit()?;
        self.task(task_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_quota_observation(
        &mut self,
        provider: &str,
        account: &str,
        surface: &str,
        remaining_percent: Option<f64>,
        reserve_percent: f64,
        reset_at: Option<DateTime<Utc>>,
        source: &str,
        unknown_reason: Option<&str>,
    ) -> Result<QuotaSurface> {
        validate_percentage(remaining_percent, "remaining")?;
        validate_percentage(Some(reserve_percent), "reserve")?;
        if remaining_percent.is_none() && unknown_reason.is_none() {
            bail!("unknown quota requires an unknown reason");
        }
        let id = format!("{provider}:{account}:{surface}");
        let now = Utc::now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO quota_surfaces(
                id, provider, account, surface_key, observed_remaining_percent,
                reserve_percent, reset_at, source, unknown_reason, observed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(provider, account, surface_key) DO UPDATE SET
                observed_remaining_percent = excluded.observed_remaining_percent,
                reserve_percent = excluded.reserve_percent,
                reset_at = excluded.reset_at,
                source = excluded.source,
                unknown_reason = excluded.unknown_reason,
                observed_at = excluded.observed_at",
            params![
                id,
                provider,
                account,
                surface,
                remaining_percent,
                reserve_percent,
                reset_at.map(|v| v.to_rfc3339()),
                source,
                unknown_reason,
                now.to_rfc3339(),
            ],
        )?;
        append_event_tx(
            &tx,
            None,
            None,
            None,
            "quota.observed",
            "quota_provider",
            &serde_json::json!({
                "provider": provider,
                "account": account,
                "surface": surface,
                "remaining_percent": remaining_percent,
                "unknown_reason": unknown_reason,
            }),
        )?;
        tx.commit()?;
        self.quota_surface(provider, account, surface)
    }

    pub fn override_quota(
        &mut self,
        provider: &str,
        account: &str,
        surface: &str,
        effective_remaining_percent: f64,
        reason: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<QuotaSurface> {
        validate_percentage(Some(effective_remaining_percent), "effective remaining")?;
        if reason.trim().is_empty() {
            bail!("quota override reason is required");
        }
        let quota = self.quota_surface(provider, account, surface)?;
        let id = Ulid::new().to_string();
        let now = Utc::now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO quota_overrides(
                id, surface_id, effective_remaining_percent, reason, actor,
                created_at, expires_at
             ) VALUES (?1, ?2, ?3, ?4, 'user', ?5, ?6)",
            params![
                id,
                quota.id,
                effective_remaining_percent,
                reason,
                now.to_rfc3339(),
                expires_at.map(|v| v.to_rfc3339()),
            ],
        )?;
        append_event_tx(
            &tx,
            None,
            None,
            None,
            "quota.overridden",
            "user",
            &serde_json::json!({
                "surface_id": quota.id,
                "effective_remaining_percent": effective_remaining_percent,
                "reason": reason,
                "expires_at": expires_at,
            }),
        )?;
        tx.commit()?;
        self.quota_surface(provider, account, surface)
    }

    pub fn quota_surface(
        &self,
        provider: &str,
        account: &str,
        surface: &str,
    ) -> Result<QuotaSurface> {
        self.conn
            .query_row(
                QUOTA_SELECT,
                params![provider, account, surface, Utc::now().to_rfc3339()],
                map_quota,
            )
            .optional()?
            .ok_or_else(|| anyhow!("quota surface not found: {provider}:{account}:{surface}"))
    }

    pub fn list_quota(&self) -> Result<Vec<QuotaSurface>> {
        let sql = format!(
            "{} ORDER BY q.provider, q.account, q.surface_key",
            QUOTA_SELECT_ALL
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([Utc::now().to_rfc3339()], map_quota)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn record_route(&mut self, decision: &RouteDecision) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO route_decisions(
                id, task_id, selected_adapter, allowed, reason,
                required_headroom_percent, quota_json, policy_hash, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                decision.id,
                decision.task_id,
                decision.selected_adapter,
                decision.allowed,
                decision.reason,
                decision.required_headroom_percent,
                to_json(&decision.quota)?,
                decision.policy_hash,
                decision.created_at.to_rfc3339(),
            ],
        )?;
        append_event_tx(
            &tx,
            None,
            Some(&decision.task_id),
            None,
            "route.decided",
            "router",
            decision,
        )?;
        tx.commit()?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_run(
        &mut self,
        run_id: &str,
        task_id: &str,
        adapter: &str,
        route_decision_id: &str,
        worktree: &str,
        branch: &str,
        base_commit: &str,
        lease_expires_at: DateTime<Utc>,
    ) -> Result<()> {
        let now = Utc::now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO runs(
                id, task_id, adapter, route_decision_id, worktree_path, branch,
                base_commit, status, started_at, heartbeat_at, checkpoint_due_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', ?8, ?8, ?9)",
            params![
                run_id,
                task_id,
                adapter,
                route_decision_id,
                worktree,
                branch,
                base_commit,
                now.to_rfc3339(),
                lease_expires_at.to_rfc3339(),
            ],
        )?;
        tx.execute(
            "INSERT INTO leases(id, task_id, run_id, owner, acquired_at, heartbeat_at, expires_at, generation)
             VALUES (?1, ?2, ?3, 'local', ?4, ?4, ?5, 1)",
            params![
                Ulid::new().to_string(),
                task_id,
                run_id,
                now.to_rfc3339(),
                lease_expires_at.to_rfc3339(),
            ],
        )?;
        append_event_tx(
            &tx,
            None,
            Some(task_id),
            Some(run_id),
            "run.started",
            "control_plane",
            &serde_json::json!({"adapter": adapter, "worktree": worktree, "branch": branch}),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn finish_run(
        &mut self,
        run_id: &str,
        status: &str,
        head_commit: Option<&str>,
        exit_code: i32,
    ) -> Result<()> {
        let now = Utc::now();
        let tx = self.conn.transaction()?;
        let task_id: String =
            tx.query_row("SELECT task_id FROM runs WHERE id = ?1", [run_id], |row| {
                row.get(0)
            })?;
        tx.execute(
            "UPDATE runs SET status = ?2, head_commit = ?3, exit_code = ?4, ended_at = ?5
             WHERE id = ?1",
            params![run_id, status, head_commit, exit_code, now.to_rfc3339()],
        )?;
        tx.execute(
            "UPDATE leases SET released_at = ?2 WHERE run_id = ?1 AND released_at IS NULL",
            params![run_id, now.to_rfc3339()],
        )?;
        append_event_tx(
            &tx,
            None,
            Some(&task_id),
            Some(run_id),
            "run.finished",
            "control_plane",
            &serde_json::json!({"status": status, "exit_code": exit_code}),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn append_run_event<T: Serialize>(
        &mut self,
        task_id: &str,
        run_id: &str,
        event_type: &str,
        actor: &str,
        payload: &T,
    ) -> Result<String> {
        let tx = self.conn.transaction()?;
        let id = append_event_tx(
            &tx,
            None,
            Some(task_id),
            Some(run_id),
            event_type,
            actor,
            payload,
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn events_for_run(&self, run_id: &str) -> Result<Vec<serde_json::Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, sequence, event_type, occurred_at, actor, payload_json, previous_digest, digest
             FROM events WHERE run_id = ?1 ORDER BY sequence",
        )?;
        let rows = stmt.query_map([run_id], |row| {
            let payload: String = row.get(5)?;
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "sequence": row.get::<_, i64>(1)?,
                "type": row.get::<_, String>(2)?,
                "occurred_at": row.get::<_, String>(3)?,
                "actor": row.get::<_, String>(4)?,
                "payload": serde_json::from_str::<serde_json::Value>(&payload).unwrap_or_default(),
                "previous_digest": row.get::<_, Option<String>>(6)?,
                "digest": row.get::<_, String>(7)?,
            }))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn recover_expired_leases(&mut self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let tx = self.conn.transaction()?;
        let mut stmt = tx.prepare(
            "SELECT DISTINCT l.task_id, l.run_id
             FROM leases l
             JOIN tasks t ON t.id = l.task_id
             WHERE l.released_at IS NULL AND l.expires_at < ?1
               AND t.status IN ('leased', 'planning', 'awaiting_approval', 'running')",
        )?;
        let rows = stmt.query_map([now.to_rfc3339()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let expired = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (task_id, run_id) in &expired {
            tx.execute(
                "UPDATE tasks SET status = 'paused', version = version + 1, updated_at = ?2 WHERE id = ?1",
                params![task_id, now.to_rfc3339()],
            )?;
            tx.execute(
                "UPDATE runs SET status = 'orphaned', ended_at = ?2 WHERE id = ?1",
                params![run_id, now.to_rfc3339()],
            )?;
            tx.execute(
                "UPDATE leases SET released_at = ?2 WHERE run_id = ?1 AND released_at IS NULL",
                params![run_id, now.to_rfc3339()],
            )?;
            append_event_tx(
                &tx,
                None,
                Some(task_id),
                Some(run_id),
                "lease.expired",
                "recovery",
                &serde_json::json!({"recovered_to": "paused"}),
            )?;
        }
        tx.commit()?;
        Ok(expired.into_iter().map(|(task, _)| task).collect())
    }

    pub fn create_approval(
        &mut self,
        task_id: &str,
        effect_class: u8,
        action: &serde_json::Value,
        expires_at: DateTime<Utc>,
    ) -> Result<String> {
        let id = Ulid::new().to_string();
        let action_json = serde_json::to_string(action)?;
        let digest = hex::encode(Sha256::digest(action_json.as_bytes()));
        let now = Utc::now();
        self.conn.execute(
            "INSERT INTO approvals(
                id, task_id, effect_class, action_digest, action_json, decision,
                requested_at, expires_at, single_use
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7, 1)",
            params![
                id,
                task_id,
                effect_class,
                digest,
                action_json,
                now.to_rfc3339(),
                expires_at.to_rfc3339(),
            ],
        )?;
        Ok(id)
    }

    pub fn decide_approval(&mut self, approval_id: &str, approve: bool) -> Result<()> {
        let decision = if approve { "approved" } else { "denied" };
        let changed = self.conn.execute(
            "UPDATE approvals SET decision = ?2, decided_by = 'user', decided_at = ?3
             WHERE id = ?1 AND decision = 'pending' AND expires_at > ?3",
            params![approval_id, decision, Utc::now().to_rfc3339()],
        )?;
        if changed != 1 {
            bail!("approval is missing, expired, or already decided");
        }
        Ok(())
    }

    pub fn consume_approval(
        &mut self,
        approval_id: &str,
        action: &serde_json::Value,
    ) -> Result<()> {
        let action_json = serde_json::to_string(action)?;
        let digest = hex::encode(Sha256::digest(action_json.as_bytes()));
        let now = Utc::now().to_rfc3339();
        let changed = self.conn.execute(
            "UPDATE approvals SET consumed_at = ?3
             WHERE id = ?1 AND action_digest = ?2 AND decision = 'approved'
               AND consumed_at IS NULL AND expires_at > ?3",
            params![approval_id, digest, now],
        )?;
        if changed != 1 {
            bail!("approval action mismatch, expiry, replay, or denial");
        }
        Ok(())
    }
}

fn validate_percentage(value: Option<f64>, label: &str) -> Result<()> {
    if let Some(value) = value
        && (!value.is_finite() || !(0.0..=100.0).contains(&value))
    {
        bail!("{label} percentage must be in 0..=100");
    }
    Ok(())
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(Into::into)
}

fn parse_time(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                value.len(),
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })
}

fn parse_json<T: serde::de::DeserializeOwned>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            value.len(),
            rusqlite::types::Type::Text,
            Box::new(err),
        )
    })
}

fn map_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        slug: row.get(1)?,
        title: row.get(2)?,
        root_path: row.get(3)?,
        created_at: parse_time(row.get(4)?)?,
    })
}

const TASK_SELECT: &str = "SELECT
    id, project_id, title, goal, rationale, scope_json, non_scope_json,
    acceptance_json, verification_argv_json, priority, risk_class,
    estimated_seconds, uncertainty_percent, checkpoint_seconds,
    fake_write_path, fake_write_content, status, version, created_at, updated_at
    FROM tasks";
const TASK_SELECT_BY_ID: &str = "SELECT
    id, project_id, title, goal, rationale, scope_json, non_scope_json,
    acceptance_json, verification_argv_json, priority, risk_class,
    estimated_seconds, uncertainty_percent, checkpoint_seconds,
    fake_write_path, fake_write_content, status, version, created_at, updated_at
    FROM tasks WHERE id = ?1";

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status: String = row.get(16)?;
    Ok(Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        goal: row.get(3)?,
        rationale: row.get(4)?,
        scope: parse_json(row.get(5)?)?,
        non_scope: parse_json(row.get(6)?)?,
        acceptance: parse_json(row.get(7)?)?,
        verification_argv: parse_json(row.get(8)?)?,
        priority: row.get(9)?,
        risk_class: row.get(10)?,
        estimated_seconds: nonnegative_u64(row, 11)?,
        uncertainty_percent: row.get(12)?,
        checkpoint_seconds: nonnegative_u64(row, 13)?,
        fake_write_path: row.get(14)?,
        fake_write_content: row.get(15)?,
        status: TaskStatus::from_str(&status).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                status.len(),
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })?,
        version: row.get(17)?,
        created_at: parse_time(row.get(18)?)?,
        updated_at: parse_time(row.get(19)?)?,
    })
}

fn nonnegative_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    value.try_into().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

const QUOTA_SELECT: &str = "SELECT
    q.id, q.provider, q.account, q.surface_key, q.observed_remaining_percent,
    COALESCE(o.effective_remaining_percent, q.observed_remaining_percent),
    q.reserve_percent, q.reset_at, q.source, q.unknown_reason, q.observed_at, o.reason
 FROM quota_surfaces q
 LEFT JOIN quota_overrides o ON o.id = (
    SELECT id FROM quota_overrides x
    WHERE x.surface_id = q.id AND (x.expires_at IS NULL OR x.expires_at > ?4)
    ORDER BY x.created_at DESC LIMIT 1
 )
 WHERE q.provider = ?1 AND q.account = ?2 AND q.surface_key = ?3";

const QUOTA_SELECT_ALL: &str = "SELECT
    q.id, q.provider, q.account, q.surface_key, q.observed_remaining_percent,
    COALESCE(o.effective_remaining_percent, q.observed_remaining_percent),
    q.reserve_percent, q.reset_at, q.source, q.unknown_reason, q.observed_at, o.reason
 FROM quota_surfaces q
 LEFT JOIN quota_overrides o ON o.id = (
    SELECT id FROM quota_overrides x
    WHERE x.surface_id = q.id AND (x.expires_at IS NULL OR x.expires_at > ?1)
    ORDER BY x.created_at DESC LIMIT 1
 )";

fn map_quota(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuotaSurface> {
    let reset_at: Option<String> = row.get(7)?;
    Ok(QuotaSurface {
        id: row.get(0)?,
        provider: row.get(1)?,
        account: row.get(2)?,
        surface: row.get(3)?,
        observed_remaining_percent: row.get(4)?,
        effective_remaining_percent: row.get(5)?,
        reserve_percent: row.get(6)?,
        reset_at: reset_at.map(parse_time).transpose()?,
        source: row.get(8)?,
        unknown_reason: row.get(9)?,
        observed_at: parse_time(row.get(10)?)?,
        override_reason: row.get(11)?,
    })
}

fn ensure_acyclic_tx(tx: &Transaction<'_>) -> Result<()> {
    let cycle: bool = tx.query_row(
        "WITH RECURSIVE walk(origin, node) AS (
             SELECT task_id, depends_on_task_id FROM task_dependencies
             UNION ALL
             SELECT walk.origin, d.depends_on_task_id
             FROM walk JOIN task_dependencies d ON d.task_id = walk.node
         )
         SELECT EXISTS(SELECT 1 FROM walk WHERE origin = node)",
        [],
        |row| row.get(0),
    )?;
    if cycle {
        bail!("dependency cycle detected");
    }
    Ok(())
}

fn transition_task_tx(
    tx: &Transaction<'_>,
    task_id: &str,
    expected: TaskStatus,
    next: TaskStatus,
    reason: &str,
) -> Result<()> {
    if !expected.can_transition_to(next) {
        bail!("illegal task transition: {expected} -> {next}");
    }
    let project_id: String = tx.query_row(
        "SELECT project_id FROM tasks WHERE id = ?1",
        [task_id],
        |row| row.get(0),
    )?;
    let changed = tx.execute(
        "UPDATE tasks SET status = ?3, version = version + 1, updated_at = ?4
         WHERE id = ?1 AND status = ?2",
        params![
            task_id,
            expected.to_string(),
            next.to_string(),
            Utc::now().to_rfc3339()
        ],
    )?;
    if changed != 1 {
        bail!("task transition compare-and-swap failed for {task_id}");
    }
    append_event_tx(
        tx,
        Some(&project_id),
        Some(task_id),
        None,
        "task.transitioned",
        "control_plane",
        &serde_json::json!({"from": expected, "to": next, "reason": reason}),
    )?;
    Ok(())
}

fn append_event_tx<T: Serialize>(
    tx: &Transaction<'_>,
    project_id: Option<&str>,
    task_id: Option<&str>,
    run_id: Option<&str>,
    event_type: &str,
    actor: &str,
    payload: &T,
) -> Result<String> {
    let id = Ulid::new().to_string();
    let sequence: i64 = tx.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM events",
        [],
        |row| row.get(0),
    )?;
    let previous_digest: Option<String> = tx
        .query_row(
            "SELECT digest FROM events ORDER BY sequence DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let payload_json = serde_json::to_string(payload)?;
    let occurred_at = Utc::now().to_rfc3339();
    let canonical = serde_json::json!({
        "id": id,
        "sequence": sequence,
        "project_id": project_id,
        "task_id": task_id,
        "run_id": run_id,
        "event_type": event_type,
        "occurred_at": occurred_at,
        "actor": actor,
        "payload": serde_json::from_str::<serde_json::Value>(&payload_json)?,
        "previous_digest": previous_digest,
    });
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(&canonical)?));
    tx.execute(
        "INSERT INTO events(
            id, sequence, project_id, task_id, run_id, event_type, schema_version,
            occurred_at, actor, payload_json, previous_digest, digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            sequence,
            project_id,
            task_id,
            run_id,
            event_type,
            occurred_at,
            actor,
            payload_json,
            previous_digest,
            digest,
        ],
    )?;
    Ok(id)
}

const MIGRATION_1: &str = r#"
CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    root_path TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL
);

CREATE TABLE project_links (
    parent_project_id TEXT NOT NULL REFERENCES projects(id),
    child_project_id TEXT NOT NULL REFERENCES projects(id),
    relationship TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(parent_project_id, child_project_id, relationship),
    CHECK(parent_project_id != child_project_id)
);

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id),
    title TEXT NOT NULL,
    goal TEXT NOT NULL,
    rationale TEXT NOT NULL,
    scope_json TEXT NOT NULL,
    non_scope_json TEXT NOT NULL,
    acceptance_json TEXT NOT NULL,
    verification_argv_json TEXT NOT NULL,
    priority INTEGER NOT NULL,
    risk_class INTEGER NOT NULL CHECK(risk_class BETWEEN 0 AND 3),
    estimated_seconds INTEGER NOT NULL CHECK(estimated_seconds > 0),
    uncertainty_percent INTEGER NOT NULL CHECK(uncertainty_percent BETWEEN 0 AND 100),
    checkpoint_seconds INTEGER NOT NULL CHECK(checkpoint_seconds BETWEEN 1 AND 300),
    fake_write_path TEXT,
    fake_write_content TEXT,
    status TEXT NOT NULL,
    version INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE task_dependencies (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on_task_id TEXT NOT NULL REFERENCES tasks(id),
    created_at TEXT NOT NULL,
    PRIMARY KEY(task_id, depends_on_task_id),
    CHECK(task_id != depends_on_task_id)
);

CREATE TABLE quota_surfaces (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    account TEXT NOT NULL,
    surface_key TEXT NOT NULL,
    observed_remaining_percent REAL,
    reserve_percent REAL NOT NULL,
    reset_at TEXT,
    source TEXT NOT NULL,
    unknown_reason TEXT,
    observed_at TEXT NOT NULL,
    UNIQUE(provider, account, surface_key)
);

CREATE TABLE quota_overrides (
    id TEXT PRIMARY KEY,
    surface_id TEXT NOT NULL REFERENCES quota_surfaces(id),
    effective_remaining_percent REAL NOT NULL,
    reason TEXT NOT NULL,
    actor TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT
);

CREATE TABLE route_decisions (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    selected_adapter TEXT,
    allowed INTEGER NOT NULL,
    reason TEXT NOT NULL,
    required_headroom_percent REAL NOT NULL,
    quota_json TEXT NOT NULL,
    policy_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    adapter TEXT NOT NULL,
    route_decision_id TEXT NOT NULL REFERENCES route_decisions(id),
    worktree_path TEXT NOT NULL,
    branch TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    head_commit TEXT,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    checkpoint_due_at TEXT NOT NULL,
    ended_at TEXT,
    exit_code INTEGER
);

CREATE TABLE leases (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    run_id TEXT NOT NULL REFERENCES runs(id),
    owner TEXT NOT NULL,
    acquired_at TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    generation INTEGER NOT NULL,
    released_at TEXT
);

CREATE TABLE approvals (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    effect_class INTEGER NOT NULL,
    action_digest TEXT NOT NULL,
    action_json TEXT NOT NULL,
    decision TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    decided_by TEXT,
    decided_at TEXT,
    single_use INTEGER NOT NULL,
    consumed_at TEXT
);

CREATE TABLE events (
    id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL UNIQUE,
    project_id TEXT,
    task_id TEXT,
    run_id TEXT,
    event_type TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    occurred_at TEXT NOT NULL,
    actor TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    previous_digest TEXT,
    digest TEXT NOT NULL
);

CREATE INDEX idx_tasks_project_status ON tasks(project_id, status, priority DESC);
CREATE INDEX idx_dependencies_task ON task_dependencies(task_id);
CREATE INDEX idx_events_run_sequence ON events(run_id, sequence);
CREATE INDEX idx_leases_expiry ON leases(expires_at) WHERE released_at IS NULL;
CREATE INDEX idx_quota_override_surface ON quota_overrides(surface_id, created_at DESC);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn database() -> (tempfile::TempDir, Database) {
        let dir = tempdir().unwrap();
        let db = Database::open(dir.path().join("state.db")).unwrap();
        (dir, db)
    }

    fn new_task(project_id: &str, title: &str, dependencies: Vec<String>) -> NewTask {
        NewTask {
            project_id: project_id.into(),
            title: title.into(),
            goal: format!("Complete {title}"),
            rationale: "test".into(),
            scope: vec!["fixture".into()],
            non_scope: vec![],
            acceptance: vec!["done".into()],
            verification_argv: vec!["true".into()],
            dependencies,
            priority: 1,
            risk_class: 1,
            estimated_seconds: 60,
            uncertainty_percent: 10,
            checkpoint_seconds: 60,
            fake_write_path: None,
            fake_write_content: None,
        }
    }

    #[test]
    fn creates_wal_database_and_project() {
        let (dir, mut db) = database();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        assert_eq!(db.project("one").unwrap().id, project.id);
        assert_eq!(db.list_projects().unwrap().len(), 1);
    }

    #[test]
    fn quota_override_preserves_observation() {
        let (_dir, mut db) = database();
        db.set_quota_observation(
            "claude",
            "max",
            "five_hour",
            Some(7.0),
            20.0,
            None,
            "fake",
            None,
        )
        .unwrap();
        let value = db
            .override_quota(
                "claude",
                "max",
                "five_hour",
                80.0,
                "subscription changed",
                None,
            )
            .unwrap();
        assert_eq!(value.observed_remaining_percent, Some(7.0));
        assert_eq!(value.effective_remaining_percent, Some(80.0));
        assert_eq!(
            value.override_reason.as_deref(),
            Some("subscription changed")
        );
    }

    #[test]
    fn approval_is_bound_and_single_use() {
        let (dir, mut db) = database();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let task = db
            .add_task(&NewTask {
                project_id: project.id,
                title: "Task".into(),
                goal: "Goal".into(),
                rationale: "Why".into(),
                scope: vec!["a".into()],
                non_scope: vec![],
                acceptance: vec!["done".into()],
                verification_argv: vec!["true".into()],
                dependencies: vec![],
                priority: 1,
                risk_class: 2,
                estimated_seconds: 60,
                uncertainty_percent: 10,
                checkpoint_seconds: 60,
                fake_write_path: None,
                fake_write_content: None,
            })
            .unwrap();
        let action = serde_json::json!({"kind":"download","target":"example"});
        let approval = db
            .create_approval(
                &task.id,
                2,
                &action,
                Utc::now() + chrono::Duration::minutes(5),
            )
            .unwrap();
        db.decide_approval(&approval, true).unwrap();
        assert!(
            db.consume_approval(&approval, &serde_json::json!({"kind":"other"}))
                .is_err()
        );
        db.consume_approval(&approval, &action).unwrap();
        assert!(db.consume_approval(&approval, &action).is_err());
    }

    #[test]
    fn dependencies_wait_promote_and_cycles_rollback_atomically() {
        let (dir, mut db) = database();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let prerequisite = db
            .add_task(&new_task(&project.id, "prerequisite", vec![]))
            .unwrap();
        let dependent = db
            .add_task(&new_task(
                &project.id,
                "dependent",
                vec![prerequisite.id.clone()],
            ))
            .unwrap();
        assert_eq!(dependent.status, TaskStatus::Draft);
        assert!(db.add_dependency(&prerequisite.id, &dependent.id).is_err());
        assert!(db.dependencies_satisfied(&prerequisite.id).unwrap());

        for (expected, next) in [
            (TaskStatus::Ready, TaskStatus::Leased),
            (TaskStatus::Leased, TaskStatus::Planning),
            (TaskStatus::Planning, TaskStatus::Running),
            (TaskStatus::Running, TaskStatus::Verifying),
            (TaskStatus::Verifying, TaskStatus::Review),
        ] {
            db.transition_task(&prerequisite.id, expected, next, "test")
                .unwrap();
        }
        let promoted = db.complete_review(&prerequisite.id).unwrap();
        assert_eq!(promoted.len(), 1);
        assert_eq!(promoted[0].id, dependent.id);
        assert_eq!(db.task(&dependent.id).unwrap().status, TaskStatus::Ready);
    }

    #[test]
    fn restart_expires_orphan_once_without_duplicate_recovery() {
        let (dir, mut db) = database();
        let database_path = db.path().to_path_buf();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let task = db
            .add_task(&new_task(&project.id, "recover", vec![]))
            .unwrap();
        let decision = RouteDecision {
            id: Ulid::new().to_string(),
            task_id: task.id.clone(),
            selected_adapter: Some("fake".into()),
            allowed: true,
            reason: "fixture".into(),
            required_headroom_percent: 21.0,
            quota: vec![],
            candidates: vec![],
            next_wake_at: None,
            policy_hash: "fixture-policy".into(),
            created_at: Utc::now(),
        };
        db.record_route(&decision).unwrap();
        for (expected, next) in [
            (TaskStatus::Ready, TaskStatus::Leased),
            (TaskStatus::Leased, TaskStatus::Planning),
            (TaskStatus::Planning, TaskStatus::Running),
        ] {
            db.transition_task(&task.id, expected, next, "fixture")
                .unwrap();
        }
        db.create_run(
            "run-orphan",
            &task.id,
            "fake",
            &decision.id,
            "/fixture/worktree",
            "garnish/task-fixture",
            "0123456789abcdef",
            Utc::now() - chrono::Duration::seconds(1),
        )
        .unwrap();
        drop(db);

        let mut reopened = Database::open(database_path).unwrap();
        assert_eq!(
            reopened.recover_expired_leases(Utc::now()).unwrap(),
            vec![task.id.clone()]
        );
        assert_eq!(reopened.task(&task.id).unwrap().status, TaskStatus::Paused);
        assert!(
            reopened
                .recover_expired_leases(Utc::now())
                .unwrap()
                .is_empty()
        );
    }
}
