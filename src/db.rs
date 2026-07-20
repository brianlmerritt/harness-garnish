use crate::{
    domain::{
        BackupRecord, CalendarException, CalendarProfile, CheckpointAction, CircuitBreaker,
        ClaimedRunStart, ControlState, DayKind, EmergencyStopResult, FailureCategory,
        LocalNotification, NewTask, Project, ProjectLink, QuotaSurface, RetryPlan, RetryState,
        RouteDecision, RunCheckpoint, SchedulerClaim, SchedulerLeader, SchedulerWake, Task,
        TaskStatus,
    },
    schedule,
};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    str::FromStr,
};
use ulid::Ulid;

const SCHEMA_VERSION: i64 = 6;

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
        secure_database_file(&path)?;
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

    pub fn schema_version(&self) -> i64 {
        SCHEMA_VERSION
    }

    pub fn control_state(&self) -> Result<ControlState> {
        self.conn
            .query_row(
                "SELECT pause_new_work, emergency_stop, reason, updated_at
                 FROM control_state WHERE singleton = 1",
                [],
                |row| {
                    Ok(ControlState {
                        pause_new_work: row.get(0)?,
                        emergency_stop: row.get(1)?,
                        reason: row.get(2)?,
                        updated_at: parse_time(row.get(3)?)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn set_pause_new_work(
        &mut self,
        paused: bool,
        reason: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<ControlState> {
        if paused && reason.is_none_or(|value| value.trim().is_empty()) {
            bail!("pausing new work requires a reason");
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let emergency: bool = tx.query_row(
            "SELECT emergency_stop FROM control_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        if emergency && !paused {
            bail!("resume requires clearing emergency stop explicitly");
        }
        tx.execute(
            "UPDATE control_state SET pause_new_work = ?1, reason = ?2, updated_at = ?3
             WHERE singleton = 1",
            params![paused, reason, now.to_rfc3339()],
        )?;
        append_event_tx(
            &tx,
            None,
            None,
            None,
            if paused {
                "operations.paused"
            } else {
                "operations.resumed"
            },
            "user",
            &serde_json::json!({"reason": reason}),
        )?;
        tx.commit()?;
        self.control_state()
    }

    pub fn resume_operations(&mut self, reason: &str, now: DateTime<Utc>) -> Result<ControlState> {
        if reason.trim().is_empty() {
            bail!("resuming operations requires a reason");
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE control_state
             SET pause_new_work = 0, emergency_stop = 0, reason = ?1, updated_at = ?2
             WHERE singleton = 1",
            params![reason, now.to_rfc3339()],
        )?;
        append_event_tx(
            &tx,
            None,
            None,
            None,
            "operations.resumed",
            "user",
            &serde_json::json!({"reason": reason, "cleared_emergency_stop": true}),
        )?;
        tx.commit()?;
        self.control_state()
    }

    pub fn emergency_stop(
        &mut self,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<EmergencyStopResult> {
        if reason.trim().is_empty() {
            bail!("emergency stop requires a reason");
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE control_state
             SET pause_new_work = 1, emergency_stop = 1, reason = ?1, updated_at = ?2
             WHERE singleton = 1",
            params![reason, now.to_rfc3339()],
        )?;

        let mut run_stmt = tx.prepare(
            "SELECT r.id, r.task_id FROM runs r
             JOIN tasks t ON t.id = r.task_id
             WHERE r.status = 'running' AND t.status = 'running'
             ORDER BY r.started_at, r.id",
        )?;
        let active_runs = run_stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(run_stmt);
        for (run_id, task_id) in &active_runs {
            tx.execute(
                "UPDATE run_supervision
                 SET cancellation_status = 'requested', cancellation_reason = ?2,
                     cancellation_requested_at = COALESCE(cancellation_requested_at, ?3),
                     requested_action = 'cancel', updated_at = ?3, version = version + 1
                 WHERE run_id = ?1 AND cancellation_status != 'completed'",
                params![run_id, reason, now.to_rfc3339()],
            )?;
            append_event_tx(
                &tx,
                None,
                Some(task_id),
                Some(run_id),
                "run.emergency_stop_requested",
                "user",
                &serde_json::json!({"reason": reason}),
            )?;
        }

        let mut claim_stmt = tx.prepare(
            "SELECT id, task_id FROM scheduler_claims
             WHERE status = 'active' ORDER BY acquired_at, id",
        )?;
        let claims = claim_stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(claim_stmt);
        let mut released_task_ids = Vec::new();
        for (claim_id, task_id) in &claims {
            let status: String =
                tx.query_row("SELECT status FROM tasks WHERE id = ?1", [task_id], |row| {
                    row.get(0)
                })?;
            if status == TaskStatus::Leased.to_string() {
                transition_task_tx(
                    &tx,
                    task_id,
                    TaskStatus::Leased,
                    TaskStatus::Paused,
                    "emergency_stop",
                )?;
                released_task_ids.push(task_id.clone());
            }
            tx.execute(
                "UPDATE scheduler_claims SET status = 'released', released_at = ?2
                 WHERE id = ?1 AND status = 'active'",
                params![claim_id, now.to_rfc3339()],
            )?;
            tx.execute(
                "UPDATE resource_locks SET released_at = ?2
                 WHERE claim_id = ?1 AND released_at IS NULL",
                params![claim_id, now.to_rfc3339()],
            )?;
        }
        append_event_tx(
            &tx,
            None,
            None,
            None,
            "operations.emergency_stop",
            "user",
            &serde_json::json!({
                "reason": reason,
                "active_run_count": active_runs.len(),
                "released_claim_count": claims.len(),
            }),
        )?;
        enqueue_notification_tx(
            &tx,
            "operation",
            "critical",
            None,
            None,
            "Harness Garnish emergency stop",
            reason,
            now,
        )?;
        tx.commit()?;
        Ok(EmergencyStopResult {
            control: self.control_state()?,
            cancellation_requested_run_ids: active_runs
                .into_iter()
                .map(|(run_id, _)| run_id)
                .collect(),
            released_task_ids,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_notification(
        &mut self,
        kind: &str,
        severity: &str,
        task_id: Option<&str>,
        run_id: Option<&str>,
        title: &str,
        body: &str,
        now: DateTime<Utc>,
    ) -> Result<LocalNotification> {
        let tx = self.conn.transaction()?;
        let notification =
            enqueue_notification_tx(&tx, kind, severity, task_id, run_id, title, body, now)?;
        tx.commit()?;
        Ok(notification)
    }

    pub fn local_notifications(
        &self,
        include_acknowledged: bool,
        limit: usize,
    ) -> Result<Vec<LocalNotification>> {
        if limit == 0 || limit > 200 {
            bail!("notification limit must be in 1..=200");
        }
        let sql = if include_acknowledged {
            "SELECT id, kind, severity, task_id, run_id, title, body, created_at, acknowledged_at
             FROM local_notifications ORDER BY created_at DESC, id DESC LIMIT ?1"
        } else {
            "SELECT id, kind, severity, task_id, run_id, title, body, created_at, acknowledged_at
             FROM local_notifications WHERE acknowledged_at IS NULL
             ORDER BY created_at DESC, id DESC LIMIT ?1"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([limit as i64], map_local_notification)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn acknowledge_notification(
        &mut self,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<LocalNotification> {
        let changed = self.conn.execute(
            "UPDATE local_notifications SET acknowledged_at = ?2
             WHERE id = ?1 AND acknowledged_at IS NULL",
            params![id, now.to_rfc3339()],
        )?;
        if changed != 1 {
            bail!("notification is missing or already acknowledged: {id}");
        }
        self.conn
            .query_row(
                "SELECT id, kind, severity, task_id, run_id, title, body, created_at, acknowledged_at
                 FROM local_notifications WHERE id = ?1",
                [id],
                map_local_notification,
            )
            .map_err(Into::into)
    }

    pub fn operational_counts(&self, now: DateTime<Utc>) -> Result<serde_json::Value> {
        let task_counts = self.counts_by_status("tasks")?;
        let run_counts = self.counts_by_status("runs")?;
        let active_claims: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM scheduler_claims WHERE status = 'active' AND expires_at > ?1",
            [now.to_rfc3339()],
            |row| row.get(0),
        )?;
        let active_schedulers: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM scheduler_instances WHERE status = 'active'",
            [],
            |row| row.get(0),
        )?;
        let pending_notifications: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM local_notifications WHERE acknowledged_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(serde_json::json!({
            "evaluated_at": now,
            "control": self.control_state()?,
            "task_counts": task_counts,
            "run_counts": run_counts,
            "active_scheduler_claims": active_claims,
            "active_schedulers": active_schedulers,
            "pending_notifications": pending_notifications,
        }))
    }

    fn counts_by_status(&self, table: &str) -> Result<serde_json::Map<String, serde_json::Value>> {
        let sql = match table {
            "tasks" => "SELECT status, COUNT(*) FROM tasks GROUP BY status ORDER BY status",
            "runs" => "SELECT status, COUNT(*) FROM runs GROUP BY status ORDER BY status",
            _ => bail!("unsupported status-count table"),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut counts = serde_json::Map::new();
        for row in rows {
            let (status, count) = row?;
            counts.insert(status, serde_json::json!(count));
        }
        Ok(counts)
    }

    pub fn backup_to(&self, destination: &Path, now: DateTime<Utc>) -> Result<BackupRecord> {
        if destination.exists() {
            bail!(
                "backup destination already exists: {}",
                destination.display()
            );
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating backup directory {}", parent.display()))?;
        }
        self.conn
            .execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])?;
        secure_database_file(destination)?;
        let check = Connection::open(destination)?;
        let integrity: String = check.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            let _ = fs::remove_file(destination);
            bail!("backup failed integrity check: {integrity}");
        }
        let mut file = fs::File::open(destination)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        Ok(BackupRecord {
            path: destination.to_string_lossy().into_owned(),
            schema_version: SCHEMA_VERSION,
            size_bytes: fs::metadata(destination)?.len(),
            sha256: hex::encode(digest.finalize()),
            integrity,
            created_at: now,
        })
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
        }
        if current < 2 {
            tx.execute_batch(MIGRATION_2)?;
        }
        if current < 3 {
            tx.execute_batch(MIGRATION_3)?;
        }
        if current < 4 {
            tx.execute_batch(MIGRATION_4)?;
        }
        if current < 5 {
            tx.execute_batch(MIGRATION_5)?;
        }
        if current < 6 {
            tx.execute_batch(MIGRATION_6)?;
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
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
        secure_database_file(&backup)?;
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

    pub fn configure_calendar(
        &mut self,
        slug: &str,
        timezone: &str,
        weekly_pattern: &str,
    ) -> Result<CalendarProfile> {
        if slug.is_empty()
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            bail!("calendar slug must contain lowercase ASCII letters, digits, or hyphens");
        }
        schedule::validate_profile(timezone, weekly_pattern)?;
        let existing_id: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM calendar_profiles WHERE slug = ?1",
                [slug],
                |row| row.get(0),
            )
            .optional()?;
        let id = existing_id.unwrap_or_else(|| Ulid::new().to_string());
        let now = Utc::now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO calendar_profiles(
                id, slug, timezone, weekly_pattern, version, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)
             ON CONFLICT(slug) DO UPDATE SET
                timezone = excluded.timezone,
                weekly_pattern = excluded.weekly_pattern,
                version = calendar_profiles.version + 1,
                updated_at = excluded.updated_at",
            params![id, slug, timezone, weekly_pattern, now.to_rfc3339()],
        )?;
        append_event_tx(
            &tx,
            None,
            None,
            None,
            "calendar.configured",
            "user",
            &serde_json::json!({
                "calendar_id": id,
                "slug": slug,
                "timezone": timezone,
                "weekly_pattern": weekly_pattern,
            }),
        )?;
        tx.commit()?;
        self.calendar(&id)
    }

    pub fn calendar(&self, id_or_slug: &str) -> Result<CalendarProfile> {
        self.conn
            .query_row(
                "SELECT id, slug, timezone, weekly_pattern, version, created_at, updated_at
                 FROM calendar_profiles WHERE id = ?1 OR slug = ?1",
                [id_or_slug],
                map_calendar,
            )
            .optional()?
            .ok_or_else(|| anyhow!("calendar not found: {id_or_slug}"))
    }

    pub fn assign_project_calendar(
        &mut self,
        project_id_or_slug: &str,
        calendar_id_or_slug: &str,
    ) -> Result<CalendarProfile> {
        let project = self.project(project_id_or_slug)?;
        let calendar = self.calendar(calendar_id_or_slug)?;
        let changed = self.conn.execute(
            "UPDATE projects SET calendar_profile_id = ?2, version = version + 1,
             updated_at = ?3 WHERE id = ?1",
            params![project.id, calendar.id, Utc::now().to_rfc3339()],
        )?;
        if changed != 1 {
            bail!("project calendar assignment failed");
        }
        Ok(calendar)
    }

    pub fn project_calendar(&self, project_id_or_slug: &str) -> Result<CalendarProfile> {
        let project = self.project(project_id_or_slug)?;
        self.conn
            .query_row(
                "SELECT c.id, c.slug, c.timezone, c.weekly_pattern, c.version,
                    c.created_at, c.updated_at
             FROM projects p
             JOIN calendar_profiles c ON c.id = COALESCE(p.calendar_profile_id, 'default')
             WHERE p.id = ?1",
                [project.id],
                map_calendar,
            )
            .map_err(Into::into)
    }

    pub fn set_calendar_exception(
        &mut self,
        calendar_id_or_slug: &str,
        local_date: chrono::NaiveDate,
        day_kind: DayKind,
        reason: &str,
    ) -> Result<CalendarException> {
        if reason.trim().is_empty() {
            bail!("calendar exception reason is required");
        }
        let calendar = self.calendar(calendar_id_or_slug)?;
        let now = Utc::now();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO calendar_exceptions(profile_id, local_date, day_kind, reason, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(profile_id, local_date) DO UPDATE SET
                day_kind = excluded.day_kind,
                reason = excluded.reason,
                created_at = excluded.created_at",
            params![
                calendar.id,
                local_date.to_string(),
                day_kind.to_string(),
                reason,
                now.to_rfc3339(),
            ],
        )?;
        append_event_tx(
            &tx,
            None,
            None,
            None,
            "calendar.exception_set",
            "user",
            &serde_json::json!({
                "calendar_id": calendar.id,
                "local_date": local_date,
                "day_kind": day_kind,
                "reason": reason,
            }),
        )?;
        tx.commit()?;
        Ok(CalendarException {
            profile_id: calendar.id,
            local_date,
            day_kind,
            reason: reason.into(),
            created_at: now,
        })
    }

    pub fn calendar_exceptions(&self, profile_id: &str) -> Result<Vec<CalendarException>> {
        let mut stmt = self.conn.prepare(
            "SELECT profile_id, local_date, day_kind, reason, created_at
             FROM calendar_exceptions WHERE profile_id = ?1 ORDER BY local_date",
        )?;
        let rows = stmt.query_map([profile_id], |row| {
            let date: String = row.get(1)?;
            let kind: String = row.get(2)?;
            Ok(CalendarException {
                profile_id: row.get(0)?,
                local_date: chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(
                    |error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            date.len(),
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    },
                )?,
                day_kind: DayKind::from_str(&kind).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        kind.len(),
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                reason: row.get(3)?,
                created_at: parse_time(row.get(4)?)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn register_scheduler_instance(
        &mut self,
        instance_id: &str,
        hostname: &str,
        process_id: u32,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if instance_id.trim().is_empty() || hostname.trim().is_empty() {
            bail!("scheduler instance ID and hostname are required");
        }
        self.conn.execute(
            "INSERT INTO scheduler_instances(
                id, hostname, process_id, started_at, heartbeat_at, status
             ) VALUES (?1, ?2, ?3, ?4, ?4, 'active')
             ON CONFLICT(id) DO UPDATE SET
                hostname = excluded.hostname,
                process_id = excluded.process_id,
                heartbeat_at = excluded.heartbeat_at,
                status = 'active'",
            params![
                instance_id,
                hostname,
                i64::from(process_id),
                now.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn acquire_scheduler_leader(
        &mut self,
        instance_id: &str,
        now: DateTime<Utc>,
        ttl: std::time::Duration,
    ) -> Result<SchedulerLeader> {
        let expires_at =
            now + chrono::Duration::from_std(ttl).context("scheduler leader TTL is too large")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let instance_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM scheduler_instances WHERE id = ?1 AND status = 'active')",
            [instance_id],
            |row| row.get(0),
        )?;
        if !instance_exists {
            bail!("scheduler instance is not registered: {instance_id}");
        }
        let current: Option<(String, i64, String)> = tx
            .query_row(
                "SELECT instance_id, generation, expires_at FROM scheduler_leader WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let generation = match current {
            None => 1,
            Some((owner, generation, _)) if owner == instance_id => generation,
            Some((_, generation, expiry)) if parse_time(expiry.clone())? <= now => generation + 1,
            Some((owner, _, expiry)) => {
                bail!("scheduler leader is held by {owner} until {expiry}")
            }
        };
        tx.execute(
            "INSERT INTO scheduler_leader(
                singleton, instance_id, generation, acquired_at, heartbeat_at, expires_at
             ) VALUES (1, ?1, ?2, ?3, ?3, ?4)
             ON CONFLICT(singleton) DO UPDATE SET
                instance_id = excluded.instance_id,
                generation = excluded.generation,
                acquired_at = CASE
                    WHEN scheduler_leader.instance_id = excluded.instance_id
                    THEN scheduler_leader.acquired_at ELSE excluded.acquired_at END,
                heartbeat_at = excluded.heartbeat_at,
                expires_at = excluded.expires_at",
            params![
                instance_id,
                generation,
                now.to_rfc3339(),
                expires_at.to_rfc3339(),
            ],
        )?;
        append_event_tx(
            &tx,
            None,
            None,
            None,
            "scheduler.leader_acquired",
            "scheduler",
            &serde_json::json!({
                "instance_id": instance_id,
                "generation": generation,
                "expires_at": expires_at,
            }),
        )?;
        tx.commit()?;
        Ok(SchedulerLeader {
            instance_id: instance_id.into(),
            generation,
            acquired_at: now,
            heartbeat_at: now,
            expires_at,
        })
    }

    pub fn heartbeat_scheduler_leader(
        &mut self,
        instance_id: &str,
        generation: i64,
        now: DateTime<Utc>,
        ttl: std::time::Duration,
    ) -> Result<SchedulerLeader> {
        let expires_at =
            now + chrono::Duration::from_std(ttl).context("scheduler leader TTL is too large")?;
        let changed = self.conn.execute(
            "UPDATE scheduler_leader SET heartbeat_at = ?3, expires_at = ?4
             WHERE singleton = 1 AND instance_id = ?1 AND generation = ?2 AND expires_at > ?3",
            params![
                instance_id,
                generation,
                now.to_rfc3339(),
                expires_at.to_rfc3339(),
            ],
        )?;
        if changed != 1 {
            bail!("scheduler leadership was lost or expired");
        }
        self.conn.execute(
            "UPDATE scheduler_instances SET heartbeat_at = ?2 WHERE id = ?1",
            params![instance_id, now.to_rfc3339()],
        )?;
        let acquired_at: String = self.conn.query_row(
            "SELECT acquired_at FROM scheduler_leader WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(SchedulerLeader {
            instance_id: instance_id.into(),
            generation,
            acquired_at: parse_time(acquired_at)?,
            heartbeat_at: now,
            expires_at,
        })
    }

    pub fn stop_scheduler_instance(
        &mut self,
        instance_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE scheduler_instances SET status = 'stopped', heartbeat_at = ?2
             WHERE id = ?1 AND status = 'active'",
            params![instance_id, now.to_rfc3339()],
        )?;
        if changed != 1 {
            bail!("scheduler instance is missing or already stopped");
        }
        tx.execute(
            "UPDATE scheduler_leader SET heartbeat_at = ?2, expires_at = ?2
             WHERE singleton = 1 AND instance_id = ?1",
            params![instance_id, now.to_rfc3339()],
        )?;
        let released = release_scheduler_claims_for_instance_tx(&tx, instance_id, now)?;
        append_event_tx(
            &tx,
            None,
            None,
            None,
            "scheduler.stopped",
            "scheduler",
            &serde_json::json!({
                "instance_id": instance_id,
                "released_task_ids": &released,
            }),
        )?;
        tx.commit()?;
        Ok(released)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim_task_for_scheduler(
        &mut self,
        instance_id: &str,
        leader_generation: i64,
        task_id: &str,
        expected_task_version: i64,
        now: DateTime<Utc>,
        ttl: std::time::Duration,
        max_active_claims: usize,
        route_decision_id: Option<&str>,
        resources: &[(String, String)],
    ) -> Result<SchedulerClaim> {
        if max_active_claims == 0 {
            bail!("scheduler concurrency limit must be greater than zero");
        }
        let expires_at =
            now + chrono::Duration::from_std(ttl).context("scheduler claim TTL is too large")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_scheduler_leader_tx(&tx, instance_id, leader_generation, now)?;
        let (pause_new_work, emergency_stop): (bool, bool) = tx.query_row(
            "SELECT pause_new_work, emergency_stop FROM control_state WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if emergency_stop {
            bail!("operations.emergency_stop: new scheduler claims are disabled");
        }
        if pause_new_work {
            bail!("operations.paused: new scheduler claims are disabled");
        }
        expire_scheduler_claims_tx(&tx, now)?;
        let active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM scheduler_claims WHERE status = 'active' AND expires_at > ?1",
            [now.to_rfc3339()],
            |row| row.get(0),
        )?;
        if active >= i64::try_from(max_active_claims)? {
            bail!("scheduler concurrency limit reached ({max_active_claims})");
        }
        let (status, version, project_id): (String, i64, String) = tx
            .query_row(
                "SELECT status, version, project_id FROM tasks WHERE id = ?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("task not found: {task_id}"))?;
        if status != "ready" || version != expected_task_version {
            bail!(
                "task claim compare-and-swap failed: expected ready version {expected_task_version}, found {status} version {version}"
            );
        }
        let dependencies_satisfied: bool = tx.query_row(
            "SELECT NOT EXISTS(
                SELECT 1 FROM task_dependencies d
                JOIN tasks dependency ON dependency.id = d.depends_on_task_id
                WHERE d.task_id = ?1 AND dependency.status != 'completed'
             )",
            [task_id],
            |row| row.get(0),
        )?;
        if !dependencies_satisfied {
            bail!("task dependencies are not complete");
        }
        let claim_id = Ulid::new().to_string();
        tx.execute(
            "INSERT INTO scheduler_claims(
                id, task_id, instance_id, leader_generation, task_version,
                status, acquired_at, expires_at, route_decision_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8)",
            params![
                claim_id,
                task_id,
                instance_id,
                leader_generation,
                expected_task_version,
                now.to_rfc3339(),
                expires_at.to_rfc3339(),
                route_decision_id,
            ],
        )?;
        let mut resource_keys = Vec::new();
        let mut all_resources = vec![("project".to_owned(), project_id.clone())];
        all_resources.extend_from_slice(resources);
        for (kind, key) in all_resources {
            tx.execute(
                "INSERT INTO resource_locks(
                    id, resource_kind, resource_key, claim_id, mode, acquired_at, expires_at
                 ) VALUES (?1, ?2, ?3, ?4, 'exclusive', ?5, ?6)",
                params![
                    Ulid::new().to_string(),
                    kind,
                    key,
                    claim_id,
                    now.to_rfc3339(),
                    expires_at.to_rfc3339(),
                ],
            )
            .with_context(|| format!("resource lock is unavailable: {kind}:{key}"))?;
            resource_keys.push(format!("{kind}:{key}"));
        }
        transition_task_tx(
            &tx,
            task_id,
            TaskStatus::Ready,
            TaskStatus::Leased,
            "scheduler_claimed",
        )?;
        append_event_tx(
            &tx,
            Some(&project_id),
            Some(task_id),
            None,
            "scheduler.task_claimed",
            "scheduler",
            &serde_json::json!({
                "claim_id": claim_id,
                "instance_id": instance_id,
                "leader_generation": leader_generation,
                "expires_at": expires_at,
                "resources": resource_keys,
            }),
        )?;
        tx.commit()?;
        Ok(SchedulerClaim {
            id: claim_id,
            task_id: task_id.into(),
            instance_id: instance_id.into(),
            task_version: expected_task_version,
            acquired_at: now,
            expires_at,
            resource_keys,
            route_decision_id: route_decision_id.map(str::to_owned),
        })
    }

    pub fn active_scheduler_claim_count(&self, now: DateTime<Utc>) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM scheduler_claims WHERE status = 'active' AND expires_at > ?1",
            [now.to_rfc3339()],
            |row| row.get(0),
        )?;
        usize::try_from(count).map_err(Into::into)
    }

    pub fn heartbeat_scheduler_claims(
        &mut self,
        instance_id: &str,
        leader_generation: i64,
        now: DateTime<Utc>,
        ttl: std::time::Duration,
    ) -> Result<usize> {
        let expires_at =
            now + chrono::Duration::from_std(ttl).context("scheduler claim TTL is too large")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_scheduler_leader_tx(&tx, instance_id, leader_generation, now)?;
        expire_scheduler_claims_tx(&tx, now)?;
        let changed = tx.execute(
            "UPDATE scheduler_claims SET expires_at = ?4
             WHERE instance_id = ?1 AND leader_generation = ?2
               AND status = 'active' AND expires_at > ?3",
            params![
                instance_id,
                leader_generation,
                now.to_rfc3339(),
                expires_at.to_rfc3339(),
            ],
        )?;
        tx.execute(
            "UPDATE resource_locks SET expires_at = ?3
             WHERE claim_id IN (
                 SELECT id FROM scheduler_claims
                 WHERE instance_id = ?1 AND leader_generation = ?2 AND status = 'active'
             ) AND released_at IS NULL",
            params![instance_id, leader_generation, expires_at.to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(changed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_claimed_run(
        &mut self,
        claim_id: &str,
        instance_id: &str,
        leader_generation: i64,
        run_id: &str,
        adapter: &str,
        worktree: &str,
        branch: &str,
        base_commit: &str,
        now: DateTime<Utc>,
        lease_ttl: std::time::Duration,
    ) -> Result<ClaimedRunStart> {
        let lease_expires_at =
            now + chrono::Duration::from_std(lease_ttl).context("run lease TTL is too large")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        assert_scheduler_leader_tx(&tx, instance_id, leader_generation, now)?;
        let claim: Option<(String, Option<String>, String, String, String)> = tx
            .query_row(
                "SELECT c.task_id, c.route_decision_id, c.status, c.expires_at, t.status
                 FROM scheduler_claims c
                 JOIN tasks t ON t.id = c.task_id
                 WHERE c.id = ?1 AND c.instance_id = ?2 AND c.leader_generation = ?3",
                params![claim_id, instance_id, leader_generation],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((task_id, route_decision_id, claim_status, claim_expiry, task_status)) = claim
        else {
            bail!("scheduler claim is missing or fenced: {claim_id}");
        };
        if claim_status != "active" || parse_time(claim_expiry)? <= now {
            bail!("scheduler claim is not active or has expired: {claim_id}");
        }
        if task_status != "leased" {
            bail!("claimed task is not leased: {task_id} ({task_status})");
        }
        let route_decision_id = route_decision_id
            .ok_or_else(|| anyhow!("scheduler claim has no route decision: {claim_id}"))?;
        let (route_allowed, selected_adapter): (bool, Option<String>) = tx.query_row(
            "SELECT allowed, selected_adapter FROM route_decisions
             WHERE id = ?1 AND task_id = ?2",
            params![route_decision_id, task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !route_allowed || selected_adapter.as_deref() != Some(adapter) {
            bail!("scheduler claim route no longer authorizes adapter {adapter}");
        }
        let action_key = format!("agent-start:{claim_id}");

        transition_task_tx(
            &tx,
            &task_id,
            TaskStatus::Leased,
            TaskStatus::Planning,
            "scheduler_claim_consumed",
        )?;
        transition_task_tx(
            &tx,
            &task_id,
            TaskStatus::Planning,
            TaskStatus::Running,
            "sandbox_attested",
        )?;
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
            "INSERT INTO leases(
                id, task_id, run_id, owner, acquired_at, heartbeat_at, expires_at, generation
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7)",
            params![
                Ulid::new().to_string(),
                task_id,
                run_id,
                instance_id,
                now.to_rfc3339(),
                lease_expires_at.to_rfc3339(),
                leader_generation,
            ],
        )?;
        tx.execute(
            "INSERT INTO run_supervision(run_id, attempt, updated_at)
             SELECT ?1, retries_used + 1, ?2
             FROM task_retry_state WHERE task_id = ?3",
            params![run_id, now.to_rfc3339(), task_id],
        )?;
        let consumed = tx.execute(
            "UPDATE scheduler_claims
             SET status = 'consumed', consumed_at = ?2, run_id = ?3, action_key = ?4
             WHERE id = ?1 AND status = 'active'",
            params![claim_id, now.to_rfc3339(), run_id, action_key],
        )?;
        if consumed != 1 {
            bail!("scheduler claim was already consumed: {claim_id}");
        }
        append_event_tx(
            &tx,
            None,
            Some(&task_id),
            Some(run_id),
            "scheduler.claim_consumed",
            "scheduler",
            &serde_json::json!({
                "claim_id": claim_id,
                "action_key": action_key,
                "adapter": adapter,
                "instance_id": instance_id,
                "leader_generation": leader_generation,
            }),
        )?;
        append_event_tx(
            &tx,
            None,
            Some(&task_id),
            Some(run_id),
            "run.started",
            "control_plane",
            &serde_json::json!({"adapter": adapter, "worktree": worktree, "branch": branch}),
        )?;
        tx.commit()?;
        Ok(ClaimedRunStart {
            claim_id: claim_id.into(),
            task_id,
            run_id: run_id.into(),
            route_decision_id,
            action_key,
            started_at: now,
        })
    }

    pub fn recover_expired_scheduler_claims(&mut self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recovered = expire_scheduler_claims_tx(&tx, now)?;
        tx.commit()?;
        Ok(recovered)
    }

    pub fn record_scheduler_wake(
        &mut self,
        task_id: &str,
        reason_code: &str,
        wake_at: Option<DateTime<Utc>>,
        detail: &serde_json::Value,
        now: DateTime<Utc>,
    ) -> Result<SchedulerWake> {
        self.task(task_id)?;
        self.conn.execute(
            "INSERT INTO scheduler_wakes(task_id, reason_code, wake_at, detail_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(task_id) DO UPDATE SET
                reason_code = excluded.reason_code,
                wake_at = excluded.wake_at,
                detail_json = excluded.detail_json,
                updated_at = excluded.updated_at",
            params![
                task_id,
                reason_code,
                wake_at.map(|value| value.to_rfc3339()),
                serde_json::to_string(detail)?,
                now.to_rfc3339(),
            ],
        )?;
        Ok(SchedulerWake {
            task_id: task_id.into(),
            reason_code: reason_code.into(),
            wake_at,
            detail: detail.clone(),
            updated_at: now,
        })
    }

    pub fn scheduler_wakes(&self) -> Result<Vec<SchedulerWake>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, reason_code, wake_at, detail_json, updated_at
             FROM scheduler_wakes ORDER BY wake_at IS NULL, wake_at, task_id",
        )?;
        let rows = stmt.query_map([], |row| {
            let wake_at: Option<String> = row.get(2)?;
            let detail: String = row.get(3)?;
            Ok(SchedulerWake {
                task_id: row.get(0)?,
                reason_code: row.get(1)?,
                wake_at: wake_at.map(parse_time).transpose()?,
                detail: parse_json(detail)?,
                updated_at: parse_time(row.get(4)?)?,
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
                estimated_seconds, uncertainty_percent, checkpoint_seconds, day_affinity,
                fake_write_path, fake_write_content, status, version, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, 'draft', 1, ?18, ?18
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
                new.day_affinity.to_string(),
                new.fake_write_path,
                new.fake_write_content,
                now.to_rfc3339(),
            ],
        )?;
        tx.execute(
            "INSERT INTO task_retry_state(task_id, retry_limit, retries_used, updated_at)
             VALUES (?1, 3, 0, ?2)",
            params![id, now.to_rfc3339()],
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
                required_headroom_percent, quota_json, schedule_json, policy_hash, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                decision.id,
                decision.task_id,
                decision.selected_adapter,
                decision.allowed,
                decision.reason,
                decision.required_headroom_percent,
                to_json(&decision.quota)?,
                to_json(&decision.schedule)?,
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
        tx.execute(
            "INSERT INTO run_supervision(run_id, attempt, updated_at)
             SELECT ?1, retries_used + 1, ?2
             FROM task_retry_state WHERE task_id = ?3",
            params![run_id, now.to_rfc3339(), task_id],
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
        tx.execute(
            "UPDATE resource_locks SET released_at = ?2
             WHERE claim_id IN (
                 SELECT id FROM scheduler_claims WHERE run_id = ?1
             ) AND released_at IS NULL",
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

    pub fn retry_state(&self, task_id: &str) -> Result<RetryState> {
        self.conn
            .query_row(
                "SELECT task_id, retry_limit, retries_used, retry_not_before,
                        last_failure_category, updated_at
                 FROM task_retry_state WHERE task_id = ?1",
                [task_id],
                map_retry_state,
            )
            .optional()?
            .ok_or_else(|| anyhow!("retry state not found for task: {task_id}"))
    }

    pub fn set_retry_limit(&mut self, task_id: &str, retry_limit: u32) -> Result<RetryState> {
        if retry_limit > 20 {
            bail!("retry limit must be in 0..=20");
        }
        let changed = self.conn.execute(
            "UPDATE task_retry_state SET retry_limit = ?2, updated_at = ?3 WHERE task_id = ?1",
            params![task_id, retry_limit, Utc::now().to_rfc3339()],
        )?;
        if changed != 1 {
            bail!("retry state not found for task: {task_id}");
        }
        self.retry_state(task_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn plan_retry(
        &mut self,
        task_id: &str,
        run_id: &str,
        failure: FailureCategory,
        now: DateTime<Utc>,
        base_delay: std::time::Duration,
        max_delay: std::time::Duration,
    ) -> Result<RetryPlan> {
        if max_delay < base_delay {
            bail!("maximum retry delay must be at least the base delay");
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (status, retry_limit, retries_used): (String, u32, u32) = tx.query_row(
            "SELECT t.status, r.retry_limit, r.retries_used
             FROM tasks t JOIN task_retry_state r ON r.task_id = t.id
             WHERE t.id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if status != TaskStatus::Failed.to_string() {
            bail!("retry can only be planned for a failed task; current status is {status}");
        }
        let retry_number = retries_used.saturating_add(1);
        let scheduled = failure.retryable() && retries_used < retry_limit;
        let (reason_code, retry_at, delay_seconds) = if scheduled {
            let delay = deterministic_retry_delay(task_id, retry_number, base_delay, max_delay)?;
            let retry_at =
                now + chrono::Duration::from_std(delay).context("retry delay is too large")?;
            tx.execute(
                "UPDATE task_retry_state
                 SET retries_used = retries_used + 1, retry_not_before = ?2,
                     last_failure_category = ?3, updated_at = ?4
                 WHERE task_id = ?1 AND retries_used = ?5",
                params![
                    task_id,
                    retry_at.to_rfc3339(),
                    failure.to_string(),
                    now.to_rfc3339(),
                    retries_used,
                ],
            )?;
            transition_task_tx(
                &tx,
                task_id,
                TaskStatus::Failed,
                TaskStatus::Ready,
                "retry_scheduled",
            )?;
            (
                "retry.scheduled".to_owned(),
                Some(retry_at),
                Some(delay.as_secs()),
            )
        } else {
            tx.execute(
                "UPDATE task_retry_state
                 SET retry_not_before = NULL, last_failure_category = ?2, updated_at = ?3
                 WHERE task_id = ?1",
                params![task_id, failure.to_string(), now.to_rfc3339()],
            )?;
            (
                if failure.retryable() {
                    "retry.exhausted"
                } else {
                    "retry.permanent_failure"
                }
                .to_owned(),
                None,
                None,
            )
        };
        let plan = RetryPlan {
            task_id: task_id.into(),
            run_id: run_id.into(),
            scheduled,
            reason_code,
            retry_number,
            retry_at,
            delay_seconds,
            failure_category: failure,
        };
        append_event_tx(
            &tx,
            None,
            Some(task_id),
            Some(run_id),
            "run.retry_planned",
            "supervisor",
            &plan,
        )?;
        tx.commit()?;
        Ok(plan)
    }

    pub fn run_lease_context(&self, run_id: &str) -> Result<(String, String, i64)> {
        self.conn
            .query_row(
                "SELECT task_id, owner, generation FROM leases
                 WHERE run_id = ?1 AND released_at IS NULL",
                [run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("active run lease not found: {run_id}"))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_run_checkpoint(
        &mut self,
        run_id: &str,
        owner: &str,
        generation: i64,
        now: DateTime<Utc>,
        lease_ttl: std::time::Duration,
        action: CheckpointAction,
        reason_code: &str,
        next_checkpoint_at: Option<DateTime<Utc>>,
        detail: &serde_json::Value,
    ) -> Result<RunCheckpoint> {
        let lease_expires_at =
            now + chrono::Duration::from_std(lease_ttl).context("run lease TTL is too large")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let context: Option<(String, String, String, i64, String)> = tx
            .query_row(
                "SELECT r.task_id, r.status, t.status, l.generation, l.expires_at
                 FROM runs r JOIN tasks t ON t.id = r.task_id
                 JOIN leases l ON l.run_id = r.id
                 WHERE r.id = ?1 AND l.owner = ?2 AND l.released_at IS NULL",
                params![run_id, owner],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((task_id, run_status, task_status, lease_generation, lease_expiry)) = context
        else {
            bail!("active run lease is missing or owned by another supervisor");
        };
        if lease_generation != generation || parse_time(lease_expiry)? <= now {
            bail!("run lease is expired or fenced by another generation");
        }
        if run_status != "running" || task_status != TaskStatus::Running.to_string() {
            bail!("checkpoint requires a running run and task");
        }
        let sequence: i64 = tx.query_row(
            "UPDATE run_supervision
             SET checkpoint_sequence = checkpoint_sequence + 1, updated_at = ?2,
                 version = version + 1
             WHERE run_id = ?1
             RETURNING checkpoint_sequence",
            params![run_id, now.to_rfc3339()],
            |row| row.get(0),
        )?;
        let checkpoint = RunCheckpoint {
            id: Ulid::new().to_string(),
            run_id: run_id.into(),
            sequence,
            evaluated_at: now,
            action,
            reason_code: reason_code.into(),
            next_checkpoint_at,
            detail: detail.clone(),
        };
        tx.execute(
            "INSERT INTO run_checkpoints(
                id, run_id, sequence, evaluated_at, action, reason_code,
                next_checkpoint_at, detail_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                checkpoint.id,
                run_id,
                sequence,
                now.to_rfc3339(),
                action.to_string(),
                reason_code,
                next_checkpoint_at.map(|value| value.to_rfc3339()),
                to_json(detail)?,
            ],
        )?;
        match action {
            CheckpointAction::Continue | CheckpointAction::ShortenCheckpoint => {
                tx.execute(
                    "UPDATE leases SET heartbeat_at = ?4, expires_at = ?5
                     WHERE run_id = ?1 AND owner = ?2 AND generation = ?3
                       AND released_at IS NULL",
                    params![
                        run_id,
                        owner,
                        generation,
                        now.to_rfc3339(),
                        lease_expires_at.to_rfc3339(),
                    ],
                )?;
                tx.execute(
                    "UPDATE runs SET heartbeat_at = ?2, checkpoint_due_at = ?3 WHERE id = ?1",
                    params![
                        run_id,
                        now.to_rfc3339(),
                        next_checkpoint_at.unwrap_or(lease_expires_at).to_rfc3339(),
                    ],
                )?;
            }
            CheckpointAction::Pause | CheckpointAction::Cancel => {
                let requested_action = if action == CheckpointAction::Pause {
                    "pause"
                } else {
                    "cancel"
                };
                tx.execute(
                    "UPDATE runs SET heartbeat_at = ?2, checkpoint_due_at = ?3 WHERE id = ?1",
                    params![run_id, now.to_rfc3339(), lease_expires_at.to_rfc3339()],
                )?;
                tx.execute(
                    "UPDATE leases SET heartbeat_at = ?2, expires_at = ?3
                     WHERE run_id = ?1 AND released_at IS NULL",
                    params![run_id, now.to_rfc3339(), lease_expires_at.to_rfc3339()],
                )?;
                tx.execute(
                    "UPDATE run_supervision
                     SET cancellation_status = 'requested', cancellation_reason = ?2,
                         cancellation_requested_at = COALESCE(cancellation_requested_at, ?3),
                         requested_action = ?4, updated_at = ?3, version = version + 1
                     WHERE run_id = ?1",
                    params![run_id, reason_code, now.to_rfc3339(), requested_action],
                )?;
            }
        }
        append_event_tx(
            &tx,
            None,
            Some(&task_id),
            Some(run_id),
            "run.checkpointed",
            "supervisor",
            &checkpoint,
        )?;
        tx.commit()?;
        Ok(checkpoint)
    }

    pub fn request_run_cancellation(
        &mut self,
        run_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task_id: String = tx.query_row(
            "SELECT task_id FROM runs WHERE id = ?1 AND status = 'running'",
            [run_id],
            |row| row.get(0),
        )?;
        let changed = tx.execute(
            "UPDATE run_supervision
             SET cancellation_status = 'requested', cancellation_reason = ?2,
                 cancellation_requested_at = ?3, requested_action = 'cancel',
                 updated_at = ?3, version = version + 1
             WHERE run_id = ?1 AND cancellation_status = 'none'",
            params![run_id, reason, now.to_rfc3339()],
        )?;
        if changed == 1 {
            append_event_tx(
                &tx,
                None,
                Some(&task_id),
                Some(run_id),
                "run.cancellation_requested",
                "user",
                &serde_json::json!({"reason": reason}),
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn run_cancellation_requested(&self, run_id: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT cancellation_status = 'requested' FROM run_supervision WHERE run_id = ?1",
                [run_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("run supervision not found: {run_id}"))
    }

    pub fn record_process_outcome(
        &mut self,
        run_id: &str,
        failure: Option<FailureCategory>,
        exit_code: Option<i32>,
        outcome: &serde_json::Value,
        termination: Option<&serde_json::Value>,
        now: DateTime<Utc>,
    ) -> Result<String> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (task_id, run_status, task_status, requested_action): (
            String,
            String,
            String,
            Option<String>,
        ) = tx.query_row(
            "SELECT r.task_id, r.status, t.status, s.requested_action
             FROM runs r JOIN tasks t ON t.id = r.task_id
             JOIN run_supervision s ON s.run_id = r.id WHERE r.id = ?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        if run_status != "running" || task_status != TaskStatus::Running.to_string() {
            bail!("process outcome requires a running run and task");
        }
        let (run_next, task_next) = match failure {
            None => ("verifying", TaskStatus::Verifying),
            Some(FailureCategory::Cancelled) if requested_action.as_deref() == Some("pause") => {
                ("paused", TaskStatus::Paused)
            }
            Some(FailureCategory::Cancelled) => ("cancelled", TaskStatus::Cancelled),
            Some(_) => ("failed", TaskStatus::Failed),
        };
        transition_task_tx(
            &tx,
            &task_id,
            TaskStatus::Running,
            task_next,
            "process_outcome",
        )?;
        tx.execute(
            "UPDATE runs SET status = ?2, exit_code = ?3, heartbeat_at = ?4,
                 ended_at = CASE WHEN ?2 = 'verifying' THEN ended_at ELSE ?4 END
             WHERE id = ?1",
            params![run_id, run_next, exit_code, now.to_rfc3339()],
        )?;
        tx.execute(
            "UPDATE run_supervision
             SET failure_category = ?2, termination_json = ?3, outcome_json = ?4,
                 cancellation_status = CASE WHEN ?2 = 'cancelled' THEN 'completed'
                                            ELSE cancellation_status END,
                 updated_at = ?5, version = version + 1 WHERE run_id = ?1",
            params![
                run_id,
                failure.map(|value| value.to_string()),
                termination.map(to_json).transpose()?,
                to_json(outcome)?,
                now.to_rfc3339(),
            ],
        )?;
        if failure.is_some() {
            tx.execute(
                "UPDATE leases SET released_at = ?2 WHERE run_id = ?1 AND released_at IS NULL",
                params![run_id, now.to_rfc3339()],
            )?;
            tx.execute(
                "UPDATE resource_locks SET released_at = ?2
                 WHERE claim_id IN (SELECT id FROM scheduler_claims WHERE run_id = ?1)
                   AND released_at IS NULL",
                params![run_id, now.to_rfc3339()],
            )?;
        }
        append_event_tx(
            &tx,
            None,
            Some(&task_id),
            Some(run_id),
            "run.process_outcome",
            "supervisor",
            outcome,
        )?;
        match task_next {
            TaskStatus::Paused => {
                enqueue_notification_tx(
                    &tx,
                    "blocked",
                    "warning",
                    Some(&task_id),
                    Some(run_id),
                    "Task paused",
                    "Runtime supervision paused the task after terminating its process safely.",
                    now,
                )?;
            }
            TaskStatus::Failed => {
                enqueue_notification_tx(
                    &tx,
                    "failure",
                    "error",
                    Some(&task_id),
                    Some(run_id),
                    "Task run failed",
                    "The supervised process failed; inspect the bounded run evidence and retry state.",
                    now,
                )?;
            }
            TaskStatus::Cancelled => {
                enqueue_notification_tx(
                    &tx,
                    "operation",
                    "warning",
                    Some(&task_id),
                    Some(run_id),
                    "Task run cancelled",
                    "The supervised process exited after a cancellation request.",
                    now,
                )?;
            }
            _ => {}
        }
        tx.commit()?;
        Ok(task_id)
    }

    pub fn adapter_circuit_gate(
        &mut self,
        adapter: &str,
        provider: &str,
        account: &str,
        now: DateTime<Utc>,
        claim_probe: bool,
    ) -> Result<(bool, Option<DateTime<Utc>>, String)> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = query_circuit_tx(&tx, adapter, provider, account)?;
        let result = match existing {
            None => (true, None, "circuit.closed".to_owned()),
            Some(circuit) if circuit.state == "closed" => (true, None, "circuit.closed".to_owned()),
            Some(circuit) if circuit.state == "open" => {
                let probe_due = circuit.next_probe_at.is_none_or(|probe| probe <= now);
                if probe_due && claim_probe {
                    let changed = tx.execute(
                        "UPDATE adapter_circuits
                         SET state = 'half_open', probe_claimed_at = ?4, updated_at = ?4
                         WHERE adapter = ?1 AND provider = ?2 AND account = ?3
                           AND state = 'open' AND (next_probe_at IS NULL OR next_probe_at <= ?4)",
                        params![adapter, provider, account, now.to_rfc3339()],
                    )?;
                    if changed == 1 {
                        (true, None, "circuit.half_open_probe".to_owned())
                    } else {
                        (
                            false,
                            circuit.next_probe_at,
                            "circuit.probe_claimed".to_owned(),
                        )
                    }
                } else if probe_due {
                    (true, None, "circuit.probe_available".to_owned())
                } else {
                    (false, circuit.next_probe_at, "circuit.open".to_owned())
                }
            }
            Some(circuit) => (
                false,
                circuit.next_probe_at,
                "circuit.probe_claimed".to_owned(),
            ),
        };
        tx.commit()?;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_adapter_outcome(
        &mut self,
        adapter: &str,
        provider: &str,
        account: &str,
        failure: Option<FailureCategory>,
        now: DateTime<Utc>,
        failure_threshold: u32,
        cooldown: std::time::Duration,
    ) -> Result<CircuitBreaker> {
        if failure_threshold == 0 {
            bail!("circuit failure threshold must be greater than zero");
        }
        let next_probe =
            now + chrono::Duration::from_std(cooldown).context("cooldown is too large")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = query_circuit_tx(&tx, adapter, provider, account)?;
        let (state, consecutive_failures, opened_at, next_probe_at, probe_claimed_at) =
            match failure {
                None => ("closed", 0_u32, None, None, None),
                Some(category) if category.retryable() => {
                    let count = existing
                        .as_ref()
                        .map_or(1, |circuit| circuit.consecutive_failures.saturating_add(1));
                    let open = existing
                        .as_ref()
                        .is_some_and(|circuit| circuit.state == "half_open")
                        || count >= failure_threshold;
                    if open {
                        ("open", count, Some(now), Some(next_probe), None)
                    } else {
                        ("closed", count, None, None, None)
                    }
                }
                Some(_) => ("closed", 0_u32, None, None, None),
            };
        tx.execute(
            "INSERT INTO adapter_circuits(
                adapter, provider, account, state, consecutive_failures,
                last_failure_category, opened_at, next_probe_at, probe_claimed_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(adapter, provider, account) DO UPDATE SET
                state = excluded.state,
                consecutive_failures = excluded.consecutive_failures,
                last_failure_category = excluded.last_failure_category,
                opened_at = excluded.opened_at,
                next_probe_at = excluded.next_probe_at,
                probe_claimed_at = excluded.probe_claimed_at,
                updated_at = excluded.updated_at",
            params![
                adapter,
                provider,
                account,
                state,
                consecutive_failures,
                failure.map(|value| value.to_string()),
                opened_at.map(|value| value.to_rfc3339()),
                next_probe_at.map(|value| value.to_rfc3339()),
                probe_claimed_at.map(|value: DateTime<Utc>| value.to_rfc3339()),
                now.to_rfc3339(),
            ],
        )?;
        let circuit = query_circuit_tx(&tx, adapter, provider, account)?
            .ok_or_else(|| anyhow!("adapter circuit write was not visible"))?;
        tx.commit()?;
        Ok(circuit)
    }

    pub fn adapter_circuits(&self) -> Result<Vec<CircuitBreaker>> {
        let mut stmt = self.conn.prepare(
            "SELECT adapter, provider, account, state, consecutive_failures,
                    last_failure_category, opened_at, next_probe_at, probe_claimed_at, updated_at
             FROM adapter_circuits ORDER BY adapter, provider, account",
        )?;
        let rows = stmt.query_map([], map_circuit)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
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
            tx.execute(
                "UPDATE resource_locks SET released_at = ?2
                 WHERE claim_id IN (
                     SELECT id FROM scheduler_claims WHERE run_id = ?1
                 ) AND released_at IS NULL",
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

#[cfg(unix)]
fn secure_database_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn secure_database_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn validate_percentage(value: Option<f64>, label: &str) -> Result<()> {
    if let Some(value) = value
        && (!value.is_finite() || !(0.0..=100.0).contains(&value))
    {
        bail!("{label} percentage must be in 0..=100");
    }
    Ok(())
}

fn assert_scheduler_leader_tx(
    tx: &Transaction<'_>,
    instance_id: &str,
    generation: i64,
    now: DateTime<Utc>,
) -> Result<()> {
    let valid: bool = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM scheduler_leader
            WHERE singleton = 1 AND instance_id = ?1 AND generation = ?2 AND expires_at > ?3
         )",
        params![instance_id, generation, now.to_rfc3339()],
        |row| row.get(0),
    )?;
    if !valid {
        bail!("scheduler leadership is missing, expired, or fenced by a newer generation");
    }
    Ok(())
}

fn expire_scheduler_claims_tx(tx: &Transaction<'_>, now: DateTime<Utc>) -> Result<Vec<String>> {
    let mut stmt = tx.prepare(
        "SELECT id, task_id FROM scheduler_claims
         WHERE status = 'active' AND expires_at <= ?1 ORDER BY acquired_at, id",
    )?;
    let expired = stmt
        .query_map([now.to_rfc3339()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for (claim_id, task_id) in &expired {
        let status: String =
            tx.query_row("SELECT status FROM tasks WHERE id = ?1", [task_id], |row| {
                row.get(0)
            })?;
        if status == "leased" {
            transition_task_tx(
                tx,
                task_id,
                TaskStatus::Leased,
                TaskStatus::Paused,
                "scheduler_claim_expired",
            )?;
            transition_task_tx(
                tx,
                task_id,
                TaskStatus::Paused,
                TaskStatus::Ready,
                "scheduler_requeued",
            )?;
        }
        tx.execute(
            "UPDATE scheduler_claims SET status = 'expired', released_at = ?2
             WHERE id = ?1 AND status = 'active'",
            params![claim_id, now.to_rfc3339()],
        )?;
        tx.execute(
            "UPDATE resource_locks SET released_at = ?2
             WHERE claim_id = ?1 AND released_at IS NULL",
            params![claim_id, now.to_rfc3339()],
        )?;
        append_event_tx(
            tx,
            None,
            Some(task_id),
            None,
            "scheduler.claim_expired",
            "recovery",
            &serde_json::json!({"claim_id": claim_id, "requeued": status == "leased"}),
        )?;
    }
    Ok(expired.into_iter().map(|(_, task_id)| task_id).collect())
}

fn release_scheduler_claims_for_instance_tx(
    tx: &Transaction<'_>,
    instance_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<String>> {
    let mut stmt = tx.prepare(
        "SELECT id, task_id FROM scheduler_claims
         WHERE instance_id = ?1 AND status = 'active' ORDER BY acquired_at, id",
    )?;
    let claims = stmt
        .query_map([instance_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for (claim_id, task_id) in &claims {
        let status: String =
            tx.query_row("SELECT status FROM tasks WHERE id = ?1", [task_id], |row| {
                row.get(0)
            })?;
        if status == "leased" {
            transition_task_tx(
                tx,
                task_id,
                TaskStatus::Leased,
                TaskStatus::Paused,
                "scheduler_graceful_stop",
            )?;
            transition_task_tx(
                tx,
                task_id,
                TaskStatus::Paused,
                TaskStatus::Ready,
                "scheduler_requeued",
            )?;
        }
        tx.execute(
            "UPDATE scheduler_claims SET status = 'released', released_at = ?2
             WHERE id = ?1 AND status = 'active'",
            params![claim_id, now.to_rfc3339()],
        )?;
        tx.execute(
            "UPDATE resource_locks SET released_at = ?2
             WHERE claim_id = ?1 AND released_at IS NULL",
            params![claim_id, now.to_rfc3339()],
        )?;
    }
    Ok(claims.into_iter().map(|(_, task_id)| task_id).collect())
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
    estimated_seconds, uncertainty_percent, checkpoint_seconds, day_affinity,
    fake_write_path, fake_write_content, status, version, created_at, updated_at
    FROM tasks";
const TASK_SELECT_BY_ID: &str = "SELECT
    id, project_id, title, goal, rationale, scope_json, non_scope_json,
    acceptance_json, verification_argv_json, priority, risk_class,
    estimated_seconds, uncertainty_percent, checkpoint_seconds, day_affinity,
    fake_write_path, fake_write_content, status, version, created_at, updated_at
    FROM tasks WHERE id = ?1";

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let affinity: String = row.get(14)?;
    let status: String = row.get(17)?;
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
        day_affinity: crate::domain::DayAffinity::from_str(&affinity).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                affinity.len(),
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })?,
        fake_write_path: row.get(15)?,
        fake_write_content: row.get(16)?,
        status: TaskStatus::from_str(&status).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                status.len(),
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })?,
        version: row.get(18)?,
        created_at: parse_time(row.get(19)?)?,
        updated_at: parse_time(row.get(20)?)?,
    })
}

fn map_calendar(row: &rusqlite::Row<'_>) -> rusqlite::Result<CalendarProfile> {
    Ok(CalendarProfile {
        id: row.get(0)?,
        slug: row.get(1)?,
        timezone: row.get(2)?,
        weekly_pattern: row.get(3)?,
        version: row.get(4)?,
        created_at: parse_time(row.get(5)?)?,
        updated_at: parse_time(row.get(6)?)?,
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

fn map_retry_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<RetryState> {
    let retry_not_before: Option<String> = row.get(3)?;
    let last_failure: Option<String> = row.get(4)?;
    Ok(RetryState {
        task_id: row.get(0)?,
        retry_limit: row.get(1)?,
        retries_used: row.get(2)?,
        retry_not_before: retry_not_before.map(parse_time).transpose()?,
        last_failure_category: last_failure
            .map(|value| {
                FailureCategory::from_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        value.len(),
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .transpose()?,
        updated_at: parse_time(row.get(5)?)?,
    })
}

fn map_circuit(row: &rusqlite::Row<'_>) -> rusqlite::Result<CircuitBreaker> {
    let last_failure: Option<String> = row.get(5)?;
    let opened_at: Option<String> = row.get(6)?;
    let next_probe_at: Option<String> = row.get(7)?;
    let probe_claimed_at: Option<String> = row.get(8)?;
    Ok(CircuitBreaker {
        adapter: row.get(0)?,
        provider: row.get(1)?,
        account: row.get(2)?,
        state: row.get(3)?,
        consecutive_failures: row.get(4)?,
        last_failure_category: last_failure
            .map(|value| {
                FailureCategory::from_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        value.len(),
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .transpose()?,
        opened_at: opened_at.map(parse_time).transpose()?,
        next_probe_at: next_probe_at.map(parse_time).transpose()?,
        probe_claimed_at: probe_claimed_at.map(parse_time).transpose()?,
        updated_at: parse_time(row.get(9)?)?,
    })
}

fn query_circuit_tx(
    tx: &Transaction<'_>,
    adapter: &str,
    provider: &str,
    account: &str,
) -> Result<Option<CircuitBreaker>> {
    tx.query_row(
        "SELECT adapter, provider, account, state, consecutive_failures,
                last_failure_category, opened_at, next_probe_at, probe_claimed_at, updated_at
         FROM adapter_circuits WHERE adapter = ?1 AND provider = ?2 AND account = ?3",
        params![adapter, provider, account],
        map_circuit,
    )
    .optional()
    .map_err(Into::into)
}

fn deterministic_retry_delay(
    task_id: &str,
    retry_number: u32,
    base_delay: std::time::Duration,
    max_delay: std::time::Duration,
) -> Result<std::time::Duration> {
    let exponent = retry_number.saturating_sub(1).min(31);
    let multiplier = 1_u128 << exponent;
    let uncapped_ms = base_delay.as_millis().saturating_mul(multiplier);
    let capped_ms = uncapped_ms.min(max_delay.as_millis());
    let digest = Sha256::digest(format!("{task_id}:{retry_number}").as_bytes());
    let sample = u16::from_be_bytes([digest[0], digest[1]]) as u128;
    // Stable jitter in the inclusive 80%..120% range prevents synchronized retries.
    let basis_points = 8_000_u128 + (sample * 4_000_u128 / u16::MAX as u128);
    let jittered_ms = capped_ms.saturating_mul(basis_points) / 10_000_u128;
    let bounded_ms = jittered_ms.min(max_delay.as_millis());
    let milliseconds = u64::try_from(bounded_ms).context("retry delay exceeds u64")?;
    Ok(std::time::Duration::from_millis(milliseconds))
}

#[allow(clippy::too_many_arguments)]
fn enqueue_notification_tx(
    tx: &Transaction<'_>,
    kind: &str,
    severity: &str,
    task_id: Option<&str>,
    run_id: Option<&str>,
    title: &str,
    body: &str,
    now: DateTime<Utc>,
) -> Result<LocalNotification> {
    if !matches!(kind, "review" | "blocked" | "failure" | "operation") {
        bail!("unsupported notification kind: {kind}");
    }
    if !matches!(severity, "info" | "warning" | "error" | "critical") {
        bail!("unsupported notification severity: {severity}");
    }
    if !(1..=200).contains(&title.chars().count()) {
        bail!("notification title must contain 1..=200 characters");
    }
    if !(1..=2_000).contains(&body.chars().count()) {
        bail!("notification body must contain 1..=2000 characters");
    }
    let notification = LocalNotification {
        id: Ulid::new().to_string(),
        kind: kind.into(),
        severity: severity.into(),
        task_id: task_id.map(str::to_owned),
        run_id: run_id.map(str::to_owned),
        title: title.into(),
        body: body.into(),
        created_at: now,
        acknowledged_at: None,
    };
    tx.execute(
        "INSERT INTO local_notifications(
            id, kind, severity, task_id, run_id, title, body, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            notification.id,
            kind,
            severity,
            task_id,
            run_id,
            title,
            body,
            now.to_rfc3339(),
        ],
    )?;
    Ok(notification)
}

fn map_local_notification(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalNotification> {
    let acknowledged_at: Option<String> = row.get(8)?;
    Ok(LocalNotification {
        id: row.get(0)?,
        kind: row.get(1)?,
        severity: row.get(2)?,
        task_id: row.get(3)?,
        run_id: row.get(4)?,
        title: row.get(5)?,
        body: row.get(6)?,
        created_at: parse_time(row.get(7)?)?,
        acknowledged_at: acknowledged_at.map(parse_time).transpose()?,
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

const MIGRATION_2: &str = r#"
CREATE TABLE calendar_profiles (
    id TEXT PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    timezone TEXT NOT NULL,
    weekly_pattern TEXT NOT NULL CHECK(length(weekly_pattern) = 7),
    version INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

ALTER TABLE projects ADD COLUMN calendar_profile_id TEXT REFERENCES calendar_profiles(id);
ALTER TABLE tasks ADD COLUMN day_affinity TEXT NOT NULL DEFAULT 'B'
    CHECK(day_affinity IN ('W', 'O', 'B'));
ALTER TABLE route_decisions ADD COLUMN schedule_json TEXT;

CREATE TABLE calendar_exceptions (
    profile_id TEXT NOT NULL REFERENCES calendar_profiles(id) ON DELETE CASCADE,
    local_date TEXT NOT NULL,
    day_kind TEXT NOT NULL CHECK(day_kind IN ('W', 'O')),
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(profile_id, local_date)
);

INSERT INTO calendar_profiles(
    id, slug, timezone, weekly_pattern, version, created_at, updated_at
) VALUES (
    'default', 'default', 'Etc/UTC', 'WWWWWOO', 1,
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
);

CREATE INDEX idx_calendar_exception_date
    ON calendar_exceptions(profile_id, local_date);
"#;

const MIGRATION_3: &str = r#"
CREATE TABLE scheduler_instances (
    id TEXT PRIMARY KEY,
    hostname TEXT NOT NULL,
    process_id INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'stopped', 'lost'))
);

CREATE TABLE scheduler_leader (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    instance_id TEXT NOT NULL REFERENCES scheduler_instances(id),
    generation INTEGER NOT NULL,
    acquired_at TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE scheduler_claims (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    instance_id TEXT NOT NULL REFERENCES scheduler_instances(id),
    leader_generation INTEGER NOT NULL,
    task_version INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'consumed', 'released', 'expired')),
    acquired_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    released_at TEXT
);

CREATE UNIQUE INDEX idx_scheduler_claim_active_task
    ON scheduler_claims(task_id) WHERE status = 'active';
CREATE INDEX idx_scheduler_claim_expiry
    ON scheduler_claims(expires_at) WHERE status = 'active';

CREATE TABLE resource_locks (
    id TEXT PRIMARY KEY,
    resource_kind TEXT NOT NULL,
    resource_key TEXT NOT NULL,
    claim_id TEXT NOT NULL REFERENCES scheduler_claims(id),
    mode TEXT NOT NULL CHECK(mode IN ('exclusive')),
    acquired_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    released_at TEXT
);

CREATE UNIQUE INDEX idx_resource_lock_active
    ON resource_locks(resource_kind, resource_key) WHERE released_at IS NULL;
CREATE INDEX idx_resource_lock_expiry
    ON resource_locks(expires_at) WHERE released_at IS NULL;

CREATE TABLE scheduler_wakes (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    reason_code TEXT NOT NULL,
    wake_at TEXT,
    detail_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

const MIGRATION_4: &str = r#"
ALTER TABLE scheduler_claims
    ADD COLUMN route_decision_id TEXT REFERENCES route_decisions(id);
ALTER TABLE scheduler_claims
    ADD COLUMN run_id TEXT REFERENCES runs(id);
ALTER TABLE scheduler_claims
    ADD COLUMN action_key TEXT;

CREATE UNIQUE INDEX idx_scheduler_claim_run
    ON scheduler_claims(run_id) WHERE run_id IS NOT NULL;
CREATE UNIQUE INDEX idx_scheduler_claim_action
    ON scheduler_claims(action_key) WHERE action_key IS NOT NULL;
"#;

const MIGRATION_5: &str = r#"
CREATE TABLE task_retry_state (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    retry_limit INTEGER NOT NULL DEFAULT 3 CHECK(retry_limit BETWEEN 0 AND 20),
    retries_used INTEGER NOT NULL DEFAULT 0 CHECK(retries_used >= 0),
    retry_not_before TEXT,
    last_failure_category TEXT,
    updated_at TEXT NOT NULL
);

INSERT INTO task_retry_state(task_id, retry_limit, retries_used, updated_at)
    SELECT id, 3, 0, updated_at FROM tasks;

CREATE TABLE run_supervision (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK(attempt > 0),
    checkpoint_sequence INTEGER NOT NULL DEFAULT 0 CHECK(checkpoint_sequence >= 0),
    failure_category TEXT,
    cancellation_status TEXT NOT NULL DEFAULT 'none'
        CHECK(cancellation_status IN ('none', 'requested', 'completed')),
    cancellation_reason TEXT,
    cancellation_requested_at TEXT,
    requested_action TEXT CHECK(requested_action IN ('pause', 'cancel')),
    termination_json TEXT,
    outcome_json TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL
);

INSERT INTO run_supervision(run_id, attempt, updated_at)
    SELECT id, 1, heartbeat_at FROM runs;

CREATE TABLE run_checkpoints (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    evaluated_at TEXT NOT NULL,
    action TEXT NOT NULL
        CHECK(action IN ('continue', 'shorten_checkpoint', 'pause', 'cancel')),
    reason_code TEXT NOT NULL,
    next_checkpoint_at TEXT,
    detail_json TEXT NOT NULL,
    UNIQUE(run_id, sequence)
);

CREATE INDEX idx_run_checkpoints_run
    ON run_checkpoints(run_id, sequence);

CREATE TABLE adapter_circuits (
    adapter TEXT NOT NULL,
    provider TEXT NOT NULL,
    account TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('closed', 'open', 'half_open')),
    consecutive_failures INTEGER NOT NULL CHECK(consecutive_failures >= 0),
    last_failure_category TEXT,
    opened_at TEXT,
    next_probe_at TEXT,
    probe_claimed_at TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(adapter, provider, account)
);

CREATE INDEX idx_adapter_circuit_probe
    ON adapter_circuits(state, next_probe_at);
"#;

const MIGRATION_6: &str = r#"
CREATE TABLE control_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    pause_new_work INTEGER NOT NULL CHECK(pause_new_work IN (0, 1)),
    emergency_stop INTEGER NOT NULL CHECK(emergency_stop IN (0, 1)),
    reason TEXT,
    updated_at TEXT NOT NULL
);

INSERT INTO control_state(singleton, pause_new_work, emergency_stop, reason, updated_at)
VALUES (1, 0, 0, NULL, '1970-01-01T00:00:00Z');

CREATE TABLE local_notifications (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('review', 'blocked', 'failure', 'operation')),
    severity TEXT NOT NULL CHECK(severity IN ('info', 'warning', 'error', 'critical')),
    task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 200),
    body TEXT NOT NULL CHECK(length(body) BETWEEN 1 AND 2000),
    created_at TEXT NOT NULL,
    acknowledged_at TEXT
);

CREATE INDEX idx_local_notifications_pending
    ON local_notifications(created_at) WHERE acknowledged_at IS NULL;
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Barrier},
        thread,
    };
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
            day_affinity: crate::domain::DayAffinity::Both,
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
                day_affinity: crate::domain::DayAffinity::Both,
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
            schedule: None,
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

    #[test]
    fn schema_one_migration_creates_verified_backup_and_preserves_data() {
        let dir = tempdir().unwrap();
        let database_path = dir.path().join("state.db");
        let connection = Connection::open(&database_path).unwrap();
        connection.execute_batch(MIGRATION_1).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "INSERT INTO projects(id, slug, title, root_path, created_at, updated_at, version)
                 VALUES ('phase1-project', 'phase1', 'Phase 1', '/fixture/phase1', ?1, ?1, 1)",
                [&now],
            )
            .unwrap();
        drop(connection);

        let migrated = Database::open(&database_path).unwrap();
        assert_eq!(migrated.project("phase1").unwrap().id, "phase1-project");
        assert_eq!(
            migrated.calendar("default").unwrap().weekly_pattern,
            "WWWWWOO"
        );
        let version: i64 = migrated
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 6);

        let backup = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("state.v1.") && name.ends_with(".backup.db")
                    })
            })
            .expect("version-1 migration backup");
        let backup = Connection::open(backup).unwrap();
        let integrity: String = backup
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let backup_version: i64 = backup
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(backup_version, 1);
    }

    #[test]
    fn leader_fencing_resource_locks_and_expired_claim_recovery_are_durable() {
        let (dir, mut db) = database();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let first = db
            .add_task(&new_task(&project.id, "first", vec![]))
            .unwrap();
        let second = db
            .add_task(&new_task(&project.id, "second", vec![]))
            .unwrap();
        let now = Utc::now();
        db.register_scheduler_instance("scheduler-a", "host", 1, now)
            .unwrap();
        db.register_scheduler_instance("scheduler-b", "host", 2, now)
            .unwrap();
        let leader_a = db
            .acquire_scheduler_leader("scheduler-a", now, std::time::Duration::from_secs(30))
            .unwrap();
        assert!(
            db.acquire_scheduler_leader(
                "scheduler-b",
                now + chrono::Duration::seconds(1),
                std::time::Duration::from_secs(30),
            )
            .is_err()
        );
        let takeover_at = now + chrono::Duration::seconds(31);
        let leader_b = db
            .acquire_scheduler_leader(
                "scheduler-b",
                takeover_at,
                std::time::Duration::from_secs(60),
            )
            .unwrap();
        assert_eq!(leader_b.generation, leader_a.generation + 1);
        assert!(
            db.claim_task_for_scheduler(
                "scheduler-a",
                leader_a.generation,
                &first.id,
                first.version,
                takeover_at,
                std::time::Duration::from_secs(10),
                2,
                None,
                &[],
            )
            .is_err()
        );
        let claim = db
            .claim_task_for_scheduler(
                "scheduler-b",
                leader_b.generation,
                &first.id,
                first.version,
                takeover_at,
                std::time::Duration::from_secs(10),
                2,
                None,
                &[],
            )
            .unwrap();
        assert!(
            claim
                .resource_keys
                .contains(&format!("project:{}", project.id))
        );
        let lock_error = db
            .claim_task_for_scheduler(
                "scheduler-b",
                leader_b.generation,
                &second.id,
                second.version,
                takeover_at,
                std::time::Duration::from_secs(10),
                2,
                None,
                &[],
            )
            .unwrap_err()
            .to_string();
        assert!(lock_error.contains("resource lock"));
        assert_eq!(db.task(&second.id).unwrap().status, TaskStatus::Ready);
        let recovered = db
            .recover_expired_scheduler_claims(takeover_at + chrono::Duration::seconds(11))
            .unwrap();
        assert_eq!(recovered, vec![first.id.clone()]);
        let first = db.task(&first.id).unwrap();
        assert_eq!(first.status, TaskStatus::Ready);
        db.claim_task_for_scheduler(
            "scheduler-b",
            leader_b.generation,
            &second.id,
            second.version,
            takeover_at + chrono::Duration::seconds(11),
            std::time::Duration::from_secs(10),
            2,
            None,
            &[],
        )
        .unwrap();
        let stop_at = takeover_at + chrono::Duration::seconds(12);
        let released = db.stop_scheduler_instance("scheduler-b", stop_at).unwrap();
        assert_eq!(released, vec![second.id.clone()]);
        assert_eq!(db.task(&second.id).unwrap().status, TaskStatus::Ready);
        assert!(
            db.heartbeat_scheduler_leader(
                "scheduler-b",
                leader_b.generation,
                stop_at,
                std::time::Duration::from_secs(30),
            )
            .is_err()
        );
        let leader_a_again = db
            .acquire_scheduler_leader("scheduler-a", stop_at, std::time::Duration::from_secs(30))
            .unwrap();
        assert_eq!(leader_a_again.generation, leader_b.generation + 1);
    }

    #[test]
    fn racing_claims_respect_atomic_global_concurrency_limit() {
        let (dir, mut db) = database();
        let database_path = db.path().to_path_buf();
        let root_a = dir.path().join("project-a");
        let root_b = dir.path().join("project-b");
        fs::create_dir(&root_a).unwrap();
        fs::create_dir(&root_b).unwrap();
        let project_a = db.add_project("a", "A", &root_a).unwrap();
        let project_b = db.add_project("b", "B", &root_b).unwrap();
        let task_a = db.add_task(&new_task(&project_a.id, "a", vec![])).unwrap();
        let task_b = db.add_task(&new_task(&project_b.id, "b", vec![])).unwrap();
        let now = Utc::now();
        db.register_scheduler_instance("scheduler", "host", 1, now)
            .unwrap();
        let leader = db
            .acquire_scheduler_leader("scheduler", now, std::time::Duration::from_secs(60))
            .unwrap();
        drop(db);

        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for task in [task_a, task_b] {
            let barrier = barrier.clone();
            let path = database_path.clone();
            let generation = leader.generation;
            handles.push(thread::spawn(move || {
                let mut db = Database::open(path).unwrap();
                barrier.wait();
                db.claim_task_for_scheduler(
                    "scheduler",
                    generation,
                    &task.id,
                    task.version,
                    now,
                    std::time::Duration::from_secs(30),
                    1,
                    None,
                    &[],
                )
                .is_ok()
            }));
        }
        barrier.wait();
        let successes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 1);
        let reopened = Database::open(database_path).unwrap();
        assert_eq!(reopened.active_scheduler_claim_count(now).unwrap(), 1);
    }

    #[test]
    fn claimed_run_start_is_atomic_single_use_and_releases_project_lock() {
        let (dir, mut db) = database();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let first = db
            .add_task(&new_task(&project.id, "first", vec![]))
            .unwrap();
        let second = db
            .add_task(&new_task(&project.id, "second", vec![]))
            .unwrap();
        let now = Utc::now();
        let decision = RouteDecision {
            id: Ulid::new().to_string(),
            task_id: first.id.clone(),
            selected_adapter: Some("fake".into()),
            allowed: true,
            reason: "fixture".into(),
            required_headroom_percent: 21.0,
            quota: vec![],
            candidates: vec![],
            next_wake_at: None,
            schedule: None,
            policy_hash: "fixture-policy".into(),
            created_at: now,
        };
        db.record_route(&decision).unwrap();
        db.register_scheduler_instance("scheduler", "host", 1, now)
            .unwrap();
        let leader = db
            .acquire_scheduler_leader("scheduler", now, std::time::Duration::from_secs(60))
            .unwrap();
        let claim = db
            .claim_task_for_scheduler(
                "scheduler",
                leader.generation,
                &first.id,
                first.version,
                now,
                std::time::Duration::from_secs(30),
                2,
                Some(&decision.id),
                &[],
            )
            .unwrap();
        let started = db
            .begin_claimed_run(
                &claim.id,
                "scheduler",
                leader.generation,
                "run-claimed",
                "fake",
                "/fixture/worktree",
                "garnish/task-fixture",
                "0123456789abcdef",
                now + chrono::Duration::seconds(1),
                std::time::Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(started.route_decision_id, decision.id);
        assert_eq!(db.task(&first.id).unwrap().status, TaskStatus::Running);
        assert!(
            db.begin_claimed_run(
                &claim.id,
                "scheduler",
                leader.generation,
                "run-duplicate",
                "fake",
                "/fixture/worktree",
                "garnish/task-fixture",
                "0123456789abcdef",
                now + chrono::Duration::seconds(2),
                std::time::Duration::from_secs(30),
            )
            .is_err()
        );
        let run_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE task_id = ?1",
                [&first.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(run_count, 1);
        let action_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM scheduler_claims
                 WHERE action_key = ?1 AND status = 'consumed'",
                [&started.action_key],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(action_count, 1);

        db.transition_task(
            &first.id,
            TaskStatus::Running,
            TaskStatus::Verifying,
            "fixture",
        )
        .unwrap();
        db.finish_run("run-claimed", "review", None, 0).unwrap();
        db.claim_task_for_scheduler(
            "scheduler",
            leader.generation,
            &second.id,
            second.version,
            now + chrono::Duration::seconds(3),
            std::time::Duration::from_secs(30),
            2,
            None,
            &[],
        )
        .unwrap();
    }

    #[test]
    fn consumed_claim_recovers_orphaned_run_once_without_replaying_action() {
        let (dir, mut db) = database();
        let database_path = db.path().to_path_buf();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let task = db
            .add_task(&new_task(&project.id, "orphan", vec![]))
            .unwrap();
        let now = Utc::now();
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
            schedule: None,
            policy_hash: "fixture-policy".into(),
            created_at: now,
        };
        db.record_route(&decision).unwrap();
        db.register_scheduler_instance("scheduler", "host", 1, now)
            .unwrap();
        let leader = db
            .acquire_scheduler_leader("scheduler", now, std::time::Duration::from_secs(60))
            .unwrap();
        let claim = db
            .claim_task_for_scheduler(
                "scheduler",
                leader.generation,
                &task.id,
                task.version,
                now,
                std::time::Duration::from_secs(30),
                1,
                Some(&decision.id),
                &[],
            )
            .unwrap();
        let started = db
            .begin_claimed_run(
                &claim.id,
                "scheduler",
                leader.generation,
                "run-orphaned-claim",
                "fake",
                "/fixture/worktree",
                "garnish/task-fixture",
                "0123456789abcdef",
                now + chrono::Duration::seconds(1),
                std::time::Duration::from_secs(1),
            )
            .unwrap();
        drop(db);

        let mut reopened = Database::open(&database_path).unwrap();
        let recovery_at = now + chrono::Duration::seconds(3);
        assert_eq!(
            reopened.recover_expired_leases(recovery_at).unwrap(),
            vec![task.id.clone()]
        );
        assert_eq!(reopened.task(&task.id).unwrap().status, TaskStatus::Paused);
        assert!(
            reopened
                .recover_expired_leases(recovery_at)
                .unwrap()
                .is_empty()
        );
        let counts: (i64, i64, i64) = reopened
            .conn
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM runs WHERE task_id = ?1),
                    (SELECT COUNT(*) FROM scheduler_claims WHERE action_key = ?2),
                    (SELECT COUNT(*) FROM resource_locks WHERE claim_id = ?3 AND released_at IS NULL)",
                params![task.id, started.action_key, claim.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(counts, (1, 1, 0));
    }

    #[test]
    fn retry_backoff_is_deterministic_persisted_and_budgeted() {
        let (dir, mut db) = database();
        let database_path = db.path().to_path_buf();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let task = db
            .add_task(&new_task(&project.id, "retry", vec![]))
            .unwrap();
        db.set_retry_limit(&task.id, 2).unwrap();
        db.transition_task(&task.id, TaskStatus::Ready, TaskStatus::Leased, "fixture")
            .unwrap();
        db.transition_task(&task.id, TaskStatus::Leased, TaskStatus::Failed, "fixture")
            .unwrap();
        let now = Utc::now();
        let expected = deterministic_retry_delay(
            &task.id,
            1,
            std::time::Duration::from_secs(10),
            std::time::Duration::from_secs(100),
        )
        .unwrap();
        let first = db
            .plan_retry(
                &task.id,
                "run-1",
                FailureCategory::Infrastructure,
                now,
                std::time::Duration::from_secs(10),
                std::time::Duration::from_secs(100),
            )
            .unwrap();
        assert!(first.scheduled);
        assert_eq!(first.delay_seconds, Some(expected.as_secs()));
        let retry_at = first.retry_at.unwrap();
        drop(db);

        let mut db = Database::open(&database_path).unwrap();
        assert_eq!(
            db.retry_state(&task.id).unwrap().retry_not_before,
            Some(retry_at)
        );
        for run_number in 2..=3 {
            db.transition_task(&task.id, TaskStatus::Ready, TaskStatus::Leased, "fixture")
                .unwrap();
            db.transition_task(&task.id, TaskStatus::Leased, TaskStatus::Failed, "fixture")
                .unwrap();
            let plan = db
                .plan_retry(
                    &task.id,
                    &format!("run-{run_number}"),
                    FailureCategory::Infrastructure,
                    now + chrono::Duration::minutes(run_number),
                    std::time::Duration::from_secs(10),
                    std::time::Duration::from_secs(100),
                )
                .unwrap();
            assert_eq!(plan.scheduled, run_number == 2);
        }
        let state = db.retry_state(&task.id).unwrap();
        assert_eq!(state.retries_used, 2);
        assert!(state.retry_not_before.is_none());
        assert_eq!(db.task(&task.id).unwrap().status, TaskStatus::Failed);
    }

    #[test]
    fn adapter_circuit_opens_and_allows_only_one_half_open_probe() {
        let (_dir, mut db) = database();
        let now = Utc::now();
        for offset in 0..3 {
            db.record_adapter_outcome(
                "codex",
                "openai",
                "primary",
                Some(FailureCategory::AdapterTransient),
                now + chrono::Duration::seconds(offset),
                3,
                std::time::Duration::from_secs(60),
            )
            .unwrap();
        }
        let circuit = db.adapter_circuits().unwrap().remove(0);
        assert_eq!(circuit.state, "open");
        let probe_at = circuit.next_probe_at.unwrap();
        assert!(
            !db.adapter_circuit_gate(
                "codex",
                "openai",
                "primary",
                probe_at - chrono::Duration::seconds(1),
                true,
            )
            .unwrap()
            .0
        );
        assert!(
            db.adapter_circuit_gate("codex", "openai", "primary", probe_at, true)
                .unwrap()
                .0
        );
        assert!(
            !db.adapter_circuit_gate("codex", "openai", "primary", probe_at, true)
                .unwrap()
                .0
        );
        let closed = db
            .record_adapter_outcome(
                "codex",
                "openai",
                "primary",
                None,
                probe_at,
                3,
                std::time::Duration::from_secs(60),
            )
            .unwrap();
        assert_eq!(closed.state, "closed");
        assert_eq!(closed.consecutive_failures, 0);
    }

    #[test]
    fn checkpoints_renew_fenced_lease_and_complete_requested_cancellation() {
        let (dir, mut db) = database();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let task = db
            .add_task(&new_task(&project.id, "checkpoint", vec![]))
            .unwrap();
        let now = Utc::now();
        let decision = RouteDecision {
            id: Ulid::new().to_string(),
            task_id: task.id.clone(),
            selected_adapter: Some("fake".into()),
            allowed: true,
            reason: "fixture".into(),
            required_headroom_percent: 0.0,
            candidates: vec![],
            next_wake_at: None,
            schedule: None,
            quota: vec![],
            policy_hash: "fixture".into(),
            created_at: now,
        };
        db.record_route(&decision).unwrap();
        db.transition_task(&task.id, TaskStatus::Ready, TaskStatus::Leased, "fixture")
            .unwrap();
        db.transition_task(
            &task.id,
            TaskStatus::Leased,
            TaskStatus::Planning,
            "fixture",
        )
        .unwrap();
        db.transition_task(
            &task.id,
            TaskStatus::Planning,
            TaskStatus::Running,
            "fixture",
        )
        .unwrap();
        db.create_run(
            "run-checkpoint",
            &task.id,
            "fake",
            &decision.id,
            "/fixture/worktree",
            "fixture",
            "0123456789abcdef",
            now + chrono::Duration::seconds(60),
        )
        .unwrap();
        let first = db
            .apply_run_checkpoint(
                "run-checkpoint",
                "local",
                1,
                now + chrono::Duration::seconds(1),
                std::time::Duration::from_secs(60),
                CheckpointAction::Continue,
                "supervision.healthy",
                Some(now + chrono::Duration::seconds(61)),
                &serde_json::json!({"fixture": true}),
            )
            .unwrap();
        assert_eq!(first.sequence, 1);
        assert!(
            db.request_run_cancellation(
                "run-checkpoint",
                "user requested",
                now + chrono::Duration::seconds(2),
            )
            .unwrap()
        );
        let cancelled = db
            .apply_run_checkpoint(
                "run-checkpoint",
                "local",
                1,
                now + chrono::Duration::seconds(3),
                std::time::Duration::from_secs(60),
                CheckpointAction::Cancel,
                "cancel.requested",
                None,
                &serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(cancelled.sequence, 2);
        assert_eq!(db.task(&task.id).unwrap().status, TaskStatus::Running);
        assert!(db.run_lease_context("run-checkpoint").is_ok());
        db.record_process_outcome(
            "run-checkpoint",
            Some(FailureCategory::Cancelled),
            None,
            &serde_json::json!({"classification": "cancelled"}),
            Some(&serde_json::json!({"term_sent": true, "kill_sent": false})),
            now + chrono::Duration::seconds(4),
        )
        .unwrap();
        assert_eq!(db.task(&task.id).unwrap().status, TaskStatus::Cancelled);
        assert!(db.run_lease_context("run-checkpoint").is_err());
    }

    #[test]
    fn emergency_stop_cancels_running_work_and_releases_unstarted_claims() {
        let (dir, mut db) = database();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let project = db.add_project("one", "One", &root).unwrap();
        let running = db
            .add_task(&new_task(&project.id, "running", vec![]))
            .unwrap();
        let queued = db
            .add_task(&new_task(&project.id, "queued", vec![]))
            .unwrap();
        let now = Utc::now();
        let decision = RouteDecision {
            id: Ulid::new().to_string(),
            task_id: running.id.clone(),
            selected_adapter: Some("fake".into()),
            allowed: true,
            reason: "fixture".into(),
            required_headroom_percent: 0.0,
            candidates: vec![],
            next_wake_at: None,
            schedule: None,
            quota: vec![],
            policy_hash: "fixture".into(),
            created_at: now,
        };
        db.record_route(&decision).unwrap();
        db.transition_task(
            &running.id,
            TaskStatus::Ready,
            TaskStatus::Leased,
            "fixture",
        )
        .unwrap();
        db.transition_task(
            &running.id,
            TaskStatus::Leased,
            TaskStatus::Planning,
            "fixture",
        )
        .unwrap();
        db.transition_task(
            &running.id,
            TaskStatus::Planning,
            TaskStatus::Running,
            "fixture",
        )
        .unwrap();
        db.create_run(
            "run-emergency",
            &running.id,
            "fake",
            &decision.id,
            "/fixture/worktree",
            "fixture",
            "0123456789abcdef",
            now + chrono::Duration::minutes(5),
        )
        .unwrap();
        db.register_scheduler_instance("scheduler", "host", 1, now)
            .unwrap();
        let leader = db
            .acquire_scheduler_leader("scheduler", now, std::time::Duration::from_secs(60))
            .unwrap();
        db.claim_task_for_scheduler(
            "scheduler",
            leader.generation,
            &queued.id,
            queued.version,
            now,
            std::time::Duration::from_secs(60),
            2,
            None,
            &[],
        )
        .unwrap();

        let stopped = db.emergency_stop("fixture emergency", now).unwrap();
        assert!(stopped.control.pause_new_work);
        assert!(stopped.control.emergency_stop);
        assert_eq!(
            stopped.cancellation_requested_run_ids,
            vec!["run-emergency"]
        );
        assert_eq!(stopped.released_task_ids, vec![queued.id.clone()]);
        assert!(db.run_cancellation_requested("run-emergency").unwrap());
        assert_eq!(db.task(&running.id).unwrap().status, TaskStatus::Running);
        assert_eq!(db.task(&queued.id).unwrap().status, TaskStatus::Paused);
        assert!(
            db.claim_task_for_scheduler(
                "scheduler",
                leader.generation,
                &queued.id,
                db.task(&queued.id).unwrap().version,
                now + chrono::Duration::seconds(1),
                std::time::Duration::from_secs(60),
                2,
                None,
                &[],
            )
            .unwrap_err()
            .to_string()
            .contains("emergency_stop")
        );
        let resumed = db
            .resume_operations("incident resolved", now + chrono::Duration::seconds(2))
            .unwrap();
        assert!(!resumed.pause_new_work);
        assert!(!resumed.emergency_stop);
    }

    #[test]
    fn durable_notifications_are_bounded_and_single_acknowledgement() {
        let (_dir, mut db) = database();
        let notification = db
            .enqueue_notification(
                "operation",
                "info",
                None,
                None,
                "Fixture notice",
                "Bounded local notification body",
                Utc::now(),
            )
            .unwrap();
        assert_eq!(db.local_notifications(false, 10).unwrap().len(), 1);
        let acknowledged = db
            .acknowledge_notification(&notification.id, Utc::now())
            .unwrap();
        assert!(acknowledged.acknowledged_at.is_some());
        assert!(db.local_notifications(false, 10).unwrap().is_empty());
        assert!(
            db.acknowledge_notification(&notification.id, Utc::now())
                .is_err()
        );
        assert!(
            db.enqueue_notification(
                "operation",
                "info",
                None,
                None,
                "Fixture notice",
                &"x".repeat(2_001),
                Utc::now(),
            )
            .is_err()
        );
    }

    #[test]
    fn online_backup_is_private_integrity_checked_and_content_addressed() {
        let (dir, mut db) = database();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        db.add_project("one", "One", &root).unwrap();
        let destination = dir.path().join("backups/state.db");
        let backup = db.backup_to(&destination, Utc::now()).unwrap();
        assert_eq!(backup.integrity, "ok");
        assert_eq!(backup.schema_version, 6);
        assert_eq!(backup.sha256.len(), 64);
        assert_eq!(backup.size_bytes, fs::metadata(&destination).unwrap().len());
        assert!(db.backup_to(&destination, Utc::now()).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(destination).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
