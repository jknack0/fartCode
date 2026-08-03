// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! ade-app — Tauri 2 shell.
//!
//! Phase 0 bootstrap: opens the main window and wires the event bridge.
//! Domain services (Db, SettingsStore, ...) are injected in E1-01+ via
//! `App::init` (see ARCHITECTURE.md §7). No Tauri commands are registered
//! yet — command modules arrive with their tickets.

fn main() {
    ade_app_lib::run()
}
