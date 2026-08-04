//! Task commands (E1-04 sidebar: list, pin toggle).

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
