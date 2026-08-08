//! PR sync storage (E4-09, #49): idempotent upserts of fetched PR payloads
//! into `pull_requests`, kv-backed sync cursors, and the cached reads the
//! PR tab and the commit-card PR-open guard consume.
//!
//! Design: one row per PR URL; scalar columns for query paths, the full
//! denormalized `PrDto` (files/commits/checks/comments) in the versioned
//! `data` column (decisions/0036 — JSON sub-collections instead of four
//! normalized sub-tables).

use std::sync::Arc;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::db::versioned_json::parse_versioned;
use crate::db::Db;
use crate::events::{EventBus, InternalEvent};
use crate::github::PrDto;
use crate::Error;

/// Versioned-JSON wrapper so `data` cells are self-describing (§2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StoredPr(pub PrDto);

impl crate::db::versioned_json::Versioned for StoredPr {
    const VERSION: u32 = 1;
}

pub struct PrSyncStore {
    db: Arc<dyn Db>,
    event_bus: Arc<dyn EventBus>,
}

/// Cached PR row (scalar columns only — the full DTO comes via
/// [`PrSyncStore::get`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CachedPrRow {
    pub url: String,
    pub workspace_id: Option<String>,
    pub owner: String,
    pub repo: String,
    pub number: i64,
    pub status: String,
    pub head_ref: String,
    pub head_oid: Option<String>,
}

impl PrSyncStore {
    pub fn new(db: Arc<dyn Db>, event_bus: Arc<dyn EventBus>) -> Self {
        Self { db, event_bus }
    }

    /// Idempotent upsert keyed by `dto.url`. Returns `true` when the row
    /// changed (write + `PrUpdated` emitted); `false` when the payload is
    /// byte-identical to the stored one — no row churn, no event.
    ///
    /// `workspace_id` attaches the row to the workspace that synced it (the
    /// PR tab's query scope); existing attachments are preserved when the
    /// caller passes `None`.
    pub fn upsert(
        &self,
        workspace_id: Option<&str>,
        owner: &str,
        repo: &str,
        dto: &PrDto,
    ) -> Result<bool, Error> {
        let conn = self.conn()?;
        let existing: Option<(Option<String>, String)> = conn
            .query_row(
                "SELECT workspace_id, data FROM pull_requests WHERE url = ?1",
                [&dto.url],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((existing_ws, data)) = &existing {
            let unchanged = parse_versioned::<StoredPr>("data", Some(data))
                .map(|stored| stored.0 == *dto)
                .unwrap_or(false);
            let ws_same = workspace_id.is_none() || existing_ws.as_deref() == workspace_id;
            if unchanged && ws_same {
                return Ok(false);
            }
        }

        let data = crate::db::versioned_json::serialize_versioned(&StoredPr(dto.clone()))?;
        let effective_ws = workspace_id
            .map(str::to_string)
            .or_else(|| existing.and_then(|(ws, _)| ws));
        conn.execute(
            "INSERT INTO pull_requests
                 (url, workspace_id, owner, repo, number, title, status, draft,
                  base_ref, head_ref, head_oid, synced_at, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'), ?12)
             ON CONFLICT(url) DO UPDATE SET
                 workspace_id = COALESCE(?2, workspace_id),
                 title = excluded.title,
                 status = excluded.status,
                 draft = excluded.draft,
                 base_ref = excluded.base_ref,
                 head_ref = excluded.head_ref,
                 head_oid = excluded.head_oid,
                 synced_at = excluded.synced_at,
                 data = excluded.data",
            rusqlite::params![
                dto.url,
                effective_ws,
                owner,
                repo,
                dto.number as i64,
                dto.title,
                dto.status.as_str(),
                dto.draft as i64,
                dto.base_ref,
                dto.head_ref,
                dto.head_oid,
                data,
            ],
        )?;
        drop(conn);

        if let Some(ws) = workspace_id {
            self.event_bus.send(InternalEvent::PrUpdated {
                workspace_id: ws.to_string(),
                pr_url: dto.url.clone(),
            });
        }
        Ok(true)
    }

    /// The cached PRs of a workspace — open first, then newest. This is the
    /// PR tab's read path: instant, offline, restart-safe.
    pub fn list_for_workspace(&self, workspace_id: &str) -> Result<Vec<PrDto>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT data FROM pull_requests WHERE workspace_id = ?1
              ORDER BY CASE status WHEN 'open' THEN 0 ELSE 1 END, synced_at DESC",
        )?;
        let rows = stmt.query_map([workspace_id], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for cell in rows {
            let cell = cell?;
            if let Some(stored) = parse_versioned::<StoredPr>("data", Some(&cell)) {
                out.push(stored.0);
            }
        }
        Ok(out)
    }

    /// The open PR for a branch — the commit card's PR-open guard
    /// (`CachedPrLookup` wraps this). Local-only, so it works offline.
    pub fn open_pr_url(
        &self,
        owner: &str,
        repo: &str,
        head_ref: &str,
    ) -> Result<Option<String>, Error> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT url FROM pull_requests
              WHERE owner = ?1 AND repo = ?2 AND head_ref = ?3 AND status = 'open'
              LIMIT 1",
            rusqlite::params![owner, repo, head_ref],
            |row| row.get(0),
        )
        .optional()
        .map_err(Error::from)
    }

    // -- sync cursors (kv) ---------------------------------------------------

    /// Epoch seconds of the last successful sync of `workspace_id`.
    pub fn last_sync(&self, workspace_id: &str) -> Result<Option<i64>, Error> {
        let key = cursor_key(workspace_id);
        let conn = self.conn()?;
        crate::db::connection::kv_get_raw(&conn, &key)
            .map(|v| v.and_then(|s| s.parse::<i64>().ok()))
    }

    /// Consecutive sync failures of `workspace_id` (drives backoff).
    pub fn failure_count(&self, workspace_id: &str) -> Result<u32, Error> {
        let key = failures_key(workspace_id);
        let conn = self.conn()?;
        Ok(crate::db::connection::kv_get_raw(&conn, &key)?
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0))
    }

    pub fn record_success(&self, workspace_id: &str) -> Result<(), Error> {
        let conn = self.conn()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default();
        crate::db::connection::kv_set_raw(&conn, &cursor_key(workspace_id), &now)?;
        crate::db::connection::kv_set_raw(&conn, &failures_key(workspace_id), "0")
    }

    pub fn record_failure(&self, workspace_id: &str) -> Result<u32, Error> {
        let count = self.failure_count(workspace_id)?.saturating_add(1);
        let conn = self.conn()?;
        crate::db::connection::kv_set_raw(&conn, &failures_key(workspace_id), &count.to_string())?;
        Ok(count)
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, Error> {
        self.db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))
    }
}

fn cursor_key(workspace_id: &str) -> String {
    format!("pr_sync:last:{workspace_id}")
}

fn failures_key(workspace_id: &str) -> String {
    format!("pr_sync:failures:{workspace_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;
    use crate::events::BroadcastEventBus;
    use crate::github::{PrStatus, PrUserDto};

    fn fixture() -> (Arc<PrSyncStore>, Arc<BroadcastEventBus>) {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        let store = Arc::new(PrSyncStore::new(db, bus.clone() as Arc<dyn EventBus>));
        (store, bus)
    }

    fn pr_dto(url: &str, title: &str, status: PrStatus) -> PrDto {
        PrDto {
            number: 42,
            title: title.into(),
            url: url.into(),
            status,
            draft: false,
            author: Some(PrUserDto {
                login: "jknack0".into(),
                avatar_url: None,
                url: None,
            }),
            base_ref: "main".into(),
            head_ref: "feat/x".into(),
            head_oid: "h1".into(),
            additions: 1,
            deletions: 2,
            changed_files: 3,
            commit_count: 1,
            mergeable_state: Some("clean".into()),
            review_decision: None,
            created_at: None,
            updated_at: None,
            files: vec![],
            commits: vec![],
            checks: vec![],
            comments: vec![],
        }
    }

    #[test]
    fn upsert_is_idempotent_for_identical_payloads() {
        let (store, bus) = fixture();
        let mut rx = bus.subscribe();
        let dto = pr_dto("https://github.com/o/r/pull/42", "t", PrStatus::Open);

        assert!(store.upsert(Some("w1"), "o", "r", &dto).unwrap());
        // Same payload twice → no churn, no event.
        assert!(!store.upsert(Some("w1"), "o", "r", &dto).unwrap());
        let _ = rx.try_recv(); // first upsert's event
        assert!(rx.try_recv().is_err(), "no second PrUpdated");
    }

    #[test]
    fn upsert_emits_on_change() {
        let (store, bus) = fixture();
        let mut rx = bus.subscribe();
        let dto = pr_dto("https://github.com/o/r/pull/42", "t", PrStatus::Open);
        store.upsert(Some("w1"), "o", "r", &dto).unwrap();
        let _ = rx.try_recv();

        let changed = pr_dto("https://github.com/o/r/pull/42", "t2", PrStatus::Open);
        assert!(store.upsert(Some("w1"), "o", "r", &changed).unwrap());
        match rx.try_recv().unwrap() {
            InternalEvent::PrUpdated {
                workspace_id,
                pr_url,
            } => {
                assert_eq!(workspace_id, "w1");
                assert_eq!(pr_url, changed.url);
            }
            other => panic!("expected PrUpdated, got {other:?}"),
        }

        let listed = store.list_for_workspace("w1").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "t2");
    }

    #[test]
    fn open_pr_url_guards_by_branch_and_status() {
        let (store, _bus) = fixture();
        let open = pr_dto("https://github.com/o/r/pull/42", "t", PrStatus::Open);
        store.upsert(Some("w1"), "o", "r", &open).unwrap();
        assert_eq!(
            store.open_pr_url("o", "r", "feat/x").unwrap().as_deref(),
            Some("https://github.com/o/r/pull/42")
        );
        assert!(store.open_pr_url("o", "r", "other").unwrap().is_none());
        assert!(store.open_pr_url("o2", "r", "feat/x").unwrap().is_none());

        // Merge closes the guard.
        let merged = pr_dto("https://github.com/o/r/pull/42", "t", PrStatus::Merged);
        store.upsert(Some("w1"), "o", "r", &merged).unwrap();
        assert!(store.open_pr_url("o", "r", "feat/x").unwrap().is_none());
    }

    #[test]
    fn list_orders_open_first() {
        let (store, _bus) = fixture();
        let merged = pr_dto("https://github.com/o/r/pull/40", "old", PrStatus::Merged);
        let open = pr_dto("https://github.com/o/r/pull/42", "new", PrStatus::Open);
        store.upsert(Some("w1"), "o", "r", &merged).unwrap();
        store.upsert(Some("w1"), "o", "r", &open).unwrap();

        let listed = store.list_for_workspace("w1").unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].status, PrStatus::Open);
        assert_eq!(listed[1].status, PrStatus::Merged);
        assert!(store.list_for_workspace("w2").unwrap().is_empty());
    }

    #[test]
    fn cursors_track_success_and_failures() {
        let (store, _bus) = fixture();
        assert!(store.last_sync("w1").unwrap().is_none());
        assert_eq!(store.failure_count("w1").unwrap(), 0);

        assert_eq!(store.record_failure("w1").unwrap(), 1);
        assert_eq!(store.record_failure("w1").unwrap(), 2);

        store.record_success("w1").unwrap();
        assert!(store.last_sync("w1").unwrap().is_some());
        assert_eq!(store.failure_count("w1").unwrap(), 0);
    }
}
