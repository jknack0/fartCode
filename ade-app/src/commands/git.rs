//! Git status/diff commands (E4-02): thin wrappers over `ade_git::status` /
//! `ade_git::diff`, worktree-scoped via the task's workspace row. The
//! Changes sidebar (E4-03) refetches these on `git:changed` /
//! `files:changed` events — the commands themselves never poll or cache.

use std::path::PathBuf;
use std::sync::Arc;

use ade_git::diff::{DiffSide, FileDiff};
use ade_git::status::StatusSnapshot;
use rusqlite::OptionalExtension;
use tauri::State;

use crate::app::App;

/// Resolves a workspace's materialized worktree path.
fn workspace_path(app: &App, workspace_id: &str) -> Result<PathBuf, String> {
    let conn = app
        .db
        .conn()
        .lock()
        .map_err(|_| "db connection mutex poisoned".to_string())?;
    let path: Option<Option<String>> = conn
        .query_row(
            "SELECT path FROM workspaces WHERE id = ?1",
            [workspace_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match path {
        None => Err(format!("workspace not found: {workspace_id}")),
        Some(None) => Err(format!("workspace has no local path: {workspace_id}")),
        Some(Some(p)) if p.is_empty() => {
            Err(format!("workspace has no local path: {workspace_id}"))
        }
        Some(Some(p)) => Ok(PathBuf::from(p)),
    }
}

/// Status snapshot (staged/unstaged/conflicts) for a workspace's worktree.
#[tauri::command]
pub fn git_status(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<StatusSnapshot, String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    ade_git::status::status(&worktree).map_err(String::from)
}

/// Two-sided diff payload for one file. `side`: `"staged"` (HEAD↔index) or
/// `"unstaged"` (index↔worktree). `orig_path` echoes the status entry's
/// rename source for staged renames.
#[tauri::command]
pub fn git_file_diff(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    path: String,
    side: DiffSide,
    orig_path: Option<String>,
) -> Result<FileDiff, String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    ade_git::diff::file_diff(&worktree, &path, side, orig_path.as_deref()).map_err(String::from)
}

/// Stages the given worktree-relative paths (`git add --`).
#[tauri::command]
pub fn git_stage(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    ade_git::stage::stage(&worktree, &paths).map_err(String::from)
}

/// Stages every change in the worktree (`git add -A`).
#[tauri::command]
pub fn git_stage_all(app: State<'_, Arc<App>>, workspace_id: String) -> Result<(), String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    ade_git::stage::stage_all(&worktree).map_err(String::from)
}

/// Unstages the given paths (`git restore --staged --`; unborn-HEAD safe).
#[tauri::command]
pub fn git_unstage(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    ade_git::stage::unstage(&worktree, &paths).map_err(String::from)
}

/// Discards the given paths: tracked paths revert to the index, untracked
/// paths are deleted. The UI confirms before calling.
#[tauri::command]
pub fn git_discard(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    ade_git::stage::discard(&worktree, &paths).map_err(String::from)
}
