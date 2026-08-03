//! FTS search (E1-09): the `search_index` FTS5 virtual table (created by
//! E1-01's `ensure_fts_tables`, trigram tokenizer) backs ⌘K lookups over
//! projects, tasks, and conversations. This module owns the write path
//! (upsert/delete/backfill) and the query path.

use std::sync::Arc;

use crate::db::Db;
use crate::Error;

/// A search hit (frontend DTO shape).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub item_type: String,
    pub item_id: String,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub title: String,
}

/// Deterministic 64-bit rowid from the item key so upserts dedupe (FTS5 has
/// no unique constraint on columns). 64-bit FNV-1a — a 32-bit hash collides
/// with non-trivial odds at ~10k rows and would silently overwrite/delete the
/// wrong item.
fn rowid_for(item_type: &str, item_id: &str) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{item_type}:{item_id}").as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash as i64
}

/// Inserts or replaces a document (reference: index stays current).
pub fn upsert(
    db: &Arc<dyn Db>,
    item_type: &str,
    item_id: &str,
    project_id: Option<&str>,
    task_id: Option<&str>,
    title: &str,
    keywords: &[&str],
) -> Result<(), Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    conn.execute(
        "INSERT OR REPLACE INTO search_index (rowid, item_type, item_id, project_id, task_id, title, keywords)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            rowid_for(item_type, item_id),
            item_type,
            item_id,
            project_id,
            task_id,
            title,
            keywords.join(" ")
        ],
    )?;
    Ok(())
}

/// Updates a document's title/keywords WITHOUT touching its item/project/task
/// columns (rename path — an FTS5 INSERT OR REPLACE would wipe the links the
/// palette navigates with).
pub fn update_title(
    db: &Arc<dyn Db>,
    item_type: &str,
    item_id: &str,
    title: &str,
) -> Result<(), Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    conn.execute(
        "UPDATE search_index SET title = ?1, keywords = ?1 WHERE rowid = ?2 AND item_type = ?3",
        rusqlite::params![title, rowid_for(item_type, item_id), item_type],
    )?;
    Ok(())
}

/// Removes a document.
pub fn delete(db: &Arc<dyn Db>, item_type: &str, item_id: &str) -> Result<(), Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    conn.execute(
        "DELETE FROM search_index WHERE rowid = ?1",
        [rowid_for(item_type, item_id)],
    )?;
    Ok(())
}

/// FTS5 keyword/trigram query. The query is quoted so the trigram tokenizer
/// matches substrings (unquoted input is tokenized per-trigram already, but
/// quoted phrases match more precisely).
pub fn query(db: &Arc<dyn Db>, q: &str, limit: usize) -> Result<Vec<SearchResult>, Error> {
    let q = q.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let quoted = format!("\"{}\"", q.replace('"', "\"\""));
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    let mut stmt = conn.prepare(
        "SELECT item_type, item_id, project_id, task_id, title FROM search_index
         WHERE search_index MATCH ?1 ORDER BY rank LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![quoted, limit as i64], |row| {
        Ok(SearchResult {
            item_type: row.get(0)?,
            item_id: row.get(1)?,
            project_id: row.get(2)?,
            task_id: row.get(3)?,
            title: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
}

/// Boot-time repopulation from the source tables (reference: index stays
/// current via events, but a fresh DB needs a backfill).
pub fn backfill(
    db: &Arc<dyn Db>,
    projects: &[(String, String)],
    tasks: &[(String, String, String)],
    conversations: &[(String, String, String)],
) -> Result<(), Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    conn.execute("DELETE FROM search_index", [])?;
    for (id, name) in projects {
        upsert_row(&conn, "project", id, None, None, name, &[name])?;
    }
    for (id, project_id, name) in tasks {
        upsert_row(&conn, "task", id, Some(project_id), None, name, &[name])?;
    }
    for (id, task_id, title) in conversations {
        upsert_row(
            &conn,
            "conversation",
            id,
            None,
            Some(task_id),
            title,
            &[title],
        )?;
    }
    Ok(())
}

fn upsert_row(
    conn: &rusqlite::Connection,
    item_type: &str,
    item_id: &str,
    project_id: Option<&str>,
    task_id: Option<&str>,
    title: &str,
    keywords: &[&str],
) -> Result<(), Error> {
    conn.execute(
        "INSERT OR REPLACE INTO search_index (rowid, item_type, item_id, project_id, task_id, title, keywords)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            rowid_for(item_type, item_id),
            item_type,
            item_id,
            project_id,
            task_id,
            title,
            keywords.join(" ")
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteDb;

    fn db() -> Arc<dyn Db> {
        // ensure_fts_tables runs inside init.
        SqliteDb::init_in_memory().unwrap()
    }

    #[test]
    fn upsert_query_delete_round_trip() {
        let db = db();
        upsert(
            &db,
            "project",
            "p1",
            None,
            None,
            "acme-web",
            &["acme-web", "web"],
        )
        .unwrap();
        upsert(
            &db,
            "task",
            "t1",
            Some("p1"),
            None,
            "fix the navbar",
            &["fix", "navbar"],
        )
        .unwrap();

        let hits = query(&db, "acme", 10).unwrap();
        assert_eq!(hits.len(), 1, "project matches: {hits:?}");
        assert_eq!(hits[0].item_id, "p1");

        // Trigram substring: 3+ chars match anywhere.
        let hits = query(&db, "navba", 10).unwrap();
        assert_eq!(hits.len(), 1, "trigram substring: {hits:?}");
        assert_eq!(hits[0].item_id, "t1");

        // Upsert dedupes (same rowid).
        upsert(
            &db,
            "task",
            "t1",
            Some("p1"),
            None,
            "fix the navbar NOW",
            &["fix"],
        )
        .unwrap();
        let hits = query(&db, "navbar", 10).unwrap();
        assert_eq!(hits.len(), 1, "no dupes on upsert");

        delete(&db, "task", "t1").unwrap();
        assert!(query(&db, "navbar", 10).unwrap().is_empty());

        assert!(query(&db, "", 10).unwrap().is_empty(), "empty query");
        assert!(query(&db, "   ", 10).unwrap().is_empty(), "blank query");
    }

    #[test]
    fn backfill_repopulates() {
        let db = db();
        backfill(
            &db,
            &[("p1".into(), "acme-web".into())],
            &[("t1".into(), "p1".into(), "fix navbar".into())],
            &[("c1".into(), "t1".into(), "how do we fix it".into())],
        )
        .unwrap();
        assert_eq!(query(&db, "acme", 10).unwrap().len(), 1);
        assert_eq!(
            query(&db, "fix", 10).unwrap().len(),
            2,
            "task + conversation"
        );
        assert_eq!(query(&db, "how", 10).unwrap().len(), 1);
    }
}
