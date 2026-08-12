//! fartcode-app — Tauri 2 shell.
//!
//! Wires the domain services (ARCHITECTURE §7), registers the E1-04 command
//! modules, and forwards internal events to the frontend.

mod acp_events;
pub mod acp_runtime;
pub mod app;
pub mod byoi_tasks;
pub mod commands;
pub mod dispatch;
pub mod dossier_index;
pub mod dossiers;
pub mod indexer;
pub mod port_forwards;
pub mod remote_pty;
pub mod skills;
pub mod step_engine;
pub mod telemetry;
pub mod terminals;
mod watchers;

use std::sync::Arc;

use app::App;
use tauri::Manager;

/// Boot-time housekeeping (E1-08): prune orphaned view-state rows.
fn prune_view_state_on_boot(app: &App) {
    if let Err(e) = fartcode_core::view_state::prune_orphans(&app.db) {
        tracing::warn!(error = %e, "view-state prune failed (non-fatal)");
    }
}

/// macOS: with the Overlay title bar, the WKWebView can get stuck painting
/// short of the window after resize/minimize/fullscreen transitions
/// (tauri#14843) — a dead black strip at the window's bottom, or content
/// clipped past the edge. The stuck state can live INSIDE WebKit (frame
/// already correct, content view stale), so an identical setFrame is a
/// no-op; force a real size change instead: shrink the frame by 1pt, then
/// restore it, plus a layout pass. Both setFrames run in one runloop turn,
/// so nothing paints in between.
#[cfg(target_os = "macos")]
fn resync_webview_frames(window: &tauri::Window, reason: &'static str) {
    for webview in window.webviews() {
        let _ = webview.with_webview(move |platform| unsafe {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            use objc2_foundation::{NSRect, NSSize};
            let wv = platform.inner() as *mut AnyObject;
            if wv.is_null() {
                return;
            }
            let superview: *mut AnyObject = msg_send![&*wv, superview];
            if superview.is_null() {
                return;
            }
            let bounds: NSRect = msg_send![&*superview, bounds];
            let frame: NSRect = msg_send![&*wv, frame];
            let jiggle = NSRect {
                origin: bounds.origin,
                size: NSSize {
                    width: bounds.size.width,
                    height: (bounds.size.height - 1.0).max(1.0),
                },
            };
            let () = msg_send![&*wv, setFrame: jiggle];
            let () = msg_send![&*wv, setFrame: bounds];
            let () = msg_send![&*wv, setNeedsLayout: true];
            let () = msg_send![&*wv, layoutSubtreeIfNeeded];
            let () = msg_send![&*wv, setNeedsDisplay: true];
            // Dev-build ground truth for the desync hunt: one line per event
            // (frame vs superview bounds BEFORE the jiggle).
            #[cfg(debug_assertions)]
            {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/fartcode-webview.log")
                {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0);
                    let _ = writeln!(
                        f,
                        "{ts} {reason} bounds={}x{} frame={}x{}@{},{} mismatch={}",
                        bounds.size.width,
                        bounds.size.height,
                        frame.size.width,
                        frame.size.height,
                        frame.origin.x,
                        frame.origin.y,
                        (frame.size.height - bounds.size.height).abs() > 0.5
                            || (frame.size.width - bounds.size.width).abs() > 0.5
                    );
                }
            }
        });
    }
}

/// macOS: WebKit's compositor can clip the bottom of the page while every
/// queryable geometry (window, contentView, webview frame, even the page's
/// own innerHeight) reads correct — so the stuck state is unsensable and
/// view-level setFrame never reaches it. The one thing observed to heal it
/// is a REAL window resize, so force one: grow 1px, restore a beat later.
/// Imperceptible, but it drives a genuine windowDidResize → contentView →
/// WebKit viewport round-trip.
#[cfg(target_os = "macos")]
fn pulse_window_size(window: &tauri::Window, reason: &'static str) {
    if window.is_fullscreen().unwrap_or(false) {
        return; // set_size is a no-op/fight in native fullscreen
    }
    let Ok(size) = window.inner_size() else {
        return;
    };
    #[cfg(debug_assertions)]
    {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/fartcode-webview.log")
        {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let _ = writeln!(f, "{ts} pulse({reason}) {}x{}", size.width, size.height);
        }
    }
    let _ = window.set_size(tauri::PhysicalSize::new(size.width, size.height + 1));
    let w = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(40));
        let _ = w.set_size(tauri::PhysicalSize::new(size.width, size.height));
    });
}

/// Frontend-invoked rescue (tauri#14843 watchdog): the webview compares its
/// believed viewport against the window's true inner size and calls this on
/// mismatch. Escalates straight to the window-size pulse — the only lever
/// that reaches a stuck WebKit compositor — plus the view-frame jiggle for
/// the milder frame-level desync. No-op on other platforms.
#[tauri::command]
fn webview_resync(window: tauri::Window) {
    #[cfg(target_os = "macos")]
    {
        resync_webview_frames(&window, "watchdog");
        pulse_window_size(&window, "watchdog");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Log output for dev runs: defaults to INFO (override with RUST_LOG).
    // Without a subscriber every tracing call is silently dropped — the
    // best-effort agent/script launches were failing invisibly.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
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
            // tauri#14843: the webview frame desyncs from the window on
            // macOS (Overlay title bar) — resync on the events that follow
            // every way the stuck state can arise (resize, minimize/restore
            // via refocus, display/scale change).
            #[cfg(target_os = "macos")]
            match event {
                tauri::WindowEvent::Resized(_) => resync_webview_frames(window, "resized"),
                tauri::WindowEvent::Focused(true) => {
                    resync_webview_frames(window, "focused");
                    // Focus is the "user looks at the app" moment — heal the
                    // unsensable compositor clip right then.
                    pulse_window_size(window, "focused");
                }
                tauri::WindowEvent::ScaleFactorChanged { .. } => {
                    resync_webview_frames(window, "scale")
                }
                _ => {}
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(terminals) = window.try_state::<Arc<terminals::TerminalManager>>() {
                    terminals.detach_all();
                }
            }
        })
        .setup(|app| {
            let app_state = App::init(std::env::var("FARTCODE_DB_FILE").ok().as_deref())?;
            prune_view_state_on_boot(&app_state);
            app::spawn_event_forwarder(app.handle().clone(), app_state.event_bus.clone());
            indexer::spawn_search_indexer(app_state.clone());
            // E19-01: feature-dossier Timeline appender (ADR-0038 item 2) —
            // seeds its column map from the DB, then appends breadcrumbs
            // off the runtime worker for the app's lifetime.
            dossiers::spawn_dossier_timeline(app_state.clone());
            // E4-01: workspace file+git watches (boot backfill + provision/
            // delete subscription).
            watchers::spawn_workspace_watchers(
                app_state.db.clone(),
                app_state.fs_watch.clone(),
                app_state.event_bus.clone(),
            );
            // E4-09: PR sync scheduler — periodic background refresh of the
            // pull_requests cache (cursors in kv survive restarts).
            tauri::async_runtime::spawn(fartcode_git::pr_sync::run_scheduler(
                app_state.pr_sync.clone(),
                app_state.db.clone(),
                fartcode_git::pr_sync::SchedulerConfig::default(),
            ));
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
            // E12-05: the same registry boot rehydration uses, so a task's
            // terminal and its agent share one SSH connection per host.
            let terminal_manager = Arc::new(
                terminals::TerminalManager::new(app.handle().clone())
                    .with_remote(app_state.remote_pty.clone()),
            );
            app.manage(terminal_manager);
            // E2-11-4/5: ACP runtime — owns the SessionManager, spawns the
            // adapter per conversation, and emits `acp:update` /
            // `acp:transcript` / `acp:permission_request` via the events
            // emitter. `FARTCODE_ACP_ADAPTER` overrides the adapter binary for
            // tests (dev/test fixture only; production resolves per
            // provider).
            let acp_events = acp_events::TauriAcpEvents::new(app.handle().clone());
            let acp_runtime = crate::acp_runtime::AcpRuntime::new(
                app_state.conversations.clone(),
                app_state.tasks.clone(),
                app_state.db.clone(),
                app_state.provider_accounts.clone(),
                acp_events.clone(),
                match std::env::var("FARTCODE_ACP_ADAPTER").ok() {
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
            webview_resync,
            commands::projects::list_projects,
            commands::projects::create_project,
            commands::projects::delete_project,
            commands::remote_projects::clone_project,
            commands::remote_projects::remote_browse,
            commands::remote_projects::create_remote_project,
            commands::remote_projects::clone_remote_project,
            commands::remote_projects::new_remote_project,
            commands::remote_projects::ssh_connect,
            commands::remote_projects::ssh_disconnect,
            commands::remote_projects::ssh_connection_state,
            commands::remote_projects::ssh_connection_states,
            commands::remote_projects::remote_tasks_enabled,
            commands::port_forwards::port_forward_open,
            commands::port_forwards::port_forward_stop,
            commands::port_forwards::port_forward_list,
            commands::ssh_connections::ssh_connection_list,
            commands::ssh_connections::ssh_connection_save,
            commands::ssh_connections::ssh_connection_delete,
            commands::tasks::create_task,
            commands::tasks::list_project_branches,
            commands::tasks::provision_task,
            commands::tasks::list_tasks,
            commands::tasks::toggle_pin,
            commands::tasks::task_archive,
            commands::tasks::task_restore,
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
            commands::git::project_git_pull,
            commands::git::project_github_url,
            commands::github::pr_section_get,
            commands::github::pr_section_sync,
            commands::github::github_token_status,
            commands::github::github_token_import,
            commands::github::github_token_set,
            commands::github::github_token_clear,
            commands::line_comments::add_line_comment,
            commands::line_comments::agent_add_line_comment,
            commands::line_comments::list_line_comments,
            commands::line_comments::resolve_line_comment,
            commands::line_comments::delete_line_comment,
            commands::line_comments::create_task_from_comment,
            commands::issues::issue_create,
            commands::issues::issue_list,
            commands::issues::issue_update,
            commands::issues::issue_move,
            commands::issues::issue_delete,
            commands::issues::issue_link,
            commands::issues::issue_unlink,
            commands::issue_proposals::issue_parse_proposal,
            commands::issue_proposals::issue_apply_proposal,
            commands::issues::issue_dispatch,
            commands::issues::project_github_issues,
            commands::issues::issue_import_github,
            commands::columns::column_list,
            commands::columns::column_create,
            commands::columns::column_update,
            commands::columns::column_delete,
            commands::columns::column_reorder,
            commands::steps::issue_enter_column,
            commands::steps::step_confirm,
            commands::steps::step_parked_list,
            commands::steps::step_ledger_list,
            commands::files::write_workspace_file,
            commands::files::list_workspace_dir,
            commands::files::read_workspace_file,
            commands::conversations::create_conversation,
            commands::conversations::list_conversations,
            commands::conversations::list_project_conversations,
            commands::conversations::get_or_create_project_conversation,
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
            commands::provider_accounts::provider_auth_status,
            commands::provider_accounts::provider_auth_login,
            commands::terminals::terminal_open,
            commands::terminals::terminal_open_agent,
            commands::lifecycle::terminal_open_lifecycle,
            commands::terminals::terminal_write,
            commands::terminals::terminal_resize,
            commands::terminals::terminal_close,
            commands::terminals::terminal_surviving,
            commands::terminals::terminal_list_for_task,
            commands::terminals::terminal_tail,
            commands::settings::get_project_settings,
            commands::settings::update_project_settings,
            commands::settings::set_default_agent,
            commands::settings::share_with_team,
            commands::settings::project_settings_share,
            commands::settings::project_settings_provenance,
            commands::dependencies::host_dependency_list,
            commands::dependencies::host_dependency_install,
            commands::dependencies::host_dependency_update,
            commands::dependencies::host_dependency_registry_summary,
            commands::view_state::get_view_state,
            commands::view_state::set_view_state,
            commands::search::search,
            commands::dossiers::dossier_read,
            commands::dossiers::dossier_feature_rows,
            commands::search::resource_sample,
            commands::search::get_resource_monitor_enabled,
            commands::search::set_resource_monitor_enabled,
            // E19-04 (#73; ADR-0038 item 7): the local memory-value
            // signals. Read-only, and nothing behind it can leave the
            // machine.
            commands::telemetry::telemetry_memory_value,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
