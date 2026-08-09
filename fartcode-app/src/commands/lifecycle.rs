//! Lifecycle script terminals (E1-06): run a project's setup/run/teardown
//! scripts as interactive task terminals.
//!
//! Each run is a REAL terminal tab (`terminal_open_lifecycle`): the script
//! runs via `sh -c '<script>'` in the task worktree with the FARTCODE_* env
//! contract, output streams to the tab like any other terminal, and the
//! entry is retained after exit so a later tab attach still replays the
//! tail. In-flight runs of the same type dedupe (rerun reattaches instead
//! of spawning a second script).
//!
//! Auto-run (settings `autoRunSetupScriptOnTaskCreation` /
//! `autoRunRunScriptOnTaskCreation`) fires at task creation from
//! `create_task` / `create_task_from_comment`: the terminal is spawned
//! backend-side, and the task view surfaces it as a tab on open.
//!
//! **Threading (#80):** `terminal_open_lifecycle` is `async fn` and runs
//! `spawn_lifecycle_script` inside `tauri::async_runtime::spawn_blocking` —
//! a non-async command body is inlined into the invoke handler and would
//! read settings and fork a PTY on the macOS main thread, stalling the run
//! loop. `spawn_lifecycle_script` itself stays synchronous: the auto-run
//! hook already calls it from off-thread contexts.

use std::path::Path;
use std::sync::Arc;

use fartcode_core::settings::{DefaultBranch, ProjectSettings};
use fartcode_core::tasks::TaskStore;
use fartcode_core::terminals::lifecycle::{task_env, LifecycleScriptType};
use tauri::State;

use crate::app::App;
use crate::commands::terminals::resolve_task_context;
use crate::terminals::{TerminalManager, TerminalSpec};

/// Combined script text for `script_type` from effective project settings,
/// with `shellSetup` prepended (reference pty-spawn-platform prepends
/// shellSetup to shell-line commands). `None` when the script is unset or
/// blank — nothing to run.
pub fn lifecycle_script_text(
    settings: &ProjectSettings,
    script_type: LifecycleScriptType,
) -> Option<String> {
    let script = match script_type {
        LifecycleScriptType::Setup => settings.scripts.as_ref()?.setup.as_deref(),
        LifecycleScriptType::Run => settings.scripts.as_ref()?.run.as_deref(),
        LifecycleScriptType::Teardown => settings.scripts.as_ref()?.teardown.as_deref(),
    }?;
    let script = script.trim();
    if script.is_empty() {
        return None;
    }
    match settings.shell_setup.as_deref() {
        Some(setup) if !setup.trim().is_empty() => Some(format!("{}\n{}", setup.trim(), script)),
        _ => Some(script.to_string()),
    }
}

/// Whether the project auto-runs `script_type` on task creation. Teardown
/// has no auto-run setting (it is manual only).
pub fn auto_run_enabled(settings: &ProjectSettings, script_type: LifecycleScriptType) -> bool {
    match script_type {
        LifecycleScriptType::Setup => settings.auto_run_setup_script_on_task_creation,
        LifecycleScriptType::Run => settings.auto_run_run_script_on_task_creation,
        LifecycleScriptType::Teardown => Some(false),
    }
    .unwrap_or(false)
}

/// 7b log body: prepends a `printf` that echoes each non-blank script line
/// ANSI-dim with a `"$ "` prefix before the script runs, so the drawer
/// reads as a command log. Lines are `single_quote`d (E2-02 canonical
/// quoting) and passed as `printf` ARGS — the format is reused per arg and
/// the lines are never re-executed or escape-interpreted.
fn with_command_echo(script: &str) -> String {
    let lines: Vec<&str> = script.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return script.to_string();
    }
    let mut echo = String::from("printf '\\033[2m$ %s\\033[0m\\n'");
    for line in lines {
        echo.push(' ');
        echo.push_str(&fartcode_core::shell_escape::single_quote(line));
    }
    format!("{echo}\n{script}")
}

/// FARTCODE_DEFAULT_BRANCH for the env contract: the project's configured
/// default branch name, else "" (task_env falls back to "main").
fn default_branch_name(settings: &ProjectSettings) -> String {
    match &settings.default_branch {
        Some(DefaultBranch::Name(name)) => name.clone(),
        Some(DefaultBranch::Remote { name, .. }) => name.clone(),
        None => String::new(),
    }
}

/// Spawns (or reattaches) the task's lifecycle script terminal of
/// `script_type`. Returns the terminal id.
pub(crate) fn spawn_lifecycle_script<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
    script_type: LifecycleScriptType,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let ctx = resolve_task_context(&app.db, task_id)?;
    let settings = app
        .settings
        .get_project_settings(&ctx.project_id, Path::new(&ctx.project_path))
        .map_err(|e| e.to_string())?;
    let Some(script) = lifecycle_script_text(&settings, script_type) else {
        return Err(format!(
            "no {} script configured for this project",
            script_type.as_str()
        ));
    };

    // Dedupe: an in-flight run of the same type reattaches (the frontend
    // focuses the existing tab instead of stacking a second script).
    if let Some(existing) = terminals.find_running_lifecycle(task_id, script_type.as_str()) {
        return Ok(existing);
    }

    let task = app
        .tasks
        .get(task_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("task not found: {task_id}"))?;
    // Env contract: port seed is the worktree path so tasks in the same
    // project get isolated FARTCODE_PORTs (two tasks never share a port).
    let env = task_env(
        task_id,
        &task.name,
        Path::new(&ctx.cwd),
        Path::new(&ctx.project_path),
        &default_branch_name(&settings),
        &ctx.cwd,
    );
    let program = "/bin/sh".to_string();
    // 7b: the drawer opens on the script's own command lines, dim, "$ "-
    // prefixed — echoed up front, then the script runs unchanged (the
    // wrapper never alters the script's exit code).
    let args = vec!["-c".to_string(), with_command_echo(&script)];
    terminals
        .open(TerminalSpec {
            task_id,
            project_id: &ctx.project_id,
            agent: None,
            tmux: false,
            program: &program,
            args: &args,
            env: &env,
            remove: &[],
            cwd: Path::new(&ctx.cwd),
            rows,
            cols,
            lifecycle: Some(script_type),
        })
        .map_err(|e| e.to_string())
}

/// Auto-run hook (E1-06): fires setup/run at task creation ONLY when the
/// project's `autoRunSetupScriptOnTaskCreation` /
/// `autoRunRunScriptOnTaskCreation` flags are on (default off — scripts
/// configured but not flagged stay manual). Best-effort — a failed spawn
/// (no script configured, worktree missing) must never fail the creation
/// itself.
///
/// Returns the SETUP terminal id when a setup script auto-ran — 7b's "a
/// failed setup blocks agent start": `create_task` defers the default-agent
/// launch until that terminal exits 0. `None` when setup did not auto-run
/// (behaviour unchanged for the caller).
pub fn run_auto_lifecycle_scripts<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
) -> Option<String> {
    let ctx = match crate::commands::terminals::resolve_task_context(&app.db, task_id) {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::debug!(task_id, error = %e, "auto lifecycle scripts skipped");
            return None;
        }
    };
    let settings = match app
        .settings
        .get_project_settings(&ctx.project_id, Path::new(&ctx.project_path))
    {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(task_id, error = %e, "auto lifecycle scripts skipped");
            return None;
        }
    };
    let mut setup_terminal: Option<String> = None;
    for script_type in [LifecycleScriptType::Setup, LifecycleScriptType::Run] {
        if !auto_run_enabled(&settings, script_type) {
            continue;
        }
        match spawn_lifecycle_script(terminals, app, task_id, script_type, 24, 80) {
            Ok(id) if script_type == LifecycleScriptType::Setup => setup_terminal = Some(id),
            Ok(_) => {}
            Err(e) => tracing::debug!(
                task_id,
                script_type = script_type.as_str(),
                error = %e,
                "auto lifecycle script skipped"
            ),
        }
    }
    setup_terminal
}

/// Opens the task's lifecycle script terminal (E1-06): a tab that runs
/// setup/run/teardown in the worktree with the FARTCODE_* env contract. Returns
/// the terminal id — the frontend adds a tab keyed by it.
#[tauri::command]
pub async fn terminal_open_lifecycle(
    terminals: State<'_, Arc<TerminalManager>>,
    app: State<'_, Arc<App>>,
    task_id: String,
    script_type: String,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let terminals = terminals.inner().clone();
    let app = app.inner().clone();
    // Off the IPC thread: the spawn reads the project's `.fartCode.json`
    // settings and forks a PTY running `sh -c '<script>'`.
    tauri::async_runtime::spawn_blocking(move || {
        terminal_open_lifecycle_blocking(&terminals, &app, &task_id, &script_type, rows, cols)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `terminal_open_lifecycle`'s body, off the IPC thread. Generic over the
/// Tauri runtime so tests can drive it with `tauri::test::MockRuntime`.
pub fn terminal_open_lifecycle_blocking<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
    script_type: &str,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let script_type = LifecycleScriptType::parse(script_type)
        .ok_or_else(|| format!("unknown lifecycle script type: {script_type}"))?;
    spawn_lifecycle_script(terminals, app, task_id, script_type, rows, cols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::settings::Scripts;

    fn settings_with(scripts: Scripts) -> ProjectSettings {
        ProjectSettings {
            scripts: Some(scripts),
            ..Default::default()
        }
    }

    #[test]
    fn blank_settings_run_nothing() {
        for t in [
            LifecycleScriptType::Setup,
            LifecycleScriptType::Run,
            LifecycleScriptType::Teardown,
        ] {
            assert_eq!(lifecycle_script_text(&ProjectSettings::default(), t), None);
        }
    }

    #[test]
    fn unset_or_blank_script_is_none() {
        let settings = settings_with(Scripts {
            setup: Some("   ".into()),
            run: None,
            teardown: Some("echo bye".into()),
        });
        assert_eq!(
            lifecycle_script_text(&settings, LifecycleScriptType::Setup),
            None
        );
        assert_eq!(
            lifecycle_script_text(&settings, LifecycleScriptType::Run),
            None
        );
        assert_eq!(
            lifecycle_script_text(&settings, LifecycleScriptType::Teardown),
            Some("echo bye".into())
        );
    }

    #[test]
    fn script_is_trimmed() {
        let settings = settings_with(Scripts {
            setup: Some("  echo hi\n  ".into()),
            ..Default::default()
        });
        assert_eq!(
            lifecycle_script_text(&settings, LifecycleScriptType::Setup),
            Some("echo hi".into())
        );
    }

    #[test]
    fn shell_setup_is_prepended() {
        let settings = ProjectSettings {
            shell_setup: Some("source .envrc".into()),
            scripts: Some(Scripts {
                setup: Some("npm install".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            lifecycle_script_text(&settings, LifecycleScriptType::Setup),
            Some("source .envrc\nnpm install".into())
        );
    }

    #[test]
    fn blank_shell_setup_is_not_prepended() {
        let settings = ProjectSettings {
            shell_setup: Some("  ".into()),
            scripts: Some(Scripts {
                setup: Some("npm install".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            lifecycle_script_text(&settings, LifecycleScriptType::Setup),
            Some("npm install".into())
        );
    }

    #[test]
    fn command_echo_prefixes_each_line_and_keeps_the_script() {
        let wrapped = with_command_echo("npm install\n\nnpm run build");
        // One printf, format reused per line, blank lines skipped.
        assert!(wrapped.starts_with("printf '\\033[2m$ %s\\033[0m\\n'"));
        assert!(wrapped.contains("'npm install'"));
        assert!(wrapped.contains("'npm run build'"));
        // The original script runs unchanged after the echo.
        assert!(wrapped.ends_with("\nnpm install\n\nnpm run build"));
    }

    #[test]
    fn command_echo_quotes_are_shell_safe() {
        // A line with an embedded single quote must round-trip through the
        // canonical `'\''` escape, never re-executing.
        let wrapped = with_command_echo("echo \"it's fine\"");
        assert!(wrapped.contains("'echo \"it'\\''s fine\"'"));
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", &wrapped])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        // Dim echo line, then the script's own output.
        assert!(text.contains("\u{1b}[2m$ echo \"it's fine\"\u{1b}[0m"));
        assert!(text.contains("it's fine\n"));
    }

    #[test]
    fn auto_run_flags_gate_setup_and_run_only() {
        let settings = ProjectSettings {
            auto_run_setup_script_on_task_creation: Some(true),
            ..Default::default()
        };
        assert!(auto_run_enabled(&settings, LifecycleScriptType::Setup));
        assert!(!auto_run_enabled(&settings, LifecycleScriptType::Run));
        assert!(!auto_run_enabled(&settings, LifecycleScriptType::Teardown));
    }
}
