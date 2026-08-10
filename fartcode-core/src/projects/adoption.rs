//! One-shot adoption of per-project worktree pool segments (#81).
//!
//! Pre-#81 pools were `join(default_worktree_directory, safePathSegment(name))`
//! — two projects sharing a basename shared a pool, and deleting one deleted
//! the other's worktrees. This pass runs once per database (kv-gated), before
//! anything resolves pools (wired in `DbProjectStore::new`):
//!
//! - Legacy segment claimed by ONE project → adopted in place (stamp the
//!   legacy value; zero filesystem churn, `cd` paths unchanged).
//! - Collision → one keeper keeps the legacy dir (deterministic tiebreak: the
//!   sole project with worktrees on disk, else earliest `created_at`, else
//!   smallest id); the others get new-scheme segments and their worktree
//!   subdirectories are moved out of the shared dir (`fs::rename` +
//!   `git worktree repair`), with the stored `workspaces.path` rewritten.
//!
//! Failures never block startup: per-project errors warn and skip (a skipped
//! project lazily gets a fresh unique pool on first resolve), and the gate is
//! set once the pass completes so it runs exactly once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::db::Db;
use crate::git::GitOps;
use crate::projects::model::{project_from_row, Project, PROJECT_COLUMNS};
use crate::projects::provider::{new_pool_segment, safe_path_segment};
use crate::settings::SettingsStore;
use crate::Error;

/// kv gate — the pass runs exactly once per database.
pub const POOL_ADOPTION_GATE: &str = "worktree_pool_adoption_v1";

/// Runs the adoption pass (idempotent, kv-gated). Top-level errors are for
/// the caller to warn-log; per-project failures are handled internally.
pub fn adopt_pool_segments(
    db: &dyn Db,
    settings: &dyn SettingsStore,
    git: &dyn GitOps,
) -> Result<(), Error> {
    if db.kv_get(POOL_ADOPTION_GATE)?.is_some() {
        return Ok(());
    }

    let json = settings.get_json("localProject")?;
    let local: crate::settings::LocalProjectGroup =
        serde_json::from_value(json).map_err(|e| Error::InvalidSettingValue {
            key: "localProject".into(),
            reason: e.to_string(),
        })?;
    let root = PathBuf::from(local.default_worktree_directory);

    let projects = list_local_projects(db)?;

    // Group by LEGACY segment; BTreeMap for deterministic iteration.
    let mut groups: BTreeMap<String, Vec<&Project>> = BTreeMap::new();
    for project in &projects {
        if project.worktree_pool_segment.is_some() {
            continue; // already stamped (re-run safety)
        }
        groups
            .entry(safe_path_segment(&project.name, &project.id))
            .or_default()
            .push(project);
    }

    for (legacy, members) in groups {
        if members.len() == 1 {
            // Sole claimant: adopt in place — no filesystem changes.
            if let Err(e) = stamp(db, &members[0].id, &legacy) {
                tracing::warn!(project_id = %members[0].id, error = %e, "pool adoption stamp failed");
            }
            continue;
        }
        adopt_collision(db, git, &root, &legacy, &members);
    }

    // Set the gate even when individual projects failed: re-running would not
    // help them (they get fresh unique pools via lazy stamping instead).
    db.kv_set(POOL_ADOPTION_GATE, "done")?;
    Ok(())
}

/// Collision: one keeper keeps the legacy dir; the rest move to new pools.
fn adopt_collision(db: &dyn Db, git: &dyn GitOps, root: &Path, legacy: &str, members: &[&Project]) {
    let shared_pool = root.join(legacy);

    // Keeper: the sole project with worktrees on disk under the shared pool,
    // else earliest created_at, else smallest id (deterministic).
    let on_disk: Vec<bool> = members
        .iter()
        .map(|p| has_worktrees_on_disk(db, &p.id, &shared_pool).unwrap_or(false))
        .collect();
    let keeper_idx = if on_disk.iter().filter(|d| **d).count() == 1 {
        on_disk.iter().position(|d| *d).unwrap()
    } else {
        (0..members.len())
            .min_by(|a, b| {
                let ca = members[*a].created_at.as_deref().unwrap_or("");
                let cb = members[*b].created_at.as_deref().unwrap_or("");
                ca.cmp(cb).then_with(|| members[*a].id.cmp(&members[*b].id))
            })
            .unwrap_or(0)
    };

    for (idx, project) in members.iter().enumerate() {
        if idx == keeper_idx {
            if let Err(e) = stamp(db, &project.id, legacy) {
                tracing::warn!(project_id = %project.id, error = %e, "pool adoption stamp failed");
            }
            continue;
        }
        if let Err(e) = move_project_pool(db, git, project, &shared_pool, root) {
            // Leave the segment NULL: the resolver lazily stamps a fresh
            // unique pool; the half-adopted leftovers stay with the keeper.
            tracing::warn!(project_id = %project.id, error = %e, "pool adoption move failed (project gets a fresh pool)");
        }
    }
}

/// Moves this project's worktree subdirectories out of the shared pool into
/// its own new pool dir, repairs git linkage, rewrites stored paths, and
/// stamps the new segment.
fn move_project_pool(
    db: &dyn Db,
    git: &dyn GitOps,
    project: &Project,
    shared_pool: &Path,
    root: &Path,
) -> Result<(), Error> {
    let new_segment = new_pool_segment(project);
    let new_pool = root.join(&new_segment);

    // Attribute shared-pool worktrees via the DB: kind='worktree' workspace
    // rows of this project's tasks under the shared pool.
    let rows = with_conn(db, |conn| {
        let mut stmt = conn.prepare(
            "SELECT w.id, w.path FROM workspaces w
             JOIN tasks t ON t.workspace_id = w.id
             WHERE t.project_id = ?1 AND w.kind = 'worktree' AND w.path IS NOT NULL",
        )?;
        let rows = stmt.query_map(rusqlite::params![project.id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })?;

    for (workspace_id, path_str) in rows {
        let old_path = PathBuf::from(&path_str);
        if !old_path.starts_with(shared_pool) || old_path == shared_pool {
            continue;
        }
        let Ok(rel) = old_path.strip_prefix(shared_pool) else {
            continue;
        };
        let new_path = new_pool.join(rel);
        if !old_path.exists() {
            // Gone from disk: just repoint the row so the record stays true.
            update_workspace_path(db, &workspace_id, &new_path)?;
            continue;
        }
        if new_path.exists() {
            return Err(Error::Internal(format!(
                "adoption target already exists: {}",
                new_path.display()
            )));
        }
        std::fs::create_dir_all(&new_pool)?;
        std::fs::rename(&old_path, &new_path)?;
        if let Err(e) = git.worktree_repair(&project.path, &new_path) {
            tracing::warn!(path = %new_path.display(), error = %e, "git worktree repair after adoption move failed (non-fatal)");
        }
        update_workspace_path(db, &workspace_id, &new_path)?;
    }

    stamp(db, &project.id, &new_segment)
}

/// Does this project have worktree workspace rows whose path exists on disk
/// under `pool`?
fn has_worktrees_on_disk(db: &dyn Db, project_id: &str, pool: &Path) -> Result<bool, Error> {
    let paths: Vec<String> = with_conn(db, |conn| {
        let mut stmt = conn.prepare(
            "SELECT w.path FROM workspaces w
             JOIN tasks t ON t.workspace_id = w.id
             WHERE t.project_id = ?1 AND w.kind = 'worktree' AND w.path IS NOT NULL",
        )?;
        let rows = stmt.query_map(rusqlite::params![project_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })?;
    Ok(paths.iter().any(|p| {
        let path = PathBuf::from(p);
        path.starts_with(pool) && path != pool && path.exists()
    }))
}

fn list_local_projects(db: &dyn Db) -> Result<Vec<Project>, Error> {
    with_conn(db, |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {PROJECT_COLUMNS} FROM projects WHERE workspace_provider = 'local'"
        ))?;
        let rows = stmt.query_map([], project_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
}

fn stamp(db: &dyn Db, project_id: &str, segment: &str) -> Result<(), Error> {
    with_conn(db, |conn| {
        conn.execute(
            "UPDATE projects SET worktree_pool_segment = ?1, updated_at = datetime('now')
             WHERE id = ?2 AND worktree_pool_segment IS NULL",
            rusqlite::params![segment, project_id],
        )?;
        Ok(())
    })
}

fn update_workspace_path(db: &dyn Db, workspace_id: &str, path: &Path) -> Result<(), Error> {
    with_conn(db, |conn| {
        conn.execute(
            "UPDATE workspaces SET path = ?1, updated_at = datetime('now') WHERE id = ?2",
            rusqlite::params![path.to_string_lossy(), workspace_id],
        )?;
        Ok(())
    })
}

fn with_conn<T>(
    db: &dyn Db,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, Error>,
) -> Result<T, Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    f(&conn)
}
