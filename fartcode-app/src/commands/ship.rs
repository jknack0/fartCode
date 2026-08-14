//! Ship verb command (pipeline overhaul, ColumnKind::Ship): squash-merge
//! the task's worktree branch into the project-root checkout and push.
//! The FRONTEND drives the surrounding flow (dirty-worktree dialog →
//! `task_ship` → `issue_enter_column` → the delete-worktree dialog);
//! this command only does the git work, off the IPC thread (#80 — a
//! `git push` on the IPC thread beachballs the window).

use std::sync::Arc;

use fartcode_core::git::GitOps;
use fartcode_core::projects::ProjectStore;
use fartcode_core::tasks::TaskStore;
use fartcode_git::merge::ShipOutcome;
use fartcode_git::CliGit;
use tauri::State;

use crate::app::App;
use crate::commands::git::workspace_path;
use crate::commands::off_main_thread;

/// Push remote for the post-merge push. Settings-driven remotes arrive
/// with the commit card's pushRemote plumbing; origin is today's default
/// everywhere else too.
const PUSH_REMOTE: &str = "origin";

/// Squash-merge + push for a task's branch. `auto_commit` commits the
/// worktree's outstanding changes first (the ship dialog's "commit &
/// ship"); without it a dirty worktree is a typed refusal so the
/// frontend can ask.
#[tauri::command]
pub async fn task_ship(
    app: State<'_, Arc<App>>,
    task_id: String,
    auto_commit: Option<bool>,
) -> Result<ShipOutcome, String> {
    let app = app.inner().clone();
    off_main_thread(move || task_ship_blocking(&app, &task_id, auto_commit.unwrap_or(false))).await
}

fn task_ship_blocking(app: &App, task_id: &str, auto_commit: bool) -> Result<ShipOutcome, String> {
    let task = app
        .tasks
        .get(task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("task not found: {task_id}"))?;
    let project = app
        .projects
        .get(&task.project_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project not found: {}", task.project_id))?;
    let workspace_id = task
        .workspace_id
        .clone()
        .ok_or_else(|| format!("task {task_id} has no workspace to ship"))?;
    if project.repository_workspace_id.as_deref() == Some(workspace_id.as_str()) {
        return Err("task runs in the project root checkout — nothing to merge".into());
    }
    let worktree = workspace_path(app, &workspace_id)?;
    let branch = CliGit
        .current_branch(&worktree)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task worktree is on a detached HEAD".to_string())?;

    // Dirty worktree: refuse (frontend shows the commit-or-cancel dialog)
    // unless the dialog already answered with auto_commit.
    let status = fartcode_git::status::status(&worktree).map_err(|e| e.to_string())?;
    let dirty = !status.staged.is_empty() || !status.unstaged.is_empty() || status.truncated;
    if dirty {
        if !auto_commit {
            return Err(format!(
                "worktree has uncommitted changes on {branch} — commit them or ship with auto-commit"
            ));
        }
        fartcode_git::stage::stage_all(&worktree).map_err(|e| e.to_string())?;
        fartcode_git::commit::commit(&worktree, &format!("Ship: outstanding work on {branch}"))
            .map_err(|e| e.to_string())?;
    }

    fartcode_git::merge::squash_merge_and_push(
        &project.path,
        &branch,
        &format!("Ship: {} ({branch})", task.name),
        PUSH_REMOTE,
    )
    .map_err(|e| e.to_string())
}
