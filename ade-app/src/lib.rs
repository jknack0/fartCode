//! ade-app — Tauri 2 shell.
//!
//! Wires the domain services (ARCHITECTURE §7), registers the E1-04 command
//! modules, and forwards internal events to the frontend.

mod acp_events;
pub mod acp_runtime;
mod app;
mod commands;
mod indexer;
mod terminals;
mod watchers;

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
        // Window close = detach, not teardown (ADR-0028): the tmux sessions
        // must survive so reopening the UI reattaches the same shells.
        // (Task/tab close is the teardown path — it kills the sessions.)
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(terminals) = window.try_state::<Arc<terminals::TerminalManager>>() {
                    terminals.detach_all();
                }
            }
        })
        .setup(|app| {
            let app_state = App::init(std::env::var("ADE_DB_FILE").ok().as_deref())?;
            prune_view_state_on_boot(&app_state);
            app::spawn_event_forwarder(app.handle().clone(), app_state.event_bus.clone());
            indexer::spawn_search_indexer(
                app_state.db.clone(),
                app_state.projects.clone(),
                app_state.tasks.clone(),
                app_state.event_bus.clone(),
            );
            // E4-01: workspace file+git watches (boot backfill + provision/
            // delete subscription).
            watchers::spawn_workspace_watchers(
                app_state.db.clone(),
                app_state.fs_watch.clone(),
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
            // E2-11-4/5: ACP runtime — owns the SessionManager, spawns the
            // adapter per conversation, and emits `acp:update` /
            // `acp:transcript` / `acp:permission_request` via the events
            // emitter. `ADE_ACP_ADAPTER` overrides the adapter binary for
            // tests (dev/test fixture only; production resolves per
            // provider).
            let acp_events = acp_events::TauriAcpEvents::new(app.handle().clone());
            let acp_runtime = crate::acp_runtime::AcpRuntime::new(
                app_state.conversations.clone(),
                app_state.tasks.clone(),
                app_state.db.clone(),
                app_state.provider_accounts.clone(),
                acp_events.clone(),
                match std::env::var("ADE_ACP_ADAPTER").ok() {
                    Some(path) => {
                        let bin = std::path::PathBuf::from(path);
                        Arc::new(move |_provider_id: &str| Ok(bin.clone()))
                    }
                    None => Arc::new(|provider_id: &str| {
                        crate::acp_runtime::default_adapter_resolver(provider_id)
                    }),
                },
            );
            app.manage(acp_events);
            app.manage(acp_runtime);
            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::projects::list_projects,
            commands::projects::create_project,
            commands::projects::delete_project,
            commands::tasks::create_task,
            commands::tasks::provision_task,
            commands::tasks::list_tasks,
            commands::tasks::toggle_pin,
            commands::tasks::delete_task,
            commands::git::git_status,
            commands::git::git_file_diff,
            commands::git::git_stage,
            commands::git::git_stage_all,
            commands::git::git_unstage,
            commands::git::git_discard,
            commands::git::git_commit_state,
            commands::git::git_commit,
            commands::git::git_push,
            commands::git::git_create_pr,
            commands::git::git_fetch,
            commands::git::git_pull,
            commands::git::git_publish,
            commands::git::git_add_remote,
            commands::files::write_workspace_file,
            commands::conversations::create_conversation,
            commands::conversations::list_conversations,
            commands::conversations::acp_start,
            commands::conversations::acp_send_prompt,
            commands::conversations::acp_cancel,
            commands::conversations::acp_resolve_permission,
            commands::conversations::acp_stop,
            commands::conversations::acp_history,
            commands::provider_accounts::add_provider_account,
            commands::provider_accounts::list_provider_accounts,
            commands::provider_accounts::remove_provider_account,
            commands::provider_accounts::set_default_provider_account,
            commands::provider_accounts::list_providers,
            commands::terminals::terminal_open,
            commands::terminals::terminal_open_agent,
            commands::terminals::terminal_write,
            commands::terminals::terminal_resize,
            commands::terminals::terminal_close,
            commands::terminals::terminal_surviving,
            commands::terminals::terminal_list_for_task,
            commands::terminals::terminal_tail,
            commands::settings::get_project_settings,
            commands::settings::update_project_settings,
            commands::settings::share_with_team,
            commands::view_state::get_view_state,
            commands::view_state::set_view_state,
            commands::search::search,
            commands::search::resource_sample,
            commands::search::get_resource_monitor_enabled,
            commands::search::set_resource_monitor_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
