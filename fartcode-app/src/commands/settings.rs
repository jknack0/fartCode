//! Project-settings commands (E1-05): read/update + share-with-team.
//! `update` validates the worktree directory BEFORE storing, so an invalid
//! value surfaces the typed `invalid-worktree-directory` error instead of
//! being stored and silently falling back on read.

use std::sync::Arc;

use fartcode_core::events::EventBus as _;
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

/// 7c `⇧⌘s share local values` (E1-02): moves the project's local
/// shareable values into the repo `.fartCode.json` and clears them from the
/// DB. A no-op (nothing local) still succeeds.
#[tauri::command]
pub fn project_settings_share(app: State<'_, Arc<App>>, project_id: String) -> Result<(), String> {
    app.settings
        .share_with_team(&project_id)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// ProjectSettings "Default agent · model" row: writes the app-level
/// `defaultAgent` setting — the provider `create_task` / dispatch launch
/// when no explicit agent is chosen. The id is validated against the
/// provider registry before storing; emits `setting:changed` (key
/// `defaultAgent`) so frontends refetch.
#[tauri::command]
pub fn set_default_agent(app: State<'_, Arc<App>>, provider_id: String) -> Result<(), String> {
    set_default_agent_core(&app, &provider_id)
}

/// Command body, split out so tests can drive it without Tauri `State`.
pub fn set_default_agent_core(app: &App, provider_id: &str) -> Result<(), String> {
    if fartcode_providers::get(provider_id).is_none() {
        return Err(format!("unknown agent provider: {provider_id}"));
    }
    app.settings
        .set(
            &fartcode_core::settings::DEFAULT_AGENT,
            provider_id.to_string(),
        )
        .map_err(|e| e.to_string())?;
    app.event_bus
        .send(fartcode_core::events::InternalEvent::SettingChanged {
            key: fartcode_core::settings::DEFAULT_AGENT.name().to_string(),
        });
    Ok(())
}

/// 7c shared-vs-local tags: which source won each SHAREABLE setting key
/// (`preservePatterns` / `shellSetup` / `scripts`) under the effective
/// merge `defaults < .fartCode.json < DB`. Values: `"default"` | `"shared"`
/// | `"local"`.
#[tauri::command]
pub fn project_settings_provenance(
    app: State<'_, Arc<App>>,
    project_id: String,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let project = app
        .projects
        .get(&project_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    app.settings
        .shareable_provenance(&project_id, &project.path)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::events::{EventBus as _, InternalEvent};
    use fartcode_core::settings::DEFAULT_AGENT;

    #[test]
    fn set_default_agent_validates_persists_and_emits() {
        let app = crate::app::App::init(Some(":memory:")).unwrap();
        let mut rx = app.event_bus.subscribe();

        // Unknown provider ids are rejected before anything is stored.
        assert!(set_default_agent_core(&app, "not-a-provider").is_err());
        assert_eq!(app.settings.get(&DEFAULT_AGENT).unwrap(), "claude");

        // A registry id persists and emits setting:changed(defaultAgent).
        set_default_agent_core(&app, "codex").unwrap();
        assert_eq!(app.settings.get(&DEFAULT_AGENT).unwrap(), "codex");
        match rx.try_recv() {
            Ok(InternalEvent::SettingChanged { key }) => assert_eq!(key, "defaultAgent"),
            other => panic!("expected SettingChanged, got {other:?}"),
        }
    }
}
