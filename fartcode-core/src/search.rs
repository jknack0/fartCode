//! FTS search (E1-09): the `search_index` FTS5 virtual table (created by
//! E1-01's `ensure_fts_tables`, trigram tokenizer) backs ⌘K lookups over
//! projects and tasks. This module owns the write path
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

/// One indexable document, for the bulk paths ([`replace_group`]).
///
/// The scalar [`upsert`] takes its fields positionally; a group write of
/// N sections wants them named, and wants the keywords already joined
/// (the caller decides how it extracts and caps them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub item_id: String,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub title: String,
    pub keywords: String,
}

/// Replaces every `item_type` row whose `item_id` starts with `prefix`
/// with exactly `docs`, in one transaction.
///
/// This is the *set* operation the per-row [`upsert`] cannot express:
/// re-running it is idempotent (the deterministic rowid makes each write a
/// replace) AND it removes rows whose source has disappeared. Callers that
/// derive many rows from one file — E19-03's dossier sections — need both
/// halves or a deleted section lives on in ⌘K forever.
///
/// Prefix matching is `substr`, not `LIKE`: item ids are `iss_<uuid>`-ish
/// and `_` is a `LIKE` wildcard, so `LIKE 'iss_…'` would quietly match ids
/// this group does not own.
///
/// Returns the number of rows written.
pub fn replace_group(
    db: &Arc<dyn Db>,
    item_type: &str,
    prefix: &str,
    docs: &[Document],
) -> Result<usize, Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    let tx = conn.unchecked_transaction()?;
    for doc in docs {
        upsert_row(
            &tx,
            item_type,
            &doc.item_id,
            doc.project_id.as_deref(),
            doc.task_id.as_deref(),
            &doc.title,
            &[doc.keywords.as_str()],
        )?;
    }
    // Whatever carried the prefix before and is not in `docs` now is a
    // source that vanished.
    let stale: Vec<String> = ids_with_prefix(&tx, item_type, prefix)?
        .into_iter()
        .filter(|id| !docs.iter().any(|d| &d.item_id == id))
        .collect();
    delete_ids(&tx, item_type, &stale)?;
    tx.commit()?;
    Ok(docs.len())
}

/// Removes every `item_type` row whose `item_id` starts with `prefix`
/// (see [`replace_group`] on why this is `substr`, not `LIKE`). Returns
/// the number of rows removed.
pub fn delete_group(db: &Arc<dyn Db>, item_type: &str, prefix: &str) -> Result<usize, Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    let tx = conn.unchecked_transaction()?;
    let ids = ids_with_prefix(&tx, item_type, prefix)?;
    let removed = delete_ids(&tx, item_type, &ids)?;
    tx.commit()?;
    Ok(removed)
}

/// Removes every `item_type` row belonging to a project — the
/// project-teardown sweep for row families a project owns (E19-03's
/// `feature` rows, which the `projects` FK cascade never sees because FTS5
/// is not a relational table). `project_id` is UNINDEXED, so this scans;
/// it runs once per project deletion.
pub fn delete_by_project(
    db: &Arc<dyn Db>,
    item_type: &str,
    project_id: &str,
) -> Result<usize, Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    let tx = conn.unchecked_transaction()?;
    let ids: Vec<String> = {
        let mut stmt = tx
            .prepare("SELECT item_id FROM search_index WHERE item_type = ?1 AND project_id = ?2")?;
        let rows = stmt.query_map(rusqlite::params![item_type, project_id], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let removed = delete_ids(&tx, item_type, &ids)?;
    tx.commit()?;
    Ok(removed)
}

fn ids_with_prefix(
    conn: &rusqlite::Connection,
    item_type: &str,
    prefix: &str,
) -> Result<Vec<String>, Error> {
    let mut stmt = conn.prepare(
        "SELECT item_id FROM search_index
          WHERE item_type = ?1 AND substr(item_id, 1, ?2) = ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![item_type, prefix.chars().count() as i64, prefix],
        |row| row.get::<_, String>(0),
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
}

/// Deletes by rowid — the same key the upsert writes, so the FTS index
/// drops exactly the row a replace would have overwritten.
fn delete_ids(
    conn: &rusqlite::Connection,
    item_type: &str,
    ids: &[String],
) -> Result<usize, Error> {
    for id in ids {
        conn.execute(
            "DELETE FROM search_index WHERE rowid = ?1",
            [rowid_for(item_type, id)],
        )?;
    }
    Ok(ids.len())
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
        )
        .unwrap();
        assert_eq!(query(&db, "acme", 10).unwrap().len(), 1);
        assert_eq!(query(&db, "fix", 10).unwrap().len(), 1, "task title");
    }
}
