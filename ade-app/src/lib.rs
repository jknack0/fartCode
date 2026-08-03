//! ade-app — Tauri 2 shell.
//!
//! Wires the domain services (ARCHITECTURE §7), registers the E1-04 command
//! modules, and forwards internal events to the frontend.

mod app;
mod commands;

use app::App;
use tauri::Manager;

/// Boot-time housekeeping (E1-08): prune orphaned view-state rows.
fn prune_view_state_on_boot(app: &App) {
    if let Err(e) = ade_core::view_state::prune_orphans(&app.db) {
        tracing::warn!(error = %e, "view-state prune failed (non-fatal)");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Second launch focuses the existing window instead of opening
            // a second one (E1-08 acceptance 3) — restore it first in case
            // it was minimized/hidden.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .setup(|app| {
            let app_state = App::init(std::env::var("ADE_DB_FILE").ok().as_deref())?;
            prune_view_state_on_boot(&app_state);
            app::spawn_event_forwarder(app.handle().clone(), app_state.event_bus.clone());
            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::projects::list_projects,
            commands::projects::create_project,
            commands::projects::delete_project,
            commands::tasks::list_tasks,
            commands::tasks::toggle_pin,
            commands::settings::get_project_settings,
            commands::settings::update_project_settings,
            commands::settings::share_with_team,
            commands::view_state::get_view_state,
            commands::view_state::set_view_state,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
