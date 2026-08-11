//! Interactive terminal commands (E2-12): thin wrappers over the
//! TerminalManager. Output/exit arrive as `terminal:output` /
//! `terminal:exited` events, not command responses.
//!
//! **Threading (#80):** the commands that spawn a PTY, scan PATH or shell
//! out to tmux (`terminal_open`, `terminal_open_agent`, `terminal_close`,
//! `terminal_surviving`) are `async fn` whose whole body runs inside
//! `tauri::async_runtime::spawn_blocking` — a non-async command body is
//! inlined into the invoke handler and would run on the macOS main thread,
//! stalling the run loop. The bodies live in the `*_blocking` fns (same
//! code, same order of side effects) so tests can drive them directly and
//! so no DB guard is ever held across an await. The cheap, purely
//! in-memory commands (`terminal_write`, `terminal_list_for_task`,
//! `terminal_tail`, `terminal_resize`) stay synchronous.

use std::sync::Arc;

use fartcode_core::db::Db;
use rusqlite::OptionalExtension;
use tauri::State;

use crate::app::App;
use crate::terminals::{TerminalManager, TerminalSpec};

/// Resolves the task's terminal context: owning project (id + path, for
/// project-settings reads) and working directory (worktree path when the
/// task has a workspace with a materialized path, else the project path).
pub(crate) struct TaskContext {
    pub(crate) project_id: String,
    pub(crate) project_path: String,
    pub(crate) cwd: String,
}

pub(crate) fn resolve_task_context(db: &Arc<dyn Db>, task_id: &str) -> Result<TaskContext, String> {
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

/// Program + args for a task's terminal (E2-13, #52): the project's
/// `taskStartupCommand` replaces the blank shell when set, run via
/// `sh -c '<command>'` so flags and compound commands work; `None`/blank
/// keeps today's behavior (`$SHELL`). The command is trimmed for the
/// blank check and execution.
fn terminal_program(
    settings: &fartcode_core::settings::ProjectSettings,
    shell: &str,
) -> (String, Vec<String>) {
    match settings.task_startup_command.as_deref() {
        Some(cmd) if !cmd.trim().is_empty() => (
            "/bin/sh".to_string(),
            vec!["-c".to_string(), cmd.trim().to_string()],
        ),
        _ => (shell.to_string(), Vec::new()),
    }
}

/// Opens a shell in the task's workspace. Returns the terminal id.
///
/// With the project's `taskStartupCommand` set (E2-13, #52), the terminal
/// runs that command instead of a blank shell — e.g. `omp` to start the
/// agent CLI in the task worktree on open. The shell runs under tmux when
/// the project's `tmux` setting is on AND a tmux binary resolves
/// (ADR-0025): the session survives an app crash and the next open
/// reattaches it.
#[tauri::command]
pub async fn terminal_open(
    terminals: State<'_, Arc<TerminalManager>>,
    app: State<'_, Arc<App>>,
    task_id: String,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let terminals = terminals.inner().clone();
    let app = app.inner().clone();
    // Off the IPC thread: a PTY fork/exec always, plus `tmux -V` and
    // `tmux list-sessions` when the project runs tmux.
    tauri::async_runtime::spawn_blocking(move || {
        terminal_open_blocking(&terminals, &app, &task_id, rows, cols)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `terminal_open`'s body, off the IPC thread. Generic over the Tauri
/// runtime so tests can drive it with `tauri::test::MockRuntime`.
pub fn terminal_open_blocking<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let ctx = resolve_task_context(&app.db, task_id)?;
    // Effective project settings: defaults < .fartCode.json < DB (precedence).
    // A failed read keeps today's behavior (no tmux, blank shell).
    let settings = app
        .settings
        .get_project_settings(&ctx.project_id, std::path::Path::new(&ctx.project_path))
        .unwrap_or_default();
    // E13-02 toggle resolution (reference parity): the app-wide
    // `tmuxByDefault` is stamped into the project settings row when that row
    // is created, so a read is just "the project's toggle, else off".
    // Changing the app default later deliberately does NOT retro-apply to a
    // project that already answered the question.
    let tmux = settings.tmux.unwrap_or(false);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let (program, args) = terminal_program(&settings, &shell);
    terminals
        .open(TerminalSpec {
            task_id,
            project_id: &ctx.project_id,
            agent: None,
            tmux,
            program: &program,
            args: &args,
            env: &[],
            remove: &[],
            cwd: std::path::Path::new(&ctx.cwd),
            rows,
            cols,
            lifecycle: None,
        })
        .map_err(|e| e.to_string())
}

/// Env vars stripped from an agent launch (ADR-0034): when the provider's
/// default account authenticates via CLI login (OAuth), the API-key env
/// vars (e.g. `ANTHROPIC_API_KEY`) must not reach the CLI — inherited or
/// injected, their presence flips it to API-key billing. Empty for
/// api-key accounts and providers without a default account.
pub(crate) fn agent_env_removals(app: &App, provider_id: &str) -> Vec<String> {
    match app.provider_accounts.resolve_removals(provider_id) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(provider_id, error = %e, "auth removal resolution failed");
            Vec::new()
        }
    }
}

/// Opens an agent CLI (e.g. `omp`) in the task's workspace. The binary is
/// resolved from the provider registry's `binaries` list via PATH. Returns
/// the terminal id. Agent terminals always run as plain PTYs — tmux slot
/// durability (ADR-0025) is for shell terminals.
///
/// ONE agent terminal per task (ADR-0033): a live agent terminal
/// reattaches instead of spawning a second — dispatch, ⌘⇧O, and the
/// comment-task flow all converge on the same session. Switching agents
/// means closing the agent tab first.
#[tauri::command]
pub async fn terminal_open_agent(
    terminals: State<'_, Arc<TerminalManager>>,
    app: State<'_, Arc<App>>,
    task_id: String,
    agent: String,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let terminals = terminals.inner().clone();
    let app = app.inner().clone();
    // Off the IPC thread: `find_on_path` walks PATH plus the common bin
    // dirs per candidate binary, then forks a PTY.
    tauri::async_runtime::spawn_blocking(move || {
        terminal_open_agent_blocking(&terminals, &app, &task_id, &agent, rows, cols)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `terminal_open_agent`'s body, off the IPC thread. Generic over the Tauri
/// runtime so tests can drive it with `tauri::test::MockRuntime`.
pub fn terminal_open_agent_blocking<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
    agent: &str,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    // Reattach before provider resolution: the existing session must be
    // reachable even if its binary left PATH meanwhile.
    if let Some(existing) = terminals.find_running_agent(task_id) {
        return Ok(existing);
    }
    let ctx = resolve_task_context(&app.db, task_id)?;
    let provider =
        fartcode_providers::get(agent).ok_or_else(|| format!("unknown agent: {agent}"))?;
    let binary = provider
        .binaries
        .iter()
        .find_map(|b| fartcode_core::pty::launcher::find_on_path(b))
        .ok_or_else(|| format!("agent not installed: {agent}"))?;
    let remove = agent_env_removals(app, agent);
    terminals
        .open(TerminalSpec {
            task_id,
            project_id: &ctx.project_id,
            agent: Some(agent),
            tmux: false,
            program: &binary.to_string_lossy(),
            args: &[],
            env: &[],
            remove: &remove,
            cwd: std::path::Path::new(&ctx.cwd),
            rows,
            cols,
            lifecycle: None,
        })
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

/// Lists the task's live terminals with agent tags (diff selection prompt
/// routing: agent terminal first, ACP chat fallback).
#[tauri::command]
pub fn terminal_list_for_task(
    terminals: State<'_, Arc<TerminalManager>>,
    task_id: String,
) -> Vec<crate::terminals::TerminalInfo> {
    terminals.list_for_task(&task_id)
}

/// Base64 scrollback tail replayed on frontend reattach (webview reload).
#[tauri::command]
pub fn terminal_tail(
    terminals: State<'_, Arc<TerminalManager>>,
    terminal_id: String,
) -> Option<String> {
    terminals.tail(&terminal_id)
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

/// Kills the shell and drops the terminal. With tmux durability this also
/// kills the tmux session (ADR-0028) — a closed tab is a teardown, not a
/// detach.
#[tauri::command]
pub async fn terminal_close(
    terminals: State<'_, Arc<TerminalManager>>,
    terminal_id: String,
) -> Result<(), String> {
    let terminals = terminals.inner().clone();
    // Off the IPC thread: a tmux-backed entry shells out to
    // `tmux kill-session` and waits on it.
    tauri::async_runtime::spawn_blocking(move || terminal_close_blocking(&terminals, &terminal_id))
        .await
        .map_err(|e| e.to_string())?
}

/// `terminal_close`'s body, off the IPC thread.
pub fn terminal_close_blocking<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    terminal_id: &str,
) -> Result<(), String> {
    terminals.close(terminal_id);
    Ok(())
}

/// Count of the task's tmux sessions that SURVIVE but this process does not
/// currently show (ADR-0028): a fresh restore opens enough terminals to
/// cover persisted tabs AND every survivor. 0 when tmux is off/absent.
#[tauri::command]
pub async fn terminal_surviving(
    terminals: State<'_, Arc<TerminalManager>>,
    app: State<'_, Arc<App>>,
    task_id: String,
) -> Result<usize, String> {
    let terminals = terminals.inner().clone();
    let app = app.inner().clone();
    // Off the IPC thread: with tmux on this runs `tmux list-sessions`
    // (plus the one-time `tmux -V` probe).
    tauri::async_runtime::spawn_blocking(move || {
        terminal_surviving_blocking(&terminals, &app, &task_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `terminal_surviving`'s body, off the IPC thread. Generic over the Tauri
/// runtime so tests can drive it with `tauri::test::MockRuntime`.
pub fn terminal_surviving_blocking<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
) -> Result<usize, String> {
    let ctx = resolve_task_context(&app.db, task_id)?;
    // Plain-PTY projects have nothing to survive; skip the tmux probe.
    let tmux = app
        .settings
        .get_project_settings(&ctx.project_id, std::path::Path::new(&ctx.project_path))
        .map(|s| s.tmux.unwrap_or(false))
        .unwrap_or(false);
    if !tmux {
        return Ok(0);
    }
    Ok(terminals.surviving_session_count(&ctx.project_id, task_id))
}

#[cfg(test)]
mod tests {
    use super::terminal_program;
    use fartcode_core::settings::ProjectSettings;

    #[test]
    fn blank_settings_run_the_shell() {
        let (program, args) = terminal_program(&ProjectSettings::default(), "/bin/zsh");
        assert_eq!(program, "/bin/zsh");
        assert!(args.is_empty());
    }

    #[test]
    fn startup_command_replaces_the_shell_via_sh_c() {
        let settings = ProjectSettings {
            task_startup_command: Some("omp --task".into()),
            ..Default::default()
        };
        let (program, args) = terminal_program(&settings, "/bin/zsh");
        assert_eq!(program, "/bin/sh");
        assert_eq!(args, vec!["-c".to_string(), "omp --task".to_string()]);
    }

    #[test]
    fn blank_or_whitespace_command_keeps_the_shell() {
        for cmd in [Some(String::new()), Some("   ".to_string()), None] {
            let settings = ProjectSettings {
                task_startup_command: cmd.clone(),
                ..Default::default()
            };
            let (program, args) = terminal_program(&settings, "/bin/zsh");
            assert_eq!(program, "/bin/zsh", "cmd={cmd:?}");
            assert!(args.is_empty(), "cmd={cmd:?}");
        }
    }

    #[test]
    fn command_is_trimmed_before_run() {
        let settings = ProjectSettings {
            task_startup_command: Some("  omp  ".into()),
            ..Default::default()
        };
        let (_, args) = terminal_program(&settings, "/bin/zsh");
        assert_eq!(args, vec!["-c".to_string(), "omp".to_string()]);
    }
}
