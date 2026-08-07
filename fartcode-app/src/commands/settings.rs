//! Project-settings commands (E1-05): read/update + share-with-team.
//! `update` validates the worktree directory BEFORE storing, so an invalid
//! value surfaces the typed `invalid-worktree-directory` error instead of
//! being stored and silently falling back on read.

use std::sync::Arc;

use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{ProjectSettings, SettingsStore};
use tauri::State;

use crate::app::App;

#[tauri::command]
pub fn get_project_settings(
    app: State<'_, Arc<App>>,
    project_id: String,
) -> Result<ProjectSettings, String> {
    let project = app
        .projects
        .get(&project_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    app.settings
        .get_project_settings(&project_id, &project.path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_project_settings(
    app: State<'_, Arc<App>>,
    project_id: String,
    settings: ProjectSettings,
) -> Result<ProjectSettings, String> {
    let project = app
        .projects
        .get(&project_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project not found: {project_id}"))?;

    // Validate before storing (E1-05): `~` expands, relative paths are
    // rejected with the typed error code. Blank clears the field (reference
    // resolveAndValidateWorktreeDirectory returns ok(undefined) for blank).
    let settings = match &settings.worktree_directory {
        Some(wd) if wd.trim().is_empty() => ProjectSettings {
            worktree_directory: None,
            ..settings
        },
        Some(wd) => {
            let normalized =
                fartcode_core::settings::worktree_directory::normalize_worktree_directory(
                    wd,
                    fartcode_core::settings::worktree_directory::home_dir().as_deref(),
                )
                .map_err(|e| e.to_string())?;
            ProjectSettings {
                worktree_directory: Some(normalized.to_string_lossy().into_owned()),
                ..settings
            }
        }
        None => settings,
    };

    app.settings
        .update_project_settings(&project_id, &project.path, &settings)
        .map_err(|e| e.to_string())?;
    app.settings
        .get_project_settings(&project_id, &project.path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn share_with_team(app: State<'_, Arc<App>>, project_id: String) -> Result<bool, String> {
    app.settings
        .share_with_team(&project_id)
        .map_err(|e| e.to_string())
}
