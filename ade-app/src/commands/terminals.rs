//! Interactive terminal commands (E2-12): thin wrappers over the
//! TerminalManager. Output/exit arrive as `terminal:output` /
//! `terminal:exited` events, not command responses.

use std::sync::Arc;

use ade_core::db::Db;
use rusqlite::OptionalExtension;
use tauri::State;

use crate::app::App;
use crate::terminals::TerminalManager;

/// Resolves the task's working directory: worktree path when the task has
/// a workspace with a materialized path, else the project path.
fn resolve_task_cwd(db: &Arc<dyn Db>, task_id: &str) -> Result<String, String> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| "db connection mutex poisoned".to_string())?;
    let cwd: Option<String> = conn
        .query_row(
            "SELECT COALESCE(
                (SELECT path FROM workspaces WHERE id = t.workspace_id AND path IS NOT NULL AND path != ''),
                (SELECT p.path FROM projects p WHERE p.id = t.project_id)
             )
             FROM tasks t WHERE t.id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    cwd.ok_or_else(|| format!("task not found: {task_id}"))
}

/// Opens a shell in the task's workspace. Returns the terminal id.
#[tauri::command]
pub fn terminal_open(
    terminals: State<'_, Arc<TerminalManager>>,
    app: State<'_, Arc<App>>,
    task_id: String,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let cwd = resolve_task_cwd(&app.db, &task_id)?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    terminals
        .open(
            &task_id,
            &shell,
            &[],
            std::path::Path::new(&cwd),
            rows,
            cols,
        )
        .map_err(|e| e.to_string())
}

/// Opens an agent CLI (e.g. `omp`) in the task's workspace. The binary is
/// resolved from the provider registry's `binaries` list via PATH. Returns
/// the terminal id.
#[tauri::command]
pub fn terminal_open_agent(
    terminals: State<'_, Arc<TerminalManager>>,
    app: State<'_, Arc<App>>,
    task_id: String,
    agent: String,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let cwd = resolve_task_cwd(&app.db, &task_id)?;
    let provider = ade_providers::get(&agent).ok_or_else(|| format!("unknown agent: {agent}"))?;
    let binary = provider
        .binaries
        .iter()
        .find_map(|b| ade_core::pty::launcher::find_on_path(b))
        .ok_or_else(|| format!("agent not installed: {agent}"))?;
    terminals
        .open(
            &task_id,
            &binary.to_string_lossy(),
            &[],
            std::path::Path::new(&cwd),
            rows,
            cols,
        )
        .map_err(|e| e.to_string())
}

/// Types into the terminal (raw bytes pass through).
#[tauri::command]
pub fn terminal_write(
    terminals: State<'_, Arc<TerminalManager>>,
    terminal_id: String,
    data: String,
) -> Result<(), String> {
    terminals
        .write(&terminal_id, &data)
        .map_err(|e| e.to_string())
}

/// Resizes the terminal PTY.
#[tauri::command]
pub fn terminal_resize(
    terminals: State<'_, Arc<TerminalManager>>,
    terminal_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    terminals
        .resize(&terminal_id, cols, rows)
        .map_err(|e| e.to_string())
}

/// Kills the shell and drops the terminal.
#[tauri::command]
pub fn terminal_close(
    terminals: State<'_, Arc<TerminalManager>>,
    terminal_id: String,
) -> Result<(), String> {
    terminals.close(&terminal_id);
    Ok(())
}
