//! Git status/diff commands (E4-02): thin wrappers over `ade_git::status` /
//! `ade_git::diff`, worktree-scoped via the task's workspace row. The
//! Changes sidebar (E4-03) refetches these on `git:changed` /
//! `files:changed` events — the commands themselves never poll or cache.

use std::path::PathBuf;
use std::sync::Arc;

use ade_core::projects::ProjectStore;
use ade_git::diff::{DiffSide, FileDiff};
use ade_git::status::StatusSnapshot;
use rusqlite::OptionalExtension;
use tauri::State;

use crate::app::App;

/// Resolves a workspace's materialized worktree path. Shared by the git
/// commands and the workspace-file commands (E4-05).
pub(crate) fn workspace_path(app: &App, workspace_id: &str) -> Result<PathBuf, String> {
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

/// Commit-card repo state (E4-06): branch, push-remote presence, published
/// flag, and the PR-open guard (`StubPrLookup` until E4-07/E8 land). The
/// push remote is the owning project's effective `pushRemote` setting.
#[tauri::command]
pub fn git_commit_state(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<ade_git::commit::CommitState, String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    let push_remote = project_push_remote(&app, &workspace_id)?;
    ade_git::commit::state(&worktree, &push_remote, &ade_git::commit::StubPrLookup)
        .map_err(String::from)
}

/// Result of `git_commit` — the new commit hash (reference returns it; the
/// card could deep-link to it later).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub hash: String,
}

/// Commits exactly the staged set with `message` (E4-06). Empty messages
/// are rejected before git spawns.
#[tauri::command]
pub fn git_commit(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    message: String,
) -> Result<CommitResult, String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    let hash = ade_git::commit::commit(&worktree, &message).map_err(String::from)?;
    Ok(CommitResult { hash })
}

/// Pushes the current branch, setting upstream on first push (E4-06).
#[tauri::command]
pub fn git_push(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<ade_git::commit::PushOutcome, String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    let push_remote = project_push_remote(&app, &workspace_id)?;
    ade_git::commit::push(&worktree, &push_remote).map_err(String::from)
}

/// Phase 0 "Commit & Create PR" backend (E4-06): pushes when the branch
/// isn't published, applies the PR-open guard, and returns the GitHub
/// compare URL for the browser to finish the job (full PR creation =
/// E4-07/E8).
#[tauri::command]
pub fn git_create_pr(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<ade_git::commit::CreatePrOutcome, String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    let push_remote = project_push_remote(&app, &workspace_id)?;
    ade_git::commit::create_pr(&worktree, &push_remote, &ade_git::commit::StubPrLookup)
        .map_err(String::from)
}

/// Effective `pushRemote` of the project owning `workspace_id` (reference
/// `getPushRemote`: pushRemote ?? baseRemote ?? "origin"). Resolves
/// workspace → task → project; falls back to "origin" for workspaces not
/// owned by a task (repository-root workspaces).
fn project_push_remote(app: &App, workspace_id: &str) -> Result<String, String> {
    let project_id: Option<String> = {
        let conn = app
            .db
            .conn()
            .lock()
            .map_err(|_| "db connection mutex poisoned".to_string())?;
        conn.query_row(
            "SELECT project_id FROM tasks WHERE workspace_id = ?1",
            [workspace_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    };
    let Some(project_id) = project_id else {
        return Ok(ade_core::settings::DEFAULT_BASE_REMOTE.to_string());
    };
    let project = app
        .projects
        .get(&project_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    let settings = app
        .settings
        .get_project_settings(&project_id, &project.path)
        .map_err(|e| e.to_string())?;
    Ok(settings.effective_push_remote().to_string())
}
