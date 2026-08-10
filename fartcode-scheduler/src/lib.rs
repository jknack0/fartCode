//! Cron scheduler core — E11-01.
//!
//! Manages single-upcoming-run invariant, restart recovery, and due-run
//! transitions. Externally driven (caller owns the timer).
//!
//! # Invariants
//! - Each enabled automation has at most one cron run in `scheduled` or `queued`.
//! - Stuck intermediate runs at startup are marked `failed`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use croner::Cron;
use rusqlite::Connection;
use serde_json::Value;
use tracing::warn;

use fartcode_core::db::Db;
use fartcode_core::tasks::naming::generate_random_name;
use fartcode_core::Error;

/// A cron-based scheduler for automations.
///
/// Externally driven: call `start()` at boot for recovery + bootstrap, then
/// call `tick()` periodically (every ~60s).
pub struct Scheduler {
    db: Arc<dyn Db>,
}

impl Scheduler {
    pub fn new(db: Arc<dyn Db>) -> Self {
        Self { db }
    }

    /// Recover interrupted runs and bootstrap due runs. Call once at startup.
    pub fn start(&self) -> Result<(), Error> {
        self.recover_interrupted_runs()?;
        self.bootstrap()?;
        Ok(())
    }

    /// Periodic tick. Call every ~60s.
    pub fn tick(&self) -> Result<(), Error> {
        let now = Utc::now().timestamp_millis();
        self.transition_due_runs(now)?;
        Ok(())
    }

    // ── Restart recovery ──────────────────────────────────────────────

    /// Mark runs stuck in intermediate states as `failed` (crash recovery).
    fn recover_interrupted_runs(&self) -> Result<(), Error> {
        let conn = lock_db(&self.db)?;
        let now = Utc::now().timestamp_millis();
        let error = serde_json::json!({"step": "startup", "code": "interrupted_by_restart"});
        let error_str = error.to_string();

        for state in &["creating_task", "launching_task", "creating_conversation"] {
            let affected = conn.execute(
                "UPDATE automation_runs
                 SET status = 'failed', error = ?1, finished_at = ?2
                 WHERE status = ?3 AND (started_at IS NULL OR started_at < ?2 - 300000)",
                rusqlite::params![error_str, now, state],
            )?;
            if affected > 0 {
                warn!(count = affected, state = %state, "recovered stuck automation runs");
            }
        }
        Ok(())
    }

    // ── Bootstrap ─────────────────────────────────────────────────────

    /// Ensure every enabled automation has a scheduled cron run, then
    /// transition any due runs to queued.
    fn bootstrap(&self) -> Result<(), Error> {
        let now = Utc::now().timestamp_millis();
        let conn = lock_db(&self.db)?;
        self.ensure_pending_runs(&conn, now)?;
        self.transition_due_runs_inner(&conn, now)
    }

    /// Insert a scheduled run for every enabled automation that lacks one.
    fn ensure_pending_runs(&self, conn: &Connection, now: i64) -> Result<(), Error> {
        let mut stmt = conn.prepare(
            "SELECT a.id, a.trigger_config, a.conversation_config, a.task_config
             FROM automations a
             WHERE a.enabled = 1
               AND a.deleted_at IS NULL
               AND a.project_id IS NOT NULL
               AND a.trigger_config IS NOT NULL
               AND NOT EXISTS (
                 SELECT 1 FROM automation_runs r
                 WHERE r.automation_id = a.id
                   AND r.trigger_kind = 'cron'
                   AND r.status IN ('scheduled', 'queued')
               )",
        )?;

        let rows: Vec<(String, String, Option<String>, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (id, trigger_json, conv_json, task_json) in rows {
            let trigger: Value = serde_json::from_str(&trigger_json)?;
            let expr = trigger
                .get("expr")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if expr.is_empty() {
                continue;
            }
            if let Some(scheduled_at) = parse_next_cron_run(expr, now) {
                let deadline = parse_next_cron_run(expr, scheduled_at + 1);
                let run_id = make_id();
                let name = generate_random_name();
                let conv_snap = conv_json.unwrap_or_else(|| "{}".into());
                let task_snap = task_json;

                conn.execute(
                    "INSERT INTO automation_runs
                         (id, automation_id, scheduled_at, deadline_at, status,
                          trigger_kind, trigger_config_snapshot,
                          conversation_config_snapshot, task_config_snapshot,
                          generated_task_name)
                     VALUES (?1, ?2, ?3, ?4, 'scheduled', 'cron',
                             ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        run_id,
                        id,
                        scheduled_at,
                        deadline,
                        trigger_json,
                        conv_snap,
                        task_snap,
                        name,
                    ],
                )?;
            }
        }
        Ok(())
    }

    // ── Tick: transition due runs ─────────────────────────────────────

    /// Transition due scheduled cron runs to `queued`, scheduling the next
    /// occurrence for each.
    fn transition_due_runs(&self, now: i64) -> Result<(), Error> {
        let conn = lock_db(&self.db)?;
        self.transition_due_runs_inner(&conn, now)
    }

    fn transition_due_runs_inner(&self, conn: &Connection, now: i64) -> Result<(), Error> {
        let mut stmt = conn.prepare(
            "SELECT r.id, a.id, a.trigger_config
             FROM automation_runs r
             JOIN automations a ON a.id = r.automation_id
             WHERE r.status = 'scheduled'
               AND r.trigger_kind = 'cron'
               AND r.scheduled_at <= ?1
               AND a.enabled = 1
               AND a.project_id IS NOT NULL
               AND a.deleted_at IS NULL
             ORDER BY r.scheduled_at ASC
             LIMIT 100",
        )?;

        let due: Vec<(String, String, String)> = stmt
            .query_map([now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();

        if due.is_empty() {
            return Ok(());
        }

        for (run_id, auto_id, trigger_json) in &due {
            conn.execute(
                "UPDATE automation_runs SET status = 'queued' WHERE id = ?1 AND status = 'scheduled'",
                [run_id],
            )?;

            let trigger: Value = serde_json::from_str(trigger_json)?;
            let expr = trigger
                .get("expr")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if expr.is_empty() {
                continue;
            }
            if let Some(next_scheduled) = parse_next_cron_run(expr, now) {
                let auto = conn.query_row(
                    "SELECT id, trigger_config, conversation_config, task_config
                     FROM automations WHERE id = ?1",
                    [auto_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                );

                if let Ok((_id, _trig, conv_json, task_json)) = auto {
                    let deadline = parse_next_cron_run(expr, next_scheduled + 1);
                    let run_id = make_id();
                    let name = generate_random_name();
                    let conv_snap = conv_json.unwrap_or_else(|| "{}".into());

                    conn.execute(
                        "INSERT INTO automation_runs
                             (id, automation_id, scheduled_at, deadline_at, status,
                              trigger_kind, trigger_config_snapshot,
                              conversation_config_snapshot, task_config_snapshot,
                              generated_task_name)
                         SELECT ?1, ?2, ?3, ?4, 'scheduled', 'cron',
                                ?5, ?6, ?7, ?8
                         WHERE NOT EXISTS (
                           SELECT 1 FROM automation_runs r
                           WHERE r.automation_id = ?2
                             AND r.scheduled_at = ?3
                             AND r.trigger_kind = 'cron'
                             AND r.status IN ('scheduled', 'queued')
                         )",
                        rusqlite::params![
                            run_id,
                            auto_id,
                            next_scheduled,
                            deadline,
                            trigger_json,
                            conv_snap,
                            task_json,
                            name,
                        ],
                    )?;
                }
            }
        }
        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn lock_db(db: &Arc<dyn Db>) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
    db.conn()
        .lock()
        .map_err(|_| Error::Internal("db mutex poisoned".into()))
}

/// Parse a cron expression and compute the next run timestamp from `from_ms`.
fn parse_next_cron_run(expr: &str, from_ms: i64) -> Option<i64> {
    let cron = Cron::new(expr).parse().ok()?;
    let from = DateTime::from_timestamp_millis(from_ms)?;
    let next = cron.find_next_occurrence(&from, false).ok()?;
    Some(next.timestamp_millis())
}

/// Generate a simple unique ID (timestamp + hash).
fn make_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let ts = now.as_nanos();
    let rand = (ts.wrapping_mul(6364136223846793005) ^ (ts >> 32)) & 0xFFFF_FFFF;
    format!("s{:016x}{:08x}", ts, rand)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::db::SqliteDb;

    fn setup_db() -> Arc<dyn Db> {
        SqliteDb::init_in_memory().expect("in-memory db")
    }

    fn insert_automation(
        conn: &Connection,
        id: &str,
        enabled: bool,
        expr: &str,
    ) -> Result<(), Error> {
        let project_id = "p1";
        conn.execute(
            "INSERT OR IGNORE INTO projects (id, name, path) VALUES (?1, ?2, ?3)",
            rusqlite::params![project_id, "test-project", "/tmp/test-project"],
        )?;
        let trigger = serde_json::json!({"expr": expr});
        conn.execute(
            "INSERT INTO automations (id, name, project_id, trigger_config, conversation_config, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1000, 1000)",
            rusqlite::params![
                id,
                "test-auto",
                project_id,
                trigger.to_string(),
                r#"{"prompt":"test","provider":"claude","autoApprove":false}"#,
                enabled as i64,
            ],
        )?;
        Ok(())
    }

    #[test]
    fn recover_marks_stuck_runs_failed() {
        let db = setup_db();
        let conn = lock_db(&db).unwrap();
        insert_automation(&conn, "a1", true, "*/5 * * * *").unwrap();

        for (i, state) in ["creating_task", "launching_task", "creating_conversation"]
            .iter()
            .enumerate()
        {
            conn.execute(
                "INSERT INTO automation_runs (id, automation_id, status, trigger_kind, trigger_config_snapshot, conversation_config_snapshot, started_at)
                 VALUES (?1, 'a1', ?2, 'cron', '{}', '{}', ?3)",
                rusqlite::params![format!("r{}", i), state, 1000],
            ).unwrap();
        }
        drop(conn);

        let sched = Scheduler::new(db.clone());
        sched.recover_interrupted_runs().unwrap();

        let conn = lock_db(&db).unwrap();
        let failed_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs WHERE status = 'failed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(failed_count, 3, "all 3 stuck runs should be failed");

        let error: String = conn
            .query_row(
                "SELECT error FROM automation_runs WHERE status = 'failed' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            error.contains("interrupted_by_restart"),
            "error should mention restart: {error}"
        );
    }

    #[test]
    fn bootstrap_creates_scheduled_run_for_enabled_automation() {
        let db = setup_db();
        let conn = lock_db(&db).unwrap();
        insert_automation(&conn, "a1", true, "0 0 * * *").unwrap();
        drop(conn);

        let sched = Scheduler::new(db.clone());
        sched.bootstrap().unwrap();

        let conn = lock_db(&db).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs WHERE status = 'scheduled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "bootstrap should create one scheduled run");

        drop(conn);
        sched.bootstrap().unwrap();
        let conn = lock_db(&db).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs WHERE status = 'scheduled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "bootstrap must not duplicate scheduled runs");
    }

    #[test]
    fn disabled_automation_gets_no_scheduled_run() {
        let db = setup_db();
        let conn = lock_db(&db).unwrap();
        insert_automation(&conn, "a1", false, "0 0 * * *").unwrap();
        drop(conn);

        let sched = Scheduler::new(db.clone());
        sched.bootstrap().unwrap();

        let conn = lock_db(&db).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs WHERE status = 'scheduled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "disabled automation should have no runs");
    }

    #[test]
    fn transition_due_run_queues_and_schedules_next() {
        let db = setup_db();
        let conn = lock_db(&db).unwrap();
        insert_automation(&conn, "a1", true, "*/5 * * * *").unwrap();
        drop(conn);

        let sched = Scheduler::new(db.clone());
        sched.bootstrap().unwrap();

        let conn = lock_db(&db).unwrap();
        conn.execute(
            "UPDATE automation_runs SET scheduled_at = 1000 WHERE status = 'scheduled'",
            [],
        )
        .unwrap();
        drop(conn);

        sched.tick().unwrap();

        let conn = lock_db(&db).unwrap();
        let queued: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs WHERE status = 'queued'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(queued, 1, "due run should be queued");

        let scheduled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs WHERE status = 'scheduled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scheduled, 1, "next occurrence should be scheduled");
    }

    #[test]
    fn start_runs_recovery_and_bootstrap() {
        let db = setup_db();
        let conn = lock_db(&db).unwrap();
        insert_automation(&conn, "a1", true, "0 0 * * *").unwrap();
        conn.execute(
            "INSERT INTO automation_runs (id, automation_id, status, trigger_kind, trigger_config_snapshot, conversation_config_snapshot, started_at)
             VALUES ('stuck', 'a1', 'creating_task', 'cron', '{}', '{}', 1000)",
            [],
        )
        .unwrap();
        drop(conn);

        let sched = Scheduler::new(db.clone());
        sched.start().unwrap();

        let conn = lock_db(&db).unwrap();
        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs WHERE status = 'failed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(failed, 1, "stuck run should be recovered");

        let scheduled: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM automation_runs WHERE status = 'scheduled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(scheduled, 1, "should have scheduled run after start");
    }
}
