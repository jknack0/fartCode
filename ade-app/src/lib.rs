//! ade-app — Tauri 2 shell.
//!
//! Wires the domain services (ARCHITECTURE §7), registers the E1-04 command
//! modules, and forwards internal events to the frontend.

mod app;
mod commands;
mod indexer;
mod terminals;

use std::sync::Arc;

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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .setup(|app| {
            let app_state = App::init(std::env::var("ADE_DB_FILE").ok().as_deref())?;
            prune_view_state_on_boot(&app_state);
            app::spawn_event_forwarder(app.handle().clone(), app_state.event_bus.clone());
            indexer::spawn_search_indexer(
                app_state.db.clone(),
                app_state.projects.clone(),
                app_state.tasks.clone(),
                app_state.conversations.clone(),
                app_state.event_bus.clone(),
            );
            // E2-07: rehydrate previously-spawned agent sessions AFTER DB
            // init (reference boot order). Each launch blocks, so this runs
            // on a background thread — the window never waits on agents.
            let rehydrator = app_state.rehydrator.clone();
            std::thread::spawn(move || match rehydrator.rehydrate_all() {
                Ok(summary) => tracing::info!(
                    resumed = summary.resumed,
                    skipped = summary.skipped,
                    failed = summary.failed,
                    "boot rehydration complete"
                ),
                Err(e) => tracing::warn!(error = %e, "boot rehydration failed"),
            });
            // E2-12: interactive task terminals (needs the window handle for
            // event emission; created here rather than in App::init).
            let terminal_manager = Arc::new(terminals::TerminalManager::new(app.handle().clone()));
            app.manage(terminal_manager);
            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::projects::list_projects,
            commands::projects::create_project,
            commands::projects::delete_project,
            commands::tasks::create_task,
            commands::tasks::list_tasks,
            commands::tasks::toggle_pin,
            commands::tasks::delete_task,
            commands::provider_accounts::add_provider_account,
            commands::provider_accounts::list_provider_accounts,
            commands::provider_accounts::remove_provider_account,
            commands::provider_accounts::set_default_provider_account,
            commands::provider_accounts::list_providers,
            commands::terminals::terminal_open,
            commands::terminals::terminal_write,
            commands::terminals::terminal_resize,
            commands::terminals::terminal_close,
            commands::settings::get_project_settings,
            commands::settings::update_project_settings,
            commands::settings::share_with_team,
            commands::view_state::get_view_state,
            commands::view_state::set_view_state,
            commands::search::search,
            commands::search::resource_sample,
            commands::search::get_resource_monitor_enabled,
            commands::search::set_resource_monitor_enabled,
            commands::conversations::list_conversations,
            commands::conversations::create_conversation,
            commands::conversations::delete_conversation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
