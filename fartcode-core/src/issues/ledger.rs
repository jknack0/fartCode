//! Step spend ledger (#82) — the durable record of agent-step launches.
//!
//! One row per real launch (`kind = launch`) and one per chain-guard hold
//! (`kind = hold`). Written by the step engine: launches at launch time
//! (with `auto` marking settle-chained launches that no human confirmed),
//! token usage backfilled at settle from the provider-reported
//! context-window numbers (the only usage metadata that reaches this
//! process — see `fartcode-telemetry`'s tokens module), and hold rows when
//! the chain guard refuses the *next* launch (depth cap, cycle, budget).
//!
//! Read by the card detail (per-issue history) and by the budget guard
//! (per-project token total). Rows ride the issue/project FK cascades —
//! deleting either sweeps its ledger.

use std::sync::Arc;

use serde::Serialize;

use crate::db::Db;
use crate::Error;

/// Why the chain guard held instead of launching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldReason {
    /// The per-card chain depth cap was reached: N consecutive automatic
    /// launches with no human confirm.
    Depth,
    /// The advance target was already run for this card in the current
    /// chain — relaunching would loop.
    Cycle,
    /// The project's token budget is exhausted.
    Budget,
}

impl HoldReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            HoldReason::Depth => "depth",
            HoldReason::Cycle => "cycle",
            HoldReason::Budget => "budget",
        }
    }
}

/// One ledger row, as the card detail reads it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    pub id: String,
    pub issue_id: String,
    pub project_id: String,
    pub column_id: String,
    /// `"launch"` | `"hold"`.
    pub kind: String,
    /// `true` = settle-chained (no human confirm preceded this launch).
    pub auto: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    /// Hold reason (`depth` | `cycle` | `budget`); `None` on launches.
    pub reason: Option<String>,
    /// The refused launch target; `None` on launches.
    pub target_column_id: Option<String>,
    /// Provider-reported context usage, backfilled at settle when the
    /// session reported any.
    pub tokens_used: Option<i64>,
    pub created_at: String,
}

pub struct StepLedgerStore {
    db: Arc<dyn Db>,
}

const COLUMNS: &str = "id, issue_id, project_id, column_id, kind, auto, provider, model, \
                       reason, target_column_id, tokens_used, created_at";

fn entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LedgerEntry> {
    Ok(LedgerEntry {
        id: row.get(0)?,
        issue_id: row.get(1)?,
        project_id: row.get(2)?,
        column_id: row.get(3)?,
        kind: row.get(4)?,
        auto: row.get::<_, i64>(5)? != 0,
        provider: row.get(6)?,
        model: row.get(7)?,
        reason: row.get(8)?,
        target_column_id: row.get(9)?,
        tokens_used: row.get(10)?,
        created_at: row.get(11)?,
    })
}

impl StepLedgerStore {
    pub fn new(db: Arc<dyn Db>) -> Self {
        Self { db }
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, Error> {
        self.db
            .conn()
            .lock()
            .map_err(|e| Error::Internal(format!("db mutex poisoned: {e}")))
    }

    /// Records one real launch. `auto` marks a settle-chained launch (no
    /// human confirm) — the chain guard and the card detail both key on it.
    pub fn record_launch(
        &self,
        issue_id: &str,
        project_id: &str,
        column_id: &str,
        auto: bool,
        provider: &str,
        model: Option<&str>,
    ) -> Result<LedgerEntry, Error> {
        let id = format!("led_{}", uuid::Uuid::new_v4());
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO step_ledger (id, issue_id, project_id, column_id, kind, auto, provider, model)
             VALUES (?1, ?2, ?3, ?4, 'launch', ?5, ?6, ?7)",
            rusqlite::params![id, issue_id, project_id, column_id, auto as i64, provider, model],
        )?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM step_ledger WHERE id = ?1"),
            [&id],
            entry_from_row,
        )
        .map_err(Error::from)
    }

    /// Records one chain-guard hold: the launch of `target_column_id` was
    /// refused while the card sat on `column_id`.
    pub fn record_hold(
        &self,
        issue_id: &str,
        project_id: &str,
        column_id: &str,
        reason: HoldReason,
        target_column_id: Option<&str>,
    ) -> Result<LedgerEntry, Error> {
        let id = format!("led_{}", uuid::Uuid::new_v4());
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO step_ledger
                 (id, issue_id, project_id, column_id, kind, auto, reason, target_column_id)
             VALUES (?1, ?2, ?3, ?4, 'hold', 1, ?5, ?6)",
            rusqlite::params![
                id,
                issue_id,
                project_id,
                column_id,
                reason.as_str(),
                target_column_id
            ],
        )?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM step_ledger WHERE id = ?1"),
            [&id],
            entry_from_row,
        )
        .map_err(Error::from)
    }

    /// The issue's full ledger, oldest first — the card detail's history.
    pub fn list_for_issue(&self, issue_id: &str) -> Result<Vec<LedgerEntry>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM step_ledger WHERE issue_id = ?1 ORDER BY rowid"
        ))?;
        let rows = stmt
            .query_map([issue_id], entry_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Total provider-reported tokens across the project's ledger — what
    /// the budget guard compares against. Rows whose sessions reported
    /// nothing contribute nothing (the budget is a floor on known spend,
    /// not a claim of completeness).
    pub fn project_tokens_total(&self, project_id: &str) -> Result<i64, Error> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT COALESCE(SUM(tokens_used), 0) FROM step_ledger WHERE project_id = ?1",
            [project_id],
            |row| row.get(0),
        )
        .map_err(Error::from)
    }

    /// Backfills token usage onto the NEWEST launch row for
    /// (`issue_id`, `column_id`) that has none yet — called at settle,
    /// when the provider-reported numbers first exist. A settle with no
    /// matching row (restart heuristic, legacy dispatch) is a no-op.
    pub fn record_tokens(
        &self,
        issue_id: &str,
        column_id: &str,
        tokens_used: i64,
    ) -> Result<(), Error> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE step_ledger SET tokens_used = ?3
             WHERE id = (SELECT id FROM step_ledger
                         WHERE issue_id = ?1 AND column_id = ?2
                           AND kind = 'launch' AND tokens_used IS NULL
                         ORDER BY rowid DESC LIMIT 1)",
            rusqlite::params![issue_id, column_id, tokens_used],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;

    fn fixture() -> StepLedgerStore {
        let db = SqliteDb::init_in_memory().unwrap();
        db.conn()
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'proj', '/tmp/proj');
                 INSERT INTO board_columns (id, project_id, name) VALUES ('c1', 'p1', 'A');
                 INSERT INTO issues (id, project_id, title, lane, position, column_id)
                 VALUES ('i1', 'p1', 'card', 'ready', 0, 'c1');",
            )
            .unwrap();
        StepLedgerStore::new(db)
    }

    #[test]
    fn launches_holds_and_token_backfill_round_trip() {
        let store = fixture();
        store
            .record_launch("i1", "p1", "c1", false, "claude", Some("opus"))
            .unwrap();
        store
            .record_launch("i1", "p1", "c1", true, "claude", None)
            .unwrap();
        store
            .record_hold("i1", "p1", "c1", HoldReason::Depth, Some("c2"))
            .unwrap();

        let rows = store.list_for_issue("i1").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, "launch");
        assert!(!rows[0].auto);
        assert_eq!(rows[0].model.as_deref(), Some("opus"));
        assert!(rows[1].auto);
        assert_eq!(rows[2].kind, "hold");
        assert_eq!(rows[2].reason.as_deref(), Some("depth"));
        assert_eq!(rows[2].target_column_id.as_deref(), Some("c2"));

        // Backfill lands on the NEWEST tokenless launch row, not the hold.
        store.record_tokens("i1", "c1", 500).unwrap();
        let rows = store.list_for_issue("i1").unwrap();
        assert_eq!(rows[1].tokens_used, Some(500));
        assert_eq!(rows[0].tokens_used, None);
        assert_eq!(rows[2].tokens_used, None);

        // A second backfill fills the OLDER still-empty row.
        store.record_tokens("i1", "c1", 200).unwrap();
        let rows = store.list_for_issue("i1").unwrap();
        assert_eq!(rows[0].tokens_used, Some(200));

        assert_eq!(store.project_tokens_total("p1").unwrap(), 700);
        assert_eq!(store.project_tokens_total("ghost").unwrap(), 0);

        // No matching row → no-op, never an error.
        store.record_tokens("i1", "c1", 999).unwrap();
        assert_eq!(store.project_tokens_total("p1").unwrap(), 700);
    }

    #[test]
    fn issue_delete_cascades_the_ledger() {
        let store = fixture();
        store
            .record_launch("i1", "p1", "c1", false, "claude", None)
            .unwrap();
        store
            .db
            .conn()
            .lock()
            .unwrap()
            .execute("DELETE FROM issues WHERE id = 'i1'", [])
            .unwrap();
        assert!(store.list_for_issue("i1").unwrap().is_empty());
    }
}
