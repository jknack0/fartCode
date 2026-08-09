//! Add Task (left nav) launches the project's default agent in the new
//! worktree (PRD workflow: Add Task → spawn the chosen agent in its
//! worktree). The launch is best-effort — a missing binary leaves the task
//! standing for the frontend's terminal fallback; a present one yields
//! exactly ONE agent terminal (ADR-0033 dedupe).
//!
//! Plus the dogfood regression (2026-08-07): lifecycle setup scripts must
//! honor the `autoRunSetupScriptOnTaskCreation` flag — configured-but-
//! unflagged scripts stay manual.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_app_lib::app::App;
use fartcode_app_lib::commands::lifecycle::run_auto_lifecycle_scripts;
use fartcode_app_lib::commands::tasks::{launch_default_agent, launch_default_agent_after_setup};
use fartcode_app_lib::terminals::TerminalManager;
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{LocalProjectGroup, DEFAULT_AGENT, LOCAL_PROJECT};
use fartcode_core::tasks::operations::{
    CreateTaskParams, GitSetup, SourceBranchRef, TaskConfigParams,
};
use fartcode_core::tasks::WorkspaceTarget;

fn git_ok(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed in {:?}", args, dir);
}

fn make_repo(tmp: &tempfile::TempDir) -> PathBuf {
    let repo = tmp.path().join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git_ok(&repo, &["init", "-q"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@fartCode.dev",
            "commit",
            "-m",
            "init",
        ],
    );
    git_ok(&repo, &["branch", "-M", "main"]);
    std::fs::canonicalize(&repo).unwrap()
}

struct Fixture {
    _tmp: tempfile::TempDir,
    app: Arc<App>,
    project_id: String,
    project_path: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let app = App::init(Some(":memory:")).unwrap();
    app.settings
        .set(
            &LOCAL_PROJECT,
            LocalProjectGroup {
                default_projects_directory: tmp.path().join("repos").to_string_lossy().into_owned(),
                default_worktree_directory: tmp
                    .path()
                    .join("worktrees")
                    .to_string_lossy()
                    .into_owned(),
                write_agent_config_to_git_ignore: false,
            },
        )
        .unwrap();
    let repo = make_repo(&tmp);
    let project = app.projects.create_local(&repo, false).unwrap();
    Fixture {
        _tmp: tmp,
        app,
        project_id: project.id,
        project_path: repo,
    }
}

/// Mirrors the `create_task` command's creation path (worktree + branch).
fn create_task(fx: &Fixture, name: &str) -> String {
    let params = CreateTaskParams {
        id: None,
        project_id: fx.project_id.clone(),
        task_config: TaskConfigParams {
            name: name.to_string(),
            ..Default::default()
        },
        git: GitSetup::CreateBranch {
            branch_name: format!("fartCode/{name}"),
            from_branch: SourceBranchRef::local("main"),
            push_branch: false,
        },
        workspace: WorkspaceTarget::NewWorktree,
        automation_run_id: None,
    };
    fx.app
        .task_creation
        .create_with_provision(params)
        .expect("task creation")
        .task
        .id
}

/// Installs an executable `claude` stub into a fresh bin dir.
fn install_fake_claude(tmp: &tempfile::TempDir) -> PathBuf {
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("claude");
    std::fs::write(&exe, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// PATH is process-global and tests run in parallel threads — every test
/// that prepends the fake-claude bin dir serializes on this.
static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn add_task_launches_default_agent_and_dedupes() {
    let fx = fixture();
    let bin = install_fake_claude(&fx._tmp);
    let _path_guard = PATH_LOCK.lock().unwrap();
    let old_path = std::env::var_os("PATH").expect("PATH");
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&old_path).collect();
    dirs.insert(0, bin);
    std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());

    let task_id = create_task(&fx, "agent-launch");
    let manager = TerminalManager::new(tauri::test::mock_app().handle().clone());

    launch_default_agent(&manager, &fx.app, &task_id);
    let listed = manager.list_for_task(&task_id);
    let agents: Vec<_> = listed.iter().filter(|t| t.kind == "agent").collect();
    assert_eq!(agents.len(), 1, "exactly one agent terminal on task create");
    assert_eq!(agents[0].agent.as_deref(), Some("claude"));
    let first = agents[0].id.clone();

    // A second launch (racing frontend restore, ⌘⇧O) reattaches — ADR-0033.
    launch_default_agent(&manager, &fx.app, &task_id);
    let listed = manager.list_for_task(&task_id);
    let agents: Vec<_> = listed.iter().filter(|t| t.kind == "agent").collect();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].id, first);

    std::env::set_var("PATH", old_path);
}

#[test]
fn missing_agent_binary_leaves_the_task_standing() {
    let fx = fixture();
    fx.app
        .settings
        .set(&DEFAULT_AGENT, "no-such-agent".to_string())
        .unwrap();

    let task_id = create_task(&fx, "no-agent");
    let manager = TerminalManager::new(tauri::test::mock_app().handle().clone());

    // Best-effort: no panic, no terminal — the task stands for the
    // frontend's empty-pane surface.
    launch_default_agent(&manager, &fx.app, &task_id);
    assert!(manager.list_for_task(&task_id).is_empty());
}

/// The exact dogfood regression: a setup script configured with
/// `autoRunSetupScriptOnTaskCreation` UNSET must NOT auto-run at task
/// creation — the old hook ignored the flag and spawned omp on every new
/// task. Flag on → the script runs; flag off/unset → it stays manual.
#[test]
fn setup_script_auto_run_respects_the_project_flag() {
    let fx = fixture();

    // Configure a setup script WITHOUT enabling the auto-run flag.
    let mut settings = fx
        .app
        .settings
        .get_project_settings(&fx.project_id, &fx.project_path)
        .unwrap();
    settings.scripts = Some(fartcode_core::settings::Scripts {
        setup: Some("sleep 30".into()),
        run: None,
        teardown: None,
    });
    fx.app
        .settings
        .update_project_settings(&fx.project_id, &fx.project_path, &settings)
        .unwrap();

    let manager = TerminalManager::new(tauri::test::mock_app().handle().clone());

    let task1 = create_task(&fx, "gate-off");
    run_auto_lifecycle_scripts(&manager, &fx.app, &task1);
    let lifecycle: Vec<_> = manager
        .list_for_task(&task1)
        .into_iter()
        .filter(|t| t.kind == "lifecycle")
        .collect();
    assert!(
        lifecycle.is_empty(),
        "setup script must not auto-run with the flag unset"
    );

    // Flag on → the script runs.
    settings.auto_run_setup_script_on_task_creation = Some(true);
    fx.app
        .settings
        .update_project_settings(&fx.project_id, &fx.project_path, &settings)
        .unwrap();
    let task2 = create_task(&fx, "gate-on");
    run_auto_lifecycle_scripts(&manager, &fx.app, &task2);
    let lifecycle: Vec<_> = manager
        .list_for_task(&task2)
        .into_iter()
        .filter(|t| t.kind == "lifecycle")
        .collect();
    assert_eq!(lifecycle.len(), 1, "flag on → setup script auto-runs");
}

/// 7b "A failed setup blocks agent start": with auto-run setup on, the
/// default agent launches only AFTER the setup terminal exits 0 — a
/// non-zero exit skips the launch entirely (even with the agent binary
/// present on PATH).
#[test]
fn failed_setup_blocks_the_default_agent_launch() {
    let fx = fixture();
    let bin = install_fake_claude(&fx._tmp);
    let _path_guard = PATH_LOCK.lock().unwrap();
    let old_path = std::env::var_os("PATH").expect("PATH");
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&old_path).collect();
    dirs.insert(0, bin);
    std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());

    let mut settings = fx
        .app
        .settings
        .get_project_settings(&fx.project_id, &fx.project_path)
        .unwrap();
    settings.scripts = Some(fartcode_core::settings::Scripts {
        setup: Some("exit 3".into()),
        run: None,
        teardown: None,
    });
    settings.auto_run_setup_script_on_task_creation = Some(true);
    fx.app
        .settings
        .update_project_settings(&fx.project_id, &fx.project_path, &settings)
        .unwrap();

    let manager = TerminalManager::new(tauri::test::mock_app().handle().clone());

    // Failing setup (exit 3) → the deferred launch is skipped.
    let task1 = create_task(&fx, "setup-fails");
    let setup_id =
        run_auto_lifecycle_scripts(&manager, &fx.app, &task1).expect("setup auto-run spawned");
    launch_default_agent_after_setup(&manager, &fx.app, &task1, &setup_id);
    assert!(
        manager
            .list_for_task(&task1)
            .iter()
            .all(|t| t.kind != "agent"),
        "a failed setup must block the agent start"
    );

    // Clean setup (exit 0) → the agent launches after the script ends.
    settings.scripts = Some(fartcode_core::settings::Scripts {
        setup: Some("true".into()),
        run: None,
        teardown: None,
    });
    fx.app
        .settings
        .update_project_settings(&fx.project_id, &fx.project_path, &settings)
        .unwrap();
    let task2 = create_task(&fx, "setup-clean");
    let setup_id =
        run_auto_lifecycle_scripts(&manager, &fx.app, &task2).expect("setup auto-run spawned");
    launch_default_agent_after_setup(&manager, &fx.app, &task2, &setup_id);
    let agents: Vec<_> = manager
        .list_for_task(&task2)
        .into_iter()
        .filter(|t| t.kind == "agent")
        .collect();
    assert_eq!(agents.len(), 1, "clean setup → the agent launches");
    assert_eq!(agents[0].agent.as_deref(), Some("claude"));

    std::env::set_var("PATH", old_path);
}
