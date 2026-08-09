//! Project commands (E1-04 sidebar: list, create, delete).
//!
//! **UI thread (#80):** a non-async `#[tauri::command]` compiles to
//! `ExecutionContext::Blocking` — its body is inlined into the invoke
//! handler and runs synchronously on the IPC thread, which on macOS is the
//! main thread. Blocking there stalls the NSRunLoop and the window stops
//! repainting (a beachball, not a spinner). `create_project` (5-6 git
//! subprocesses + a `.fartCode.json` read + a `.git/info/exclude` write)
//! and `delete_project` (a recursive `remove_dir_all` of the whole worktree
//! pool + DB cascade) therefore hand their bodies to `spawn_blocking` and
//! await the join handle. Making them merely `async` would not help — the
//! body would just block a tokio worker instead of the main thread.
//!
//! The wire contract is unchanged: same argument names, same serialized
//! result, same error strings, same event ordering. `list_projects` stays
//! synchronous — it is a single indexed DB read.

use fartcode_core::projects::{ProjectDto, ProjectStore};
use std::sync::Arc;

use tauri::State;

use crate::app::App;

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
    tauri::async_runtime::spawn_blocking(move || create_project_blocking(&app, &path))
        .await
        .map_err(|e| e.to_string())?
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
pub async fn delete_project(app: State<'_, Arc<App>>, id: String) -> Result<(), String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || delete_project_blocking(&app, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Body of [`delete_project`], run on the blocking pool.
fn delete_project_blocking(app: &App, id: &str) -> Result<(), String> {
    app.projects.delete(id).map_err(|e| e.to_string())
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
        let mock = tauri::test::mock_app();
        mock.manage(app.clone());
        let state = mock.state::<Arc<App>>();

        block_on_bounded(delete_project(state, project.id.clone())).expect("delete_project");

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
        let expected = delete_project_blocking(&app(), "no-such-project").expect_err("unknown id");

        let mock = tauri::test::mock_app();
        mock.manage(app());
        let state = mock.state::<Arc<App>>();
        let got = block_on_bounded(delete_project(state, "no-such-project".to_string()))
            .expect_err("unknown id");

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

    /// Same property for the unbounded one (a full worktree-pool
    /// `remove_dir_all`).
    #[test]
    fn delete_project_yields_the_calling_thread() {
        let repo = repo_fixture();
        let app = app();
        let project = app.projects.create_local(repo.path(), false).unwrap();
        let mock = tauri::test::mock_app();
        mock.manage(app);
        let state = mock.state::<Arc<App>>();

        let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let probe_order = order.clone();
        let command_order = order.clone();
        block_on_bounded(async move {
            tokio::spawn(async move { probe_order.lock().unwrap().push("probe") });
            let deleted = delete_project(state, project.id).await;
            command_order.lock().unwrap().push("command");
            deleted.expect("delete_project");
        });

        assert_eq!(
            order.lock().unwrap().as_slice(),
            ["probe", "command"],
            "delete_project blocked its caller instead of yielding"
        );
    }
}
