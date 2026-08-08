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
    let args = vec!["-c".to_string(), script];
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
pub fn run_auto_lifecycle_scripts<R: tauri::Runtime>(
    terminals: &TerminalManager<R>,
    app: &App,
    task_id: &str,
) {
    let ctx = match crate::commands::terminals::resolve_task_context(&app.db, task_id) {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::debug!(task_id, error = %e, "auto lifecycle scripts skipped");
            return;
        }
    };
    let settings = match app
        .settings
        .get_project_settings(&ctx.project_id, Path::new(&ctx.project_path))
    {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(task_id, error = %e, "auto lifecycle scripts skipped");
            return;
        }
    };
    for script_type in [LifecycleScriptType::Setup, LifecycleScriptType::Run] {
        if !auto_run_enabled(&settings, script_type) {
            continue;
        }
        if let Err(e) = spawn_lifecycle_script(terminals, app, task_id, script_type, 24, 80) {
            tracing::debug!(
                task_id,
                script_type = script_type.as_str(),
                error = %e,
                "auto lifecycle script skipped"
            );
        }
    }
}

/// Opens the task's lifecycle script terminal (E1-06): a tab that runs
/// setup/run/teardown in the worktree with the FARTCODE_* env contract. Returns
/// the terminal id — the frontend adds a tab keyed by it.
#[tauri::command]
pub fn terminal_open_lifecycle(
    terminals: State<'_, Arc<TerminalManager>>,
    app: State<'_, Arc<App>>,
    task_id: String,
    script_type: String,
    rows: u16,
    cols: u16,
) -> Result<String, String> {
    let script_type = LifecycleScriptType::parse(&script_type)
        .ok_or_else(|| format!("unknown lifecycle script type: {script_type}"))?;
    spawn_lifecycle_script(&terminals, &app, &task_id, script_type, rows, cols)
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
