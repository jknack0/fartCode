//! Project commands (E1-04 sidebar: list, create, delete).

use fartcode_core::projects::{ProjectDto, ProjectStore};
use std::sync::Arc;

use tauri::State;

use crate::app::App;

#[tauri::command]
pub fn list_projects(app: State<'_, Arc<App>>) -> Result<Vec<ProjectDto>, String> {
    app.projects
        .list()
        .map(|ps| ps.iter().map(ProjectDto::from).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_project(app: State<'_, Arc<App>>, path: String) -> Result<ProjectDto, String> {
    app.projects
        .create_local(std::path::Path::new(&path), false)
        .map(|p| ProjectDto::from(&p))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_project(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    app.projects.delete(&id).map_err(|e| e.to_string())
}
