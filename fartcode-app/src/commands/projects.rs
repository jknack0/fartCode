//! Project commands (E1-04 sidebar: list, create, delete).
//!
//! **UI thread (#80):** a non-async `#[tauri::command]` compiles to
//! `ExecutionContext::Blocking` — its body is inlined into the invoke
//! handler and runs synchronously on the IPC thread, which on macOS is the
//! main thread. Blocking there stalls the NSRunLoop and the window stops
//! repainting (a beachball, not a spinner). `create_project` (5-6 git
//! subprocesses + a `.fartCode.json` read + a `.git/info/exclude` write)
//! and `delete_project` (a recursive `remove_dir_all` of the whole worktree
//! pool + DB cascade) therefore hand their bodies to [`off_main_thread`]
//! (`spawn_blocking` + join). Making them merely `async` would not help —
//! the body would just block a tokio worker instead of the main thread.
//!
//! The wire contract is unchanged: same argument names, same serialized
//! result, same error strings, same event ordering. `list_projects` stays
//! synchronous — it is a single indexed DB read.

use fartcode_core::projects::{ProjectDto, ProjectStore};
use fartcode_core::tasks::TaskStore;
use std::sync::Arc;

use tauri::State;

use crate::app::App;
use crate::commands::off_main_thread;
use crate::terminals::TerminalManager;

#[tauri::command]
pub fn list_projects(app: State<'_, Arc<App>>) -> Result<Vec<ProjectDto>, String> {
    app.projects
        .list()
        .map(|ps| ps.iter().map(ProjectDto::from).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_project(app: State<'_, Arc<App>>, path: String) -> Result<ProjectDto, String> {
    // `State` cannot cross into the blocking closure; the managed value is
    // an `Arc<App>`, so clone the handle and move that.
    let app = app.inner().clone();
    off_main_thread(move || create_project_blocking(&app, &path)).await
}

/// Body of [`create_project`], run on the blocking pool. Identical to the
/// former inline body — kept as a named fn so tests can assert the
/// off-thread path and the direct call agree.
fn create_project_blocking(app: &App, path: &str) -> Result<ProjectDto, String> {
    app.projects
        .create_local(std::path::Path::new(path), false)
        .map(|p| ProjectDto::from(&p))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_project(
    app: State<'_, Arc<App>>,
    terminals: State<'_, Arc<TerminalManager>>,
    acp: State<'_, Arc<crate::acp_runtime::AcpRuntime>>,
    id: String,
) -> Result<(), String> {
    let app = app.inner().clone();
    let terminals = terminals.inner().clone();
    let acp = acp.inner().clone();
    off_main_thread(move || delete_project_blocking(&app, &terminals, &acp, &id)).await
}

/// Body of [`delete_project`], run on the blocking pool.
fn delete_project_blocking<R: tauri::Runtime>(
    app: &App,
    terminals: &TerminalManager<R>,
    acp: &crate::acp_runtime::AcpRuntime,
    id: &str,
) -> Result<(), String> {
    // #84: the same teardown `delete_task` runs, scoped to every task of
    // the project — ACP stop runs BEFORE the row deletion (the FK cascade
    // wipes the conversation rows `stop_task` reads); terminal close +
    // tmux sweep run after. Both teardowns are best-effort and bounded
    // internally, so a wedged process cannot hang the delete.
    let tasks = app.tasks.list_by_project(id).map_err(|e| e.to_string())?;
    for task in &tasks {
        acp.stop_task(&task.id);
    }
    app.projects.delete(id).map_err(|e| e.to_string())?;
    // E18-04 fix round (finding 4): the FK cascade deletes the project's
    // issues without per-issue events — sweep their step-engine state
    // (parks + launch registry) by project.
    crate::step_engine::on_project_deleted(app, id);
    for task in &tasks {
        terminals.close_task(id, &task.id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::events::{EventBus, InternalEvent};
    use std::path::Path;
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::Duration;
    use tauri::Manager;

    /// Every await in these tests is bounded — an unbounded wait would
    /// wedge the suite instead of failing it.
    const TEST_TIMEOUT: Duration = Duration::from_secs(30);

    /// Drives a command future on a **single-threaded** runtime: the shape
    /// of the IPC thread, so a body that fails to leave the thread is
    /// observable (see `create_project_yields_the_calling_thread`).
    fn block_on_bounded<F: std::future::Future>(fut: F) -> F::Output {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async move {
            tokio::time::timeout(TEST_TIMEOUT, fut)
                .await
                .expect("command future timed out")
        })
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .output()
            .expect("git spawns");
        assert!(out.status.success(), "git {args:?} failed");
    }

    fn repo_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-b", "main"]);
        std::fs::write(tmp.path().join("README.md"), "hi\n").unwrap();
        git(tmp.path(), &["add", "."]);
        git(tmp.path(), &["commit", "-m", "init"]);
        tmp
    }

    fn app() -> Arc<App> {
        App::init(Some(":memory:")).expect("app init")
    }

    /// A no-op ACP runtime: project teardown only calls `stop_task`, and
    /// the fixtures' tasks own no conversations, so the adapter resolver
    /// never fires.
    fn acp_runtime(app: &Arc<App>) -> Arc<crate::acp_runtime::AcpRuntime> {
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
        crate::acp_runtime::AcpRuntime::new(
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

    /// MockRuntime terminal manager — command-level tests cannot build a
    /// Wry `TerminalManager` (`EventLoop` panics off the main thread), so
    /// the tests drive `delete_project_blocking` directly, the same
    /// pattern `tasks_terminals_offload` uses for `delete_task`. The
    /// command's async + spawn_blocking shape is gated by
    /// `tests/no_blocking_tauri_commands.rs`.
    fn manager() -> Arc<TerminalManager<tauri::test::MockRuntime>> {
        Arc::new(TerminalManager::new(
            tauri::test::mock_app().handle().clone(),
        ))
    }

    /// A path that cannot exist — the deterministic error path of
    /// `create_local` (no git subprocess involved).
    fn missing_path() -> String {
        std::env::temp_dir()
            .join(format!("fartcode-absent-{}", uuid::Uuid::new_v4()))
            .to_string_lossy()
            .into_owned()
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) -> Vec<InternalEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    #[test]
    fn create_project_returns_the_same_dto_the_store_produced() {
        let repo = repo_fixture();
        let app = app();
        let mock = tauri::test::mock_app();
        mock.manage(app.clone());
        let state = mock.state::<Arc<App>>();

        let dto = block_on_bounded(create_project(
            state,
            repo.path().to_string_lossy().into_owned(),
        ))
        .expect("create_project");

        let listed = app.projects.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, dto.id);
        assert_eq!(listed[0].name, dto.name);
        assert_eq!(listed[0].path.to_string_lossy(), dto.path);
        assert_eq!(dto.workspace_provider, "local");
    }

    #[test]
    fn create_project_still_emits_project_added() {
        let repo = repo_fixture();
        let app = app();
        let mut events = app.event_bus.subscribe();
        let mock = tauri::test::mock_app();
        mock.manage(app.clone());
        let state = mock.state::<Arc<App>>();

        let dto = block_on_bounded(create_project(
            state,
            repo.path().to_string_lossy().into_owned(),
        ))
        .expect("create_project");

        let added = drain(&mut events)
            .into_iter()
            .find_map(|ev| match ev {
                InternalEvent::ProjectAdded { id, name, path } => Some((id, name, path)),
                _ => None,
            })
            .expect("ProjectAdded emitted");
        assert_eq!(added.0, dto.id);
        assert_eq!(added.1, dto.name);
        assert_eq!(added.2, dto.path);
    }

    #[test]
    fn create_project_preserves_the_error_string() {
        let path = missing_path();
        let expected = create_project_blocking(&app(), &path).expect_err("absent path fails");

        let mock = tauri::test::mock_app();
        mock.manage(app());
        let state = mock.state::<Arc<App>>();
        let got = block_on_bounded(create_project(state, path)).expect_err("absent path fails");

        assert_eq!(got, expected);
    }

    #[test]
    fn delete_project_removes_the_row_and_emits_project_deleted() {
        let repo = repo_fixture();
        let app = app();
        let project = app.projects.create_local(repo.path(), false).unwrap();
        let mut events = app.event_bus.subscribe();
        let wry = manager();

        delete_project_blocking(&app, &wry, &acp_runtime(&app), &project.id)
            .expect("delete_project");

        assert!(app.projects.list().unwrap().is_empty());
        assert!(
            drain(&mut events)
                .iter()
                .any(|ev| matches!(ev, InternalEvent::ProjectDeleted { id } if *id == project.id)),
            "ProjectDeleted emitted"
        );
    }

    #[test]
    fn delete_project_preserves_the_error_string() {
        let app = app();
        let got = delete_project_blocking(&app, &manager(), &acp_runtime(&app), "no-such-project")
            .expect_err("unknown id");
        let expected = got.clone();

        assert_eq!(got, expected);
        assert!(got.contains("no-such-project"), "{got}");
    }

    /// The #80 property. On a single-threaded runtime a co-scheduled task
    /// can only run while the awaited command is pending — so observing
    /// `probe` before `command` proves the body left the calling thread. A
    /// body inlined into the command (today's `ExecutionContext::Blocking`)
    /// would complete on the first poll and record `command` first.
    #[test]
    fn create_project_yields_the_calling_thread() {
        let repo = repo_fixture();
        let app = app();
        let mock = tauri::test::mock_app();
        mock.manage(app);
        let state = mock.state::<Arc<App>>();
        let path = repo.path().to_string_lossy().into_owned();

        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let probe_order = order.clone();
        let command_order = order.clone();
        block_on_bounded(async move {
            tokio::spawn(async move { probe_order.lock().unwrap().push("probe") });
            let created = create_project(state, path).await;
            command_order.lock().unwrap().push("command");
            created.expect("create_project");
        });

        assert_eq!(
            order.lock().unwrap().as_slice(),
            ["probe", "command"],
            "create_project blocked its caller instead of yielding"
        );
    }

    /// #84 regression: deleting a project with a live task closes that
    /// task's terminals (tmux sweep included — see ADR-0025).
    #[test]
    fn delete_project_closes_the_tasks_terminals() {
        let repo = repo_fixture();
        let app = app();
        let project = app.projects.create_local(repo.path(), false).unwrap();
        let task = app
            .tasks
            .create(fartcode_core::tasks::CreateTaskOptions::new(
                &project.id,
                "demo",
            ))
            .unwrap();
        let terminals = manager();
        terminals
            .open(crate::terminals::TerminalSpec {
                task_id: &task.id,
                project_id: &project.id,
                agent: None,
                tmux: false,
                program: "/bin/sh",
                args: &["-c".into(), "sleep 30".into()],
                env: &[],
                remove: &[],
                cwd: &std::env::temp_dir(),
                rows: 24,
                cols: 80,
                lifecycle: None,
            })
            .expect("open terminal");
        assert_eq!(terminals.list_for_task(&task.id).len(), 1);

        delete_project_blocking(&app, &terminals, &acp_runtime(&app), &project.id)
            .expect("delete_project");

        assert!(
            terminals.list_for_task(&task.id).is_empty(),
            "task terminal closed on project delete"
        );
        assert!(app.projects.list().unwrap().is_empty());
    }
}
