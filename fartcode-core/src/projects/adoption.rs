//! One-shot adoption of per-project worktree pool segments (#81).
//!
//! Pre-#81 pools were `join(default_worktree_directory, safePathSegment(name))`
//! — two projects sharing a basename shared a pool, and deleting one deleted
//! the other's worktrees. This pass runs once per database (kv-gated), before
//! anything resolves pools (wired in `DbProjectStore::new`):
//!
//! - Legacy segment claimed by ONE project → adopted in place (stamp the
//!   legacy value; zero filesystem churn, `cd` paths unchanged) — unless the
//!   project has a `worktree_directory` override (then the legacy dir moves
//!   there, F3: pre-#81 the override was dead, so all legacy pools live under
//!   the app default) or another project already holds the legacy segment
//!   (F6: interrupted previous run — the project becomes a mover instead).
//! - Collision → one keeper keeps the legacy dir (deterministic tiebreak: the
//!   sole project with worktrees on disk, else earliest `created_at`, else
//!   smallest id); the others get new-scheme segments and their worktree
//!   subdirectories are moved out of the shared dir (`fs::rename` +
//!   `git worktree repair` sweep), with the stored `workspaces.path` rewritten.
//!
//! Failures never block startup: per-project errors warn and skip. The kv
//! gate is set ONLY when the whole pass succeeded — stamping is idempotent
//! (`WHERE worktree_pool_segment IS NULL`) and moved rows take the
//! `!old_path.exists()` repoint branch, so a partial pass safely retries on
//! the next startup (F2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::db::Db;
use crate::git::GitOps;
use crate::projects::model::{project_from_row, Project, PROJECT_COLUMNS};
use crate::projects::provider::{new_pool_segment, safe_path_segment};
use crate::settings::SettingsStore;
use crate::Error;

/// kv gate — set once the pass completes without any per-project failure.
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

    // F9: the app default root is resolved lazily per project — a settings
    // read failure skips that project instead of aborting the whole pass.
    let mut all_ok = true;
    for (legacy, members) in groups {
        if members.len() == 1 {
            if let Err(e) = adopt_single(db, settings, git, members[0], &legacy) {
                tracing::warn!(project_id = %members[0].id, error = %e, "pool adoption failed for project (retried next startup)");
                all_ok = false;
            }
        } else if !adopt_collision(db, settings, git, &legacy, &members) {
            all_ok = false;
        }
    }

    // F2: gate on success only. A partial pass retries next startup; re-runs
    // are safe (stamping is idempotent, moved rows repoint without churn).
    if all_ok {
        db.kv_set(POOL_ADOPTION_GATE, "done")?;
    }
    Ok(())
}

/// Sole claimant of a legacy segment.
fn adopt_single(
    db: &dyn Db,
    settings: &dyn SettingsStore,
    git: &dyn GitOps,
    project: &Project,
    legacy: &str,
) -> Result<(), Error> {
    // F6: another project may already hold the legacy segment (e.g. a crash
    // between keeper stamp and mover completion in a previous run). Adopting
    // in place would duplicate it — become a mover to a new-scheme segment.
    if legacy_segment_taken(db, legacy, &project.id)? {
        let from_root = app_default_root(settings)?;
        let to_root = target_root(settings, project)?;
        let segment = new_pool_segment(project);
        return move_project_pool(
            db,
            git,
            project,
            &from_root.join(legacy),
            &to_root.join(&segment),
            &segment,
        );
    }

    // F9: the default root locates the legacy pool. When it is unreadable,
    // a project with no worktree rows needs no filesystem knowledge —
    // stamping in place is still safe.
    let from_root = match app_default_root(settings) {
        Ok(root) => root,
        Err(e) => {
            if project_has_worktree_rows(db, &project.id)? {
                return Err(e);
            }
            tracing::warn!(project_id = %project.id, error = %e, "localProject unreadable; stamping project with no worktree rows");
            return stamp(db, &project.id, legacy);
        }
    };

    let to_root = target_root(settings, project)?;
    if to_root != from_root {
        // F3: the resolver honors the per-project override but the legacy
        // pool lives under the app default — relocate it there, keeping the
        // legacy segment (unique across projects: sole claimant of a
        // name-derived segment).
        move_project_pool(
            db,
            git,
            project,
            &from_root.join(legacy),
            &to_root.join(legacy),
            legacy,
        )
    } else {
        // Sole claimant, app default root: adopt in place — no fs changes.
        stamp(db, &project.id, legacy)
    }
}

/// Collision: one keeper keeps the legacy dir; the rest move to new pools.
/// Returns false when any member failed (gate stays unset → retry).
fn adopt_collision(
    db: &dyn Db,
    settings: &dyn SettingsStore,
    git: &dyn GitOps,
    legacy: &str,
    members: &[&Project],
) -> bool {
    // Pre-#81 the override was dead — the shared legacy pool always lives
    // under the app default root.
    let Ok(default_root) = app_default_root(settings) else {
        tracing::warn!(segment = legacy, "pool adoption: localProject unreadable — collision group skipped (retried next startup)");
        return false;
    };
    let shared_pool = default_root.join(legacy);

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

    let mut all_ok = true;
    for (idx, project) in members.iter().enumerate() {
        // Leave the segment NULL on failure: re-run retries the move/stamp
        // (moves are idempotent — already-moved rows just repoint).
        let res: Result<(), Error> = (|| {
            let to_root = target_root(settings, project)?;
            if idx == keeper_idx {
                if to_root != default_root {
                    // F3: keeper with an override — keep the legacy segment,
                    // relocate the dir to the override root (rows are
                    // DB-attributed, so only this project's worktrees move).
                    move_project_pool(
                        db,
                        git,
                        project,
                        &shared_pool,
                        &to_root.join(legacy),
                        legacy,
                    )
                } else {
                    stamp(db, &project.id, legacy)
                }
            } else {
                let segment = new_pool_segment(project);
                move_project_pool(
                    db,
                    git,
                    project,
                    &shared_pool,
                    &to_root.join(&segment),
                    &segment,
                )
            }
        })();
        if let Err(e) = res {
            tracing::warn!(project_id = %project.id, error = %e, "pool adoption failed for project (retried next startup)");
            all_ok = false;
        }
    }
    all_ok
}

/// Moves this project's worktree subdirectories from `from_pool` into
/// `to_pool` (possibly under a different root — F3), repairs git linkage in
/// an idempotent sweep, rewrites stored paths, and stamps `segment`.
fn move_project_pool(
    db: &dyn Db,
    git: &dyn GitOps,
    project: &Project,
    from_pool: &Path,
    to_pool: &Path,
    segment: &str,
) -> Result<(), Error> {
    // F4: stored paths can be realpathed (git reports realpath, e.g.
    // /private/var vs /var) — canonicalize both sides of every comparison.
    let from_pool_c = canonicalize_or_self(from_pool);

    for (workspace_id, path_str) in worktree_rows(db, &project.id)? {
        let old_path = canonicalize_or_self(Path::new(&path_str));
        if !old_path.starts_with(&from_pool_c) || old_path == from_pool_c {
            continue;
        }
        let Ok(rel) = old_path.strip_prefix(&from_pool_c) else {
            continue;
        };
        let new_path = to_pool.join(rel);
        if !Path::new(&path_str).exists() {
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
        // F1: production branch names nest (`<branch_prefix>/<branch>`), so
        // the target's parent dirs usually don't exist yet.
        if let Some(parent) = new_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&old_path, &new_path)?;
        update_workspace_path(db, &workspace_id, &new_path)?;
    }

    // F5: repair as a RETRYABLE SWEEP over every worktree row of this
    // project that now lives under the new pool — idempotent, and also heals
    // rows moved by a previous failed run. Only stamp when every repair
    // succeeded: a failed repair leaves `.git/worktrees/*/gitdir` pointing at
    // the old path, which the next prune would turn into a stale path.
    let to_pool_c = canonicalize_or_self(to_pool);
    let mut repair_failed = false;
    for (_workspace_id, path_str) in worktree_rows(db, &project.id)? {
        let path = PathBuf::from(&path_str);
        if !path.exists() || !canonicalize_or_self(&path).starts_with(&to_pool_c) {
            continue;
        }
        if let Err(e) = git.worktree_repair(&project.path, &path) {
            tracing::warn!(path = %path.display(), error = %e, "git worktree repair failed during adoption");
            repair_failed = true;
        }
    }
    if repair_failed {
        return Err(Error::Internal(format!(
            "git worktree repair failed for project {} — segment left unset, retried next startup",
            project.id
        )));
    }

    stamp(db, &project.id, segment)
}

/// Does this project have worktree workspace rows whose path exists on disk
/// under `pool`?
fn has_worktrees_on_disk(db: &dyn Db, project_id: &str, pool: &Path) -> Result<bool, Error> {
    let pool_c = canonicalize_or_self(pool);
    Ok(worktree_rows(db, project_id)?.iter().any(|(_id, p)| {
        let c = canonicalize_or_self(Path::new(p));
        c != pool_c && c.starts_with(&pool_c) && Path::new(p).exists()
    }))
}

/// Does another project already hold `segment`? (F6 interrupted-run check.)
fn legacy_segment_taken(db: &dyn Db, segment: &str, exclude_id: &str) -> Result<bool, Error> {
    with_conn(db, |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM projects WHERE worktree_pool_segment = ?1 AND id != ?2",
            rusqlite::params![segment, exclude_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    })
}

fn project_has_worktree_rows(db: &dyn Db, project_id: &str) -> Result<bool, Error> {
    Ok(!worktree_rows(db, project_id)?.is_empty())
}

/// `(workspace_id, path)` of this project's worktree workspaces.
fn worktree_rows(db: &dyn Db, project_id: &str) -> Result<Vec<(String, String)>, Error> {
    crate::workspaces::worktree_rows_for_project(db, project_id)
}

/// App-level default worktree root. Resolved lazily (F9): a read failure
/// must skip the affected project, not abort the pass.
fn app_default_root(settings: &dyn SettingsStore) -> Result<PathBuf, Error> {
    let json = settings.get_json("localProject")?;
    let local: crate::settings::LocalProjectGroup =
        serde_json::from_value(json).map_err(|e| Error::InvalidSettingValue {
            key: "localProject".into(),
            reason: e.to_string(),
        })?;
    Ok(PathBuf::from(local.default_worktree_directory))
}

/// The project's target pool root, resolved the SAME way as the resolver
/// (`worktree_pool_path`): override when set, else the app default (F3).
fn target_root(settings: &dyn SettingsStore, project: &Project) -> Result<PathBuf, Error> {
    match settings
        .get_project_settings(&project.id, &project.path)
        .ok()
        .and_then(|ps| ps.worktree_directory)
    {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => app_default_root(settings),
    }
}

/// F4: canonicalize with fallback to the raw path on error.
fn canonicalize_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
    crate::workspaces::set_path(db, workspace_id, path)
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
