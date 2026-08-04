//! Interactive terminal commands (E2-12): thin wrappers over the
//! TerminalManager. Output/exit arrive as `terminal:output` /
//! `terminal:exited` events, not command responses.

use std::sync::Arc;

use ade_core::db::Db;
use rusqlite::OptionalExtension;
use tauri::State;

use crate::app::App;
use crate::terminals::TerminalManager;

/// Resolves the task's terminal context: owning project (id + path, for
/// project-settings reads) and working directory (worktree path when the
/// task has a workspace with a materialized path, else the project path).
struct TaskContext {
    project_id: String,
    project_path: String,
    cwd: String,
}

fn resolve_task_context(db: &Arc<dyn Db>, task_id: &str) -> Result<TaskContext, String> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| "db connection mutex poisoned".to_string())?;
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT t.project_id,
                    p.path,
                    COALESCE(
                        (SELECT path FROM workspaces WHERE id = t.workspace_id AND path IS NOT NULL AND path != ''),
                        p.path
                    )
             FROM tasks t JOIN projects p ON p.id = t.project_id
             WHERE t.id = ?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    row.map(|(project_id, project_path, cwd)| TaskContext {
        project_id,
        project_path,
        cwd,
    })
    .ok_or_else(|| format!("task not found: {task_id}"))
}

/// Opens a shell in the task's workspace. Returns the terminal id.
///
/// The shell runs under tmux when the project's `tmux` setting is on AND a
/// tmux binary resolves (ADR-0025): the session survives an app crash and
/// the next open reattaches it.
#[tauri::command]
pub fn terminal_open(
    terminals: State<'_, Arc<TerminalManager>>,
    app: State<'_, Arc<App>>,
    task_id: String,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let ctx = resolve_task_context(&app.db, &task_id)?;
    // Effective tmux flag: defaults < .ade.json < DB (settings precedence).
    let tmux = app
        .settings
        .get_project_settings(&ctx.project_id, std::path::Path::new(&ctx.project_path))
        .map(|s| s.tmux.unwrap_or(false))
        .unwrap_or(false);
    terminals
        .open(
            &task_id,
            &ctx.project_id,
            tmux,
            std::path::Path::new(&ctx.cwd),
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
