//! #80 (UI thread): the tasks + terminals commands that block on git, the
//! network, PTY spawns or tmux now run their body inside
//! `tauri::async_runtime::spawn_blocking` instead of inline on the IPC
//! (macOS main) thread.
//!
//! Two things are proven here:
//!
//! 1. **Behaviour is unchanged.** Each command's body was lifted verbatim
//!    into a `*_blocking` fn; these tests drive those bodies against a real
//!    git repo + real PTYs and assert the same results, the same error
//!    strings, and the same emitted events as before the conversion.
//! 2. **The body actually leaves the calling thread.** For the two commands
//!    whose state is constructible under `tauri::test::MockRuntime`
//!    (`provision_task`, `list_project_branches` take only `Arc<App>`), the
//!    real `async` command is awaited on a SINGLE-THREADED tokio runtime
//!    with a cooperative ticker task spawned alongside. A ticker can only
//!    advance while the command future is parked — with the old inline
//!    body it would still read 0 when the command returns.
//!
//! Every wait here is bounded (5 s ceilings, no unbounded joins).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fartcode_app_lib::acp_runtime::AcpRuntime;
use fartcode_app_lib::app::App;
use fartcode_app_lib::commands::lifecycle::terminal_open_lifecycle_blocking;
use fartcode_app_lib::commands::tasks::{
    create_task_blocking, delete_task_blocking, list_project_branches,
    list_project_branches_blocking, provision_task, provision_task_blocking,
};
use fartcode_app_lib::commands::terminals::{
    terminal_close_blocking, terminal_open_agent_blocking, terminal_open_blocking,
    terminal_surviving_blocking,
};
use fartcode_app_lib::terminals::TerminalManager;
use fartcode_core::events::{EventBus, InternalEvent};
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{LocalProjectGroup, Scripts, DEFAULT_AGENT, LOCAL_PROJECT};
use fartcode_core::tasks::TaskStore;
use tauri::Manager as _;

// -- compile-time guard: every converted command stays `async` -----------

/// A non-async `#[tauri::command]` compiles to `ExecutionContext::Blocking`
/// — tauri-macros inlines its body into the invoke handler, which runs on
/// the IPC (macOS main) thread. These assertions only type-check while the
/// commands return a `Future`, so reverting any of them to a plain `fn`
/// breaks the build instead of silently re-freezing the window.
///
/// This is the only coverage available for the seven commands that take
/// `State<'_, Arc<TerminalManager>>`: `TerminalManager` defaults to
/// `tauri::Wry`, and an `AppHandle<Wry>` cannot be built under
/// `tauri::test::MockRuntime`, so their `State` is unconstructible in a
/// test. Their bodies are covered through the `*_blocking` fns below.
#[allow(dead_code)]
fn commands_are_async_fns() {
    fn returns_future<Args, Fut: std::future::Future>(_f: impl FnOnce(Args) -> Fut) {}
    // Tuple-shaped wrappers: the commands take 2-7 args, so each is fed
    // through a closure that destructures one tuple.
    returns_future(|(a, t, p, n, w, b)| {
        fartcode_app_lib::commands::tasks::create_task(a, t, p, n, w, b)
    });
    returns_future(|(a, p)| list_project_branches(a, p));
    returns_future(|(a, t)| provision_task(a, t));
    returns_future(|(a, t, c, p, i, w, b)| {
        fartcode_app_lib::commands::tasks::delete_task(a, t, c, p, i, w, b)
    });
    returns_future(|(t, a, i, r, c)| {
        fartcode_app_lib::commands::terminals::terminal_open(t, a, i, r, c)
    });
    returns_future(|(t, a, i, g, r, c)| {
        fartcode_app_lib::commands::terminals::terminal_open_agent(t, a, i, g, r, c)
    });
    returns_future(|(t, i)| fartcode_app_lib::commands::terminals::terminal_close(t, i));
    returns_future(|(t, a, i)| fartcode_app_lib::commands::terminals::terminal_surviving(t, a, i));
    returns_future(|(t, a, i, s, r, c)| {
        fartcode_app_lib::commands::lifecycle::terminal_open_lifecycle(t, a, i, s, r, c)
    });
}

// -- fixture -------------------------------------------------------------

fn git_ok(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {dir:?}");
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
    tmp: tempfile::TempDir,
    app: Arc<App>,
    project_id: String,
    project_path: PathBuf,
}

/// Real repo + real App on an in-memory DB. `defaultAgent` is pinned to a
/// provider that cannot resolve so the best-effort agent launch inside
/// `create_task_blocking` never spawns a real CLI off the developer's PATH
/// (the tests that DO want a launch install a fake `claude` explicitly).
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
    app.settings
        .set(&DEFAULT_AGENT, "no-such-agent".to_string())
        .unwrap();
    let repo = make_repo(&tmp);
    let project = app.projects.create_local(&repo, false).unwrap();
    Fixture {
        tmp,
        app,
        project_id: project.id,
        project_path: repo,
    }
}

fn manager() -> Arc<TerminalManager<tauri::test::MockRuntime>> {
    Arc::new(TerminalManager::new(
        tauri::test::mock_app().handle().clone(),
    ))
}

/// A no-op ACP runtime: `delete_task_blocking` only calls `stop_task`, and
/// the fixture's tasks own no conversations, so the adapter never resolves.
fn acp_runtime(app: &Arc<App>) -> Arc<AcpRuntime> {
    struct NoEvents;
    impl fartcode_acp::session::SessionEvents for NoEvents {
        fn update(&self, _: &str, _: &fartcode_acp::client::SessionUpdateEvent) {}
        fn transcript_changed(&self, _: &str, _: &fartcode_acp::LiveModels) {}
        fn permission_requested(
            &self,
            _: &str,
            _: &fartcode_acp::session::PermissionRequestedEvent,
        ) {
        }
    }
    AcpRuntime::new(
        app.conversations.clone(),
        app.tasks.clone(),
        app.db.clone(),
        app.provider_accounts.clone(),
        Arc::new(NoEvents),
        Arc::new(|provider: &str| {
            Err(fartcode_acp::Error::InvalidState(format!(
                "no adapter in tests: {provider}"
            )))
        }),
    )
}

/// Drains everything already on the bus (the sends are synchronous, so by
/// the time a command returns its events are queued) — bounded by the
/// channel, never blocking.
fn drained(rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) -> Vec<InternalEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

/// On-disk path of the task's materialized worktree (the row `provision`
/// wrote). Panics when the task has no workspace path — every fixture task
/// here is created through the provisioning path.
fn worktree_path(fx: &Fixture, task_id: &str) -> PathBuf {
    let workspace_id = fx
        .app
        .tasks
        .get(task_id)
        .unwrap()
        .and_then(|t| t.workspace_id)
        .expect("workspace id");
    let conn = fx.app.db.conn().lock().unwrap();
    let path: Option<String> = conn
        .query_row(
            "SELECT path FROM workspaces WHERE id = ?1",
            [workspace_id],
            |r| r.get(0),
        )
        .unwrap();
    PathBuf::from(path.expect("materialized worktree path"))
}

/// Polls `f` every 25 ms up to 5 s.
fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{what} did not happen within 5s");
}

fn install_fake_claude(tmp: &tempfile::TempDir) -> PathBuf {
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("claude");
    std::fs::write(&exe, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// PATH is process-global and tests run in parallel threads.
static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `f` with `bin` prepended to PATH, restoring it afterwards.
fn with_path_dir<T>(bin: PathBuf, f: impl FnOnce() -> T) -> T {
    let _guard = PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let old = std::env::var_os("PATH").expect("PATH");
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&old).collect();
    dirs.insert(0, bin);
    std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
    let out = f();
    std::env::set_var("PATH", old);
    out
}

// -- create_task ---------------------------------------------------------

#[test]
fn create_task_provisions_a_worktree_and_emits_task_created() {
    let fx = fixture();
    let terminals = manager();
    let mut events = fx.app.event_bus.subscribe();

    let dto = create_task_blocking(
        &fx.app,
        &terminals,
        &fx.project_id,
        "offload me",
        None,
        None,
    )
    .expect("create_task");

    assert_eq!(dto.name, "offload me");
    assert_eq!(dto.project_id, fx.project_id);
    // create_with_provision materialized the worktree (E2-04).
    let workspace_id = dto.workspace_id.clone().expect("workspace id");
    let task = fx.app.tasks.get(&dto.id).unwrap().expect("task row");
    assert_eq!(task.workspace_id.as_deref(), Some(workspace_id.as_str()));

    // Same event as before the conversion, with the same payload.
    let created: Vec<_> = drained(&mut events)
        .into_iter()
        .filter_map(|e| match e {
            InternalEvent::TaskCreated { id, name, .. } => Some((id, name)),
            _ => None,
        })
        .collect();
    assert_eq!(created, vec![(dto.id.clone(), "offload me".to_string())]);

    // defaultAgent cannot resolve → best-effort launch is a no-op, the
    // task still stands (unchanged behaviour).
    assert!(terminals.list_for_task(&dto.id).is_empty());
}

#[test]
fn create_task_maps_workspace_and_branch_the_same_way() {
    let fx = fixture();
    let terminals = manager();

    // project-root: no worktree, the live checkout is the workspace.
    let root = create_task_blocking(
        &fx.app,
        &terminals,
        &fx.project_id,
        "root task",
        Some("project-root"),
        None,
    )
    .expect("project-root create");
    assert_eq!(root.name, "root task");

    // An explicit branch is reused rather than minted.
    git_ok(&fx.project_path, &["branch", "feature/pick-me"]);
    let picked = create_task_blocking(
        &fx.app,
        &terminals,
        &fx.project_id,
        "branch task",
        Some("new-worktree"),
        Some("feature/pick-me"),
    )
    .expect("branch create");
    assert!(picked.workspace_id.is_some());

    // Error strings are byte-identical to the pre-conversion command.
    let err = create_task_blocking(
        &fx.app,
        &terminals,
        &fx.project_id,
        "bad",
        Some("nowhere"),
        None,
    )
    .unwrap_err();
    assert_eq!(err, "invalid workspace target: nowhere");

    let err =
        create_task_blocking(&fx.app, &terminals, "ghost-project", "bad", None, None).unwrap_err();
    assert_eq!(err, "project not found: ghost-project");
}

#[test]
fn create_task_still_launches_the_default_agent() {
    let fx = fixture();
    fx.app
        .settings
        .set(&DEFAULT_AGENT, "claude".to_string())
        .unwrap();
    let bin = install_fake_claude(&fx.tmp);
    let terminals = manager();

    let dto = with_path_dir(bin, || {
        create_task_blocking(
            &fx.app,
            &terminals,
            &fx.project_id,
            "agent task",
            None,
            None,
        )
        .expect("create_task")
    });

    // PRD workflow: Add Task → exactly ONE agent terminal (ADR-0033).
    let agents: Vec<_> = terminals
        .list_for_task(&dto.id)
        .into_iter()
        .filter(|t| t.kind == "agent")
        .collect();
    assert_eq!(agents.len(), 1, "create_task must launch the default agent");
    assert_eq!(agents[0].agent.as_deref(), Some("claude"));
    terminals.close(&agents[0].id);
}

// -- provision_task ------------------------------------------------------

#[test]
fn provision_task_is_idempotent_and_keeps_its_error_string() {
    let fx = fixture();
    let terminals = manager();
    let dto = create_task_blocking(
        &fx.app,
        &terminals,
        &fx.project_id,
        "reprovision",
        None,
        None,
    )
    .expect("create_task");

    // Reuse path: already provisioned → Ok(()) with no change.
    assert_eq!(provision_task_blocking(&fx.app, &dto.id), Ok(()));
    assert_eq!(provision_task_blocking(&fx.app, &dto.id), Ok(()));

    let err = provision_task_blocking(&fx.app, "ghost-task").unwrap_err();
    assert!(
        err.contains("ghost-task"),
        "unknown task must still surface its id: {err}"
    );
}

/// The offload proof for `provision_task`: awaited on a single-threaded
/// runtime, a cooperative task spawned alongside it MUST have advanced by
/// the time the command resolves. Inline (`ExecutionContext::Blocking`)
/// bodies never yield, so the ticker would read 0.
#[test]
fn provision_task_command_yields_while_it_works() {
    let fx = fixture();
    let terminals = manager();
    let dto = create_task_blocking(&fx.app, &terminals, &fx.project_id, "yielder", None, None)
        .expect("create_task");

    let mock = tauri::test::mock_app();
    mock.manage(fx.app.clone());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ticks = Arc::new(AtomicUsize::new(0));
    let observed = rt.block_on(async {
        let t = ticks.clone();
        // Bounded ticker — it cannot outlive the runtime or spin forever.
        let ticker = tokio::spawn(async move {
            for _ in 0..10_000 {
                t.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        });
        let result = provision_task(mock.state(), dto.id.clone()).await;
        let observed = ticks.load(Ordering::SeqCst);
        ticker.abort();
        assert_eq!(result, Ok(()), "provision must still succeed");
        observed
    });
    assert!(
        observed > 0,
        "provision_task ran inline — the calling runtime never advanced"
    );

    // The error path resolves through the same offload, unchanged.
    let err = rt
        .block_on(provision_task(mock.state(), "ghost-task".to_string()))
        .unwrap_err();
    assert_eq!(
        err,
        provision_task_blocking(&fx.app, "ghost-task").unwrap_err()
    );
}

// -- list_project_branches -----------------------------------------------

#[test]
fn list_project_branches_command_matches_its_blocking_body_and_yields() {
    let fx = fixture();
    git_ok(&fx.project_path, &["branch", "extra/one"]);

    let direct = list_project_branches_blocking(&fx.app, &fx.project_id).expect("branches");
    let names: Vec<&str> = direct.iter().map(|b| b.name.as_str()).collect();
    assert!(names.contains(&"main"), "branches: {names:?}");
    assert!(names.contains(&"extra/one"), "branches: {names:?}");

    let mock = tauri::test::mock_app();
    mock.manage(fx.app.clone());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ticks = Arc::new(AtomicUsize::new(0));
    let (via_command, observed) = rt.block_on(async {
        let t = ticks.clone();
        let ticker = tokio::spawn(async move {
            for _ in 0..10_000 {
                t.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        });
        let out = list_project_branches(mock.state(), fx.project_id.clone()).await;
        let observed = ticks.load(Ordering::SeqCst);
        ticker.abort();
        (out, observed)
    });
    // Identical wire payload, off the calling thread.
    assert_eq!(via_command.expect("branches"), direct);
    assert!(
        observed > 0,
        "list_project_branches ran inline — the calling runtime never advanced"
    );
}

// -- terminal_open / _agent / _close / _surviving ------------------------

#[test]
fn terminal_open_spawns_a_shell_and_close_drops_it() {
    let fx = fixture();
    let terminals = manager();
    let dto = create_task_blocking(&fx.app, &terminals, &fx.project_id, "shell", None, None)
        .expect("create_task");

    let id = terminal_open_blocking(&terminals, &fx.app, &dto.id, 24, 80).expect("terminal_open");
    let listed = terminals.list_for_task(&dto.id);
    let info = listed.iter().find(|t| t.id == id).expect("listed");
    assert_eq!(info.kind, "shell");
    assert!(info.agent.is_none());

    assert_eq!(terminal_close_blocking(&terminals, &id), Ok(()));
    assert!(!terminals.list_for_task(&dto.id).iter().any(|t| t.id == id));
    // Idempotent, like before.
    assert_eq!(terminal_close_blocking(&terminals, &id), Ok(()));

    let err = terminal_open_blocking(&terminals, &fx.app, "ghost-task", 24, 80).unwrap_err();
    assert_eq!(err, "task not found: ghost-task");
}

#[test]
fn terminal_open_agent_resolves_reattaches_and_keeps_its_errors() {
    let fx = fixture();
    let terminals = manager();
    let dto = create_task_blocking(&fx.app, &terminals, &fx.project_id, "agent", None, None)
        .expect("create_task");

    // Unresolvable provider / binary: the exact pre-conversion messages.
    let err = terminal_open_agent_blocking(&terminals, &fx.app, &dto.id, "not-a-provider", 24, 80)
        .unwrap_err();
    assert_eq!(err, "unknown agent: not-a-provider");
    let err = terminal_open_agent_blocking(&terminals, &fx.app, "ghost-task", "claude", 24, 80)
        .unwrap_err();
    assert_eq!(err, "task not found: ghost-task");

    let bin = install_fake_claude(&fx.tmp);
    let (first, second) = with_path_dir(bin, || {
        let first = terminal_open_agent_blocking(&terminals, &fx.app, &dto.id, "claude", 24, 80)
            .expect("agent open");
        let second = terminal_open_agent_blocking(&terminals, &fx.app, &dto.id, "claude", 24, 80)
            .expect("agent reattach");
        (first, second)
    });
    // ADR-0033: one agent terminal per task — the second open reattaches.
    assert_eq!(first, second);
    assert_eq!(
        terminals
            .list_for_task(&dto.id)
            .iter()
            .filter(|t| t.kind == "agent")
            .count(),
        1
    );
    terminals.close(&first);
}

#[test]
fn terminal_surviving_short_circuits_with_tmux_off() {
    let fx = fixture();
    let terminals = manager();
    let dto = create_task_blocking(&fx.app, &terminals, &fx.project_id, "surviving", None, None)
        .expect("create_task");

    // Default project settings leave tmux off → Ok(0) without probing.
    assert_eq!(
        terminal_surviving_blocking(&terminals, &fx.app, &dto.id),
        Ok(0)
    );
    let err = terminal_surviving_blocking(&terminals, &fx.app, "ghost-task").unwrap_err();
    assert_eq!(err, "task not found: ghost-task");
}

// -- terminal_open_lifecycle ---------------------------------------------

#[test]
fn terminal_open_lifecycle_runs_dedupes_and_keeps_its_errors() {
    let fx = fixture();
    let terminals = manager();
    let dto = create_task_blocking(&fx.app, &terminals, &fx.project_id, "lifecycle", None, None)
        .expect("create_task");

    // Unparseable type and unconfigured script: unchanged messages.
    let err = terminal_open_lifecycle_blocking(&terminals, &fx.app, &dto.id, "bogus", 24, 80)
        .unwrap_err();
    assert_eq!(err, "unknown lifecycle script type: bogus");
    let err = terminal_open_lifecycle_blocking(&terminals, &fx.app, &dto.id, "setup", 24, 80)
        .unwrap_err();
    assert_eq!(err, "no setup script configured for this project");

    let mut settings = fx
        .app
        .settings
        .get_project_settings(&fx.project_id, &fx.project_path)
        .unwrap();
    settings.scripts = Some(Scripts {
        setup: Some("sleep 5".into()),
        run: None,
        teardown: None,
    });
    fx.app
        .settings
        .update_project_settings(&fx.project_id, &fx.project_path, &settings)
        .unwrap();

    let id = terminal_open_lifecycle_blocking(&terminals, &fx.app, &dto.id, "setup", 24, 80)
        .expect("lifecycle open");
    eventually("lifecycle terminal registers", || {
        terminals.find_running_lifecycle(&dto.id, "setup").is_some()
    });
    // In-flight dedupe still reattaches instead of stacking a second run.
    let again = terminal_open_lifecycle_blocking(&terminals, &fx.app, &dto.id, "setup", 24, 80)
        .expect("lifecycle reattach");
    assert_eq!(again, id);
    terminals.close(&id);
}

// -- delete_task ---------------------------------------------------------

#[test]
fn delete_task_tears_down_worktree_terminals_and_emits_task_deleted() {
    let fx = fixture();
    let terminals = manager();
    let acp = acp_runtime(&fx.app);
    let dto = create_task_blocking(&fx.app, &terminals, &fx.project_id, "doomed", None, None)
        .expect("create_task");
    let terminal_id =
        terminal_open_blocking(&terminals, &fx.app, &dto.id, 24, 80).expect("terminal_open");

    let workspace_path = worktree_path(&fx, &dto.id);
    assert!(workspace_path.exists(), "worktree must exist before delete");

    let mut events = fx.app.event_bus.subscribe();
    delete_task_blocking(
        &fx.app,
        terminals.as_ref(),
        &acp,
        &fx.project_id,
        &dto.id,
        None,
        None,
    )
    .expect("delete_task");

    assert!(fx.app.tasks.get(&dto.id).unwrap().is_none(), "row deleted");
    assert!(
        !workspace_path.exists(),
        "worktree removed (default option)"
    );
    assert!(
        !terminals
            .list_for_task(&dto.id)
            .iter()
            .any(|t| t.id == terminal_id),
        "task terminals closed (E2-12)"
    );
    assert!(
        drained(&mut events)
            .iter()
            .any(|e| matches!(e, InternalEvent::TaskDeleted { id } if *id == dto.id)),
        "task:deleted must still be emitted"
    );
}

#[test]
fn delete_task_can_keep_the_worktree() {
    let fx = fixture();
    let terminals = manager();
    let acp = acp_runtime(&fx.app);
    let dto = create_task_blocking(&fx.app, &terminals, &fx.project_id, "keeper", None, None)
        .expect("create_task");
    let workspace_path = worktree_path(&fx, &dto.id);

    delete_task_blocking(
        &fx.app,
        terminals.as_ref(),
        &acp,
        &fx.project_id,
        &dto.id,
        Some(false),
        Some(false),
    )
    .expect("delete_task");

    assert!(fx.app.tasks.get(&dto.id).unwrap().is_none());
    assert!(
        workspace_path.exists(),
        "deleteWorktree=false must leave the worktree in place"
    );
}
