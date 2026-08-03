//! ade-app — Tauri 2 shell.
//!
//! Wires the domain services (ARCHITECTURE §7), registers the E1-04 command
//! modules, and forwards internal events to the frontend.

mod app;
mod commands;

use app::App;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .setup(|app| {
            let app_state = App::init(std::env::var("ADE_DB_FILE").ok().as_deref())?;
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
