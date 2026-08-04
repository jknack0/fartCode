//! Task commands (E1-04 sidebar: list, pin toggle; E2-09 delete).

use ade_core::tasks::deletion::DeleteTaskOptions;
use ade_core::tasks::{CreateTaskOptions, TaskDto, TaskStore};
use std::sync::Arc;

use tauri::State;

use crate::app::App;

#[tauri::command]
pub fn create_task(
    app: State<'_, Arc<App>>,
    project_id: String,
    name: String,
) -> Result<TaskDto, String> {
    app.tasks
        .create(CreateTaskOptions {
            project_id,
            name,
            id: None,
            initial_status: None,
            linked_issue: None,
            initial_conversation: None,
            automation_run_id: None,
            workspace_target: None,
            workspace_config: None,
        })
        .map(|t| TaskDto::from(&t))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_tasks(app: State<'_, Arc<App>>, project_id: String) -> Result<Vec<TaskDto>, String> {
    app.tasks
        .list_by_project(&project_id)
        .map(|ts| ts.iter().map(TaskDto::from).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_pin(app: State<'_, Arc<App>>, id: String) -> Result<TaskDto, String> {
    let task = app.tasks.get(&id).map_err(|e| e.to_string())?;
    let task = task.ok_or_else(|| format!("task not found: {id}"))?;
    app.tasks
        .set_pinned(&id, !task.is_pinned)
        .map(|t| TaskDto::from(&t))
        .map_err(|e| e.to_string())
}

/// E2-09: deletes the task with full teardown (running sessions reaped,
/// view state dropped, worktree removed when unused). Options follow the
/// reference `DeleteTaskOptions` defaults.
#[tauri::command]
pub fn delete_task(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<crate::terminals::TerminalManager>>,
    project_id: String,
    task_id: String,
    delete_worktree: Option<bool>,
    delete_branch: Option<bool>,
) -> Result<(), String> {
    let options = DeleteTaskOptions {
        delete_worktree: delete_worktree.unwrap_or(true),
        delete_branch: delete_branch.unwrap_or(false),
    };
    app.deletion
        .delete_task(&project_id, &task_id, &options)
        .map_err(|e| e.to_string())?;
    // E2-12: deleting a task closes its interactive terminals.
    terminals.close_task(&task_id);
    Ok(())
}
