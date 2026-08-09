//! Git status/diff commands (E4-02): thin wrappers over `fartcode_git::status` /
//! `fartcode_git::diff`, worktree-scoped via the task's workspace row. The
//! Changes sidebar (E4-03) refetches these on `git:changed` /
//! `files:changed` events — the commands themselves never poll or cache.
//!
//! Every command here is `async` and does its real work inside
//! [`off_main_thread`]. That is not decoration (#80): a non-async
//! `#[tauri::command]` compiles to `ExecutionContext::Blocking`, whose body
//! is inlined into the invoke handler and runs on the IPC thread — the
//! macOS main thread. A `git push` there stops the NSRunLoop, so the window
//! stops repainting: a beachball, not a spinner. `async` alone would only
//! move the stall onto a tokio worker, so each body is handed to
//! `spawn_blocking` and the command itself only awaits.

use std::path::PathBuf;
use std::sync::Arc;

use fartcode_core::projects::ProjectStore;
use fartcode_git::diff::{DiffSide, FileDiff};
use fartcode_git::status::StatusSnapshot;
use rusqlite::OptionalExtension;
use tauri::State;

use crate::app::App;

/// Resolves a workspace's materialized worktree path. Shared by the git
/// commands and the workspace-file commands (E4-05).
pub(crate) fn workspace_path(app: &App, workspace_id: &str) -> Result<PathBuf, String> {
    let conn = app
        .db
        .conn()
        .lock()
        .map_err(|_| "db connection mutex poisoned".to_string())?;
    let path: Option<Option<String>> = conn
        .query_row(
            "SELECT path FROM workspaces WHERE id = ?1",
            [workspace_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match path {
        None => Err(format!("workspace not found: {workspace_id}")),
        Some(None) => Err(format!("workspace has no local path: {workspace_id}")),
        Some(Some(p)) if p.is_empty() => {
            Err(format!("workspace has no local path: {workspace_id}"))
        }
        Some(Some(p)) => Ok(PathBuf::from(p)),
    }
}

/// Runs `work` on the blocking pool and awaits it, so the calling command
/// never occupies the IPC (main) thread while git, SQLite, or the
/// filesystem is busy (#80).
///
/// The closure owns everything it touches — commands clone the `Arc<App>`
/// out of `State` first, since `State<'_, _>` borrows the invoke scope and
/// cannot cross a thread boundary. A join failure (panic inside the
/// closure) becomes a plain command error rather than a lost invoke that
/// leaves the UI waiting forever.
pub(crate) async fn off_main_thread<T, F>(work: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("git task did not complete: {e}"))?
}

/// Status snapshot (staged/unstaged/conflicts) for a workspace's worktree.
#[tauri::command]
pub async fn git_status(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<StatusSnapshot, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_git::status::status(&worktree).map_err(String::from)
    })
    .await
}

/// Two-sided diff payload for one file. `side`: `"staged"` (HEAD↔index) or
/// `"unstaged"` (index↔worktree). `orig_path` echoes the status entry's
/// rename source for staged renames.
#[tauri::command]
pub async fn git_file_diff(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    path: String,
    side: DiffSide,
    orig_path: Option<String>,
) -> Result<FileDiff, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_git::diff::file_diff(&worktree, &path, side, orig_path.as_deref())
            .map_err(String::from)
    })
    .await
}

/// Stages the given worktree-relative paths (`git add --`).
#[tauri::command]
pub async fn git_stage(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_git::stage::stage(&worktree, &paths).map_err(String::from)
    })
    .await
}

/// Stages every change in the worktree (`git add -A`).
#[tauri::command]
pub async fn git_stage_all(app: State<'_, Arc<App>>, workspace_id: String) -> Result<(), String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_git::stage::stage_all(&worktree).map_err(String::from)
    })
    .await
}

/// Unstages the given paths (`git restore --staged --`; unborn-HEAD safe).
#[tauri::command]
pub async fn git_unstage(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_git::stage::unstage(&worktree, &paths).map_err(String::from)
    })
    .await
}

/// Discards the given paths: tracked paths revert to the index, untracked
/// paths are deleted. The UI confirms before calling.
#[tauri::command]
pub async fn git_discard(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<(), String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_git::stage::discard(&worktree, &paths).map_err(String::from)
    })
    .await
}

/// Commit-card repo state (E4-06): branch, push-remote presence, published
/// flag, and the PR-open guard (the E4-09 sync cache — local and
/// offline-safe). The push remote is the owning project's effective
/// `pushRemote` setting.
#[tauri::command]
pub async fn git_commit_state(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<fartcode_git::commit::CommitState, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        let push_remote = project_push_remote(&app, &workspace_id)?;
        let lookup = fartcode_git::pr_sync::CachedPrLookup {
            store: app.pr_sync.clone(),
        };
        fartcode_git::commit::state(&worktree, &push_remote, &lookup).map_err(String::from)
    })
    .await
}

/// Result of `git_commit` — the new commit hash (reference returns it; the
/// card could deep-link to it later).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub hash: String,
}

/// Commits exactly the staged set with `message` (E4-06). Empty messages
/// are rejected before git spawns.
#[tauri::command]
pub async fn git_commit(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    message: String,
) -> Result<CommitResult, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        let hash = fartcode_git::commit::commit(&worktree, &message).map_err(String::from)?;
        Ok(CommitResult { hash })
    })
    .await
}

/// Pushes the current branch, setting upstream on first push (E4-06).
#[tauri::command]
pub async fn git_push(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<fartcode_git::commit::PushOutcome, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        let push_remote = project_push_remote(&app, &workspace_id)?;
        fartcode_git::commit::push(&worktree, &push_remote).map_err(String::from)
    })
    .await
}

/// Phase 0 "Commit & Create PR" backend (E4-06): pushes when the branch
/// isn't published, applies the PR-open guard (sync cache), and returns the
/// GitHub compare URL for the browser to finish the job.
#[tauri::command]
pub async fn git_create_pr(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<fartcode_git::commit::CreatePrOutcome, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        let push_remote = project_push_remote(&app, &workspace_id)?;
        let lookup = fartcode_git::pr_sync::CachedPrLookup {
            store: app.pr_sync.clone(),
        };
        fartcode_git::commit::create_pr(&worktree, &push_remote, &lookup).map_err(String::from)
    })
    .await
}

/// Fetches the push remote (E4-08 footer).
#[tauri::command]
pub async fn git_fetch(app: State<'_, Arc<App>>, workspace_id: String) -> Result<(), String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        let push_remote = project_push_remote(&app, &workspace_id)?;
        fartcode_git::remote::fetch(&worktree, &push_remote).map_err(String::from)
    })
    .await
}

/// `git pull --ff-only` (E4-08): diverged history surfaces git's error,
/// never a hidden merge.
#[tauri::command]
pub async fn git_pull(app: State<'_, Arc<App>>, workspace_id: String) -> Result<(), String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_git::remote::pull(&worktree).map_err(String::from)
    })
    .await
}

/// Publishes an unpublished branch (`push -u`; E4-08). Errors when the
/// branch already tracks an upstream — that path is `git_push`.
#[tauri::command]
pub async fn git_publish(
    app: State<'_, Arc<App>>,
    workspace_id: String,
) -> Result<fartcode_git::remote::PublishOutcome, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        let push_remote = project_push_remote(&app, &workspace_id)?;
        fartcode_git::remote::publish(&worktree, &push_remote).map_err(String::from)
    })
    .await
}

/// Adds a remote (E4-08): the footer's zero-remotes affordance.
#[tauri::command]
pub async fn git_add_remote(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    name: String,
    url: String,
) -> Result<(), String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_git::remote::add_remote(&worktree, &name, &url).map_err(String::from)
    })
    .await
}

/// `git pull --ff-only` at the project root (left-nav project pull): after a
/// worktree branch lands on the remote default branch, this brings the
/// project checkout — the branch the app itself may be running on — up to
/// date. Same ff-only contract as `git_pull`: diverged history surfaces
/// git's error, never a hidden merge.
#[tauri::command]
pub async fn project_git_pull(app: State<'_, Arc<App>>, project_id: String) -> Result<(), String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let project = app
            .projects
            .get(&project_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("project not found: {project_id}"))?;
        fartcode_git::remote::pull(&project.path).map_err(String::from)?;
        // E19-03 (#72; ADR-0038 item 4): a pull is when merged dossiers
        // appear in the main checkout, so it is the second reindex trigger
        // (the first is step settle, on the worktree copy). Already off the
        // main thread inside `off_main_thread`; never fails the pull.
        crate::dossier_index::reindex_project(&app, &project_id);
        Ok(())
    })
    .await
}

/// The project's browsable GitHub URL (project view's GitHub affordance):
/// the effective base remote's URL normalized to https. `None` when there's
/// no such remote or it isn't GitHub — the UI hides the icon.
#[tauri::command]
pub async fn project_github_url(
    app: State<'_, Arc<App>>,
    project_id: String,
) -> Result<Option<String>, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let project = app
            .projects
            .get(&project_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("project not found: {project_id}"))?;
        let settings = app
            .settings
            .get_project_settings(&project_id, &project.path)
            .map_err(|e| e.to_string())?;
        let url = fartcode_git::remote::remote_url(&project.path, settings.effective_base_remote());
        Ok(url
            .as_deref()
            .and_then(fartcode_git::remote::github_https_url))
    })
    .await
}

/// Effective `pushRemote` of the project owning `workspace_id` (reference
/// `getPushRemote`: pushRemote ?? baseRemote ?? "origin"). Resolves
/// workspace → task → project; falls back to "origin" for workspaces not
/// owned by a task (repository-root workspaces).
pub(crate) fn project_push_remote(app: &App, workspace_id: &str) -> Result<String, String> {
    let project_id: Option<String> = {
        let conn = app
            .db
            .conn()
            .lock()
            .map_err(|_| "db connection mutex poisoned".to_string())?;
        conn.query_row(
            "SELECT project_id FROM tasks WHERE workspace_id = ?1",
            [workspace_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    };
    let Some(project_id) = project_id else {
        return Ok(fartcode_core::settings::DEFAULT_BASE_REMOTE.to_string());
    };
    let project = app
        .projects
        .get(&project_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    let settings = app
        .settings
        .get_project_settings(&project_id, &project.path)
        .map_err(|e| e.to_string())?;
    Ok(settings.effective_push_remote().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread::ThreadId;
    use std::time::Duration;

    use serde_json::{json, Value};
    use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse};
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};

    /// Every wait in this module is bounded. A test that hangs tells you
    /// nothing and takes the whole suite with it.
    const RESPONSE_BUDGET: Duration = Duration::from_secs(30);

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed in {dir:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    struct Fixture {
        webview: WebviewWindow<MockRuntime>,
        /// Unused by name, but the webview and the managed `App` die with it.
        _app: tauri::App<MockRuntime>,
        repo: PathBuf,
        project_id: String,
        workspace_id: String,
        /// Declared last so it is dropped last: the repo and the SQLite file
        /// live in here.
        tmp: tempfile::TempDir,
    }

    impl Fixture {
        fn branch(&self) -> String {
            git_out(&self.repo, &["branch", "--show-current"])
        }

        /// Invokes a command the way the frontend does — through the real
        /// invoke handler, with camelCase args — and reports which thread
        /// the response came back on.
        fn invoke_on(&self, cmd: &str, args: Value) -> (Result<Value, String>, ThreadId) {
            let (tx, rx) = mpsc::sync_channel(1);
            self.webview.as_ref().clone().on_message(
                InvokeRequest {
                    cmd: cmd.into(),
                    callback: CallbackFn(0),
                    error: CallbackFn(1),
                    // The app's own protocol URL: anything else reads as a
                    // remote origin and is rejected by the ACL before the
                    // command ever runs.
                    url: "tauri://localhost".parse().unwrap(),
                    body: InvokeBody::Json(args),
                    headers: Default::default(),
                    invoke_key: INVOKE_KEY.to_string(),
                },
                Box::new(move |_w, _cmd, response, _cb, _err| {
                    let _ = tx.send((response, std::thread::current().id()));
                }),
            );
            // `tauri::test::get_ipc_response` blocks forever here; we don't.
            let (response, thread) = rx
                .recv_timeout(RESPONSE_BUDGET)
                .unwrap_or_else(|e| panic!("{cmd} did not respond within budget: {e}"));
            let value = match response {
                InvokeResponse::Ok(body) => Ok(body
                    .deserialize::<Value>()
                    .unwrap_or_else(|e| panic!("{cmd} returned undeserializable body: {e}"))),
                InvokeResponse::Err(e) => Err(match e.0 {
                    Value::String(s) => s,
                    other => other.to_string(),
                }),
            };
            (value, thread)
        }

        fn invoke(&self, cmd: &str, args: Value) -> Result<Value, String> {
            self.invoke_on(cmd, args).0
        }

        fn ok(&self, cmd: &str, args: Value) -> Value {
            self.invoke(cmd, args)
                .unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
        }

        fn err(&self, cmd: &str, args: Value) -> String {
            match self.invoke(cmd, args) {
                Err(e) => e,
                Ok(v) => panic!("{cmd} unexpectedly succeeded: {v}"),
            }
        }

        fn status(&self) -> Value {
            self.ok("git_status", json!({ "workspaceId": self.workspace_id }))
        }
    }

    /// Repo with one commit + a bare sibling wired as `origin`, a real
    /// `App` over a temp DB, and a mock Tauri app running the *actual*
    /// invoke handler for these commands.
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@fartCode.dev"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let bare = tmp.path().join("bare.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "--bare", "-q"]);
        git(
            &repo,
            &["remote", "add", "origin", bare.to_string_lossy().as_ref()],
        );

        let db = tmp.path().join("fartcode.sqlite");
        let state = App::init(Some(db.to_string_lossy().as_ref())).unwrap();

        let project_id = "p_test".to_string();
        let workspace_id = "w_test".to_string();
        {
            let conn = state.db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, path) VALUES (?1, ?2, ?3)",
                rusqlite::params![&project_id, "test", repo.to_string_lossy()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, path) VALUES (?1, ?2)",
                rusqlite::params![&workspace_id, repo.to_string_lossy()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, name, status, workspace_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    "t_test",
                    &project_id,
                    "test task",
                    "in_progress",
                    &workspace_id
                ],
            )
            .unwrap();
        }

        let app = mock_builder()
            .invoke_handler(tauri::generate_handler![
                super::git_status,
                super::git_file_diff,
                super::git_stage,
                super::git_stage_all,
                super::git_unstage,
                super::git_discard,
                super::git_commit_state,
                super::git_commit,
                super::git_push,
                super::git_create_pr,
                super::git_fetch,
                super::git_pull,
                super::git_publish,
                super::git_add_remote,
                super::project_git_pull,
                super::project_github_url,
            ])
            .build(mock_context(noop_assets()))
            .unwrap();
        app.manage(state);
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();

        Fixture {
            webview,
            _app: app,
            repo,
            project_id,
            workspace_id,
            tmp,
        }
    }

    /// The property #80 turns on: these commands must stay `async`, because
    /// the macro decides `Blocking` vs `Async` at expansion and no runtime
    /// assertion can catch a regression after the fact. `.await` compiles
    /// only against a `Future`, so reverting any of these to `fn` breaks
    /// this test at build time.
    #[test]
    fn commands_are_awaitable_futures() {
        let f = fixture();
        let ws = f.workspace_id.clone();
        let project = f.project_id.clone();
        tauri::async_runtime::block_on(async {
            let snap = git_status(f.webview.state(), ws.clone()).await.unwrap();
            assert!(snap.staged.is_empty());
            let state = git_commit_state(f.webview.state(), ws.clone())
                .await
                .unwrap();
            assert!(state.has_remote);
            git_stage_all(f.webview.state(), ws.clone()).await.unwrap();
            assert!(project_github_url(f.webview.state(), project.clone())
                .await
                .unwrap()
                .is_none());
        });
    }

    #[test]
    fn git_status_reports_changes_and_answers_off_the_ipc_thread() {
        let f = fixture();
        std::fs::write(f.repo.join("tracked.txt"), "one\ntwo\nchanged\n").unwrap();
        std::fs::write(f.repo.join("new.txt"), "brand new\n").unwrap();

        let caller = std::thread::current().id();
        let (result, responder) =
            f.invoke_on("git_status", json!({ "workspaceId": f.workspace_id }));
        let snap = result.unwrap();

        // Wire contract unchanged: same camelCase envelope the sidebar reads.
        assert!(snap["staged"].as_array().unwrap().is_empty());
        let unstaged = snap["unstaged"].as_array().unwrap();
        assert_eq!(unstaged.len(), 2);
        let paths: Vec<&str> = unstaged
            .iter()
            .map(|e| e["path"].as_str().unwrap())
            .collect();
        assert!(paths.contains(&"tracked.txt") && paths.contains(&"new.txt"));

        // A `Blocking` command resolves inline on the invoking thread —
        // which on macOS is the main thread, and why the window froze.
        assert_ne!(
            caller, responder,
            "git_status resolved on the invoking thread: it is running as a blocking command again"
        );
    }

    #[test]
    fn git_file_diff_returns_both_sides() {
        let f = fixture();
        std::fs::write(f.repo.join("tracked.txt"), "one\ntwo\nchanged\n").unwrap();

        let diff = f.ok(
            "git_file_diff",
            json!({
                "workspaceId": f.workspace_id,
                "path": "tracked.txt",
                "side": "unstaged",
                "origPath": null,
            }),
        );
        assert_eq!(diff["path"], "tracked.txt");
        assert!(diff["oldExists"].as_bool().unwrap());
        assert!(diff["newExists"].as_bool().unwrap());
        assert!(diff["newContent"].as_str().unwrap().contains("changed"));
    }

    #[test]
    fn stage_unstage_discard_round_trip() {
        let f = fixture();
        std::fs::write(f.repo.join("tracked.txt"), "one\ntwo\nchanged\n").unwrap();
        std::fs::write(f.repo.join("untracked.txt"), "bye\n").unwrap();

        f.ok(
            "git_stage",
            json!({ "workspaceId": f.workspace_id, "paths": ["tracked.txt"] }),
        );
        let snap = f.status();
        assert_eq!(snap["staged"].as_array().unwrap().len(), 1);

        f.ok(
            "git_unstage",
            json!({ "workspaceId": f.workspace_id, "paths": ["tracked.txt"] }),
        );
        assert!(f.status()["staged"].as_array().unwrap().is_empty());

        f.ok("git_stage_all", json!({ "workspaceId": f.workspace_id }));
        assert_eq!(f.status()["staged"].as_array().unwrap().len(), 2);

        f.ok(
            "git_unstage",
            json!({ "workspaceId": f.workspace_id, "paths": ["tracked.txt", "untracked.txt"] }),
        );
        f.ok(
            "git_discard",
            json!({ "workspaceId": f.workspace_id, "paths": ["tracked.txt", "untracked.txt"] }),
        );
        assert_eq!(
            std::fs::read_to_string(f.repo.join("tracked.txt")).unwrap(),
            "one\ntwo\nthree\n"
        );
        assert!(!f.repo.join("untracked.txt").exists());

        // Validation errors still surface verbatim.
        let err = f.err(
            "git_stage",
            json!({ "workspaceId": f.workspace_id, "paths": ["../escape"] }),
        );
        assert!(err.contains("escape"), "unexpected error: {err}");
    }

    #[test]
    fn commit_state_then_commit_returns_head_hash() {
        let f = fixture();
        let state = f.ok("git_commit_state", json!({ "workspaceId": f.workspace_id }));
        assert_eq!(state["branch"], f.branch());
        assert!(state["hasRemote"].as_bool().unwrap());
        assert!(!state["published"].as_bool().unwrap());
        assert!(!state["prOpen"].as_bool().unwrap());
        assert!(state["canCreatePr"].as_bool().unwrap());
        assert_eq!(state["ahead"], 0);
        assert_eq!(state["behind"], 0);

        std::fs::write(f.repo.join("tracked.txt"), "one\ntwo\ncommitted\n").unwrap();
        f.ok(
            "git_stage",
            json!({ "workspaceId": f.workspace_id, "paths": ["tracked.txt"] }),
        );
        let out = f.ok(
            "git_commit",
            json!({ "workspaceId": f.workspace_id, "message": "wip" }),
        );
        assert_eq!(out["hash"], git_out(&f.repo, &["rev-parse", "HEAD"]));

        // Empty message is still rejected before git spawns.
        let err = f.err(
            "git_commit",
            json!({ "workspaceId": f.workspace_id, "message": "   " }),
        );
        assert!(err.contains("commit message is empty"), "got: {err}");
    }

    #[test]
    fn publish_fetch_pull_push_against_a_local_remote() {
        let f = fixture();
        let branch = f.branch();

        let out = f.ok("git_publish", json!({ "workspaceId": f.workspace_id }));
        assert_eq!(out["branch"], branch);
        assert_eq!(out["remote"], "origin");

        // Second publish refuses with the same message as before.
        let err = f.err("git_publish", json!({ "workspaceId": f.workspace_id }));
        assert!(err.contains("already has an upstream"), "got: {err}");

        f.ok("git_fetch", json!({ "workspaceId": f.workspace_id }));
        f.ok("git_pull", json!({ "workspaceId": f.workspace_id }));

        // A new local commit pushes over the existing upstream.
        std::fs::write(f.repo.join("tracked.txt"), "one\ntwo\npushed\n").unwrap();
        git(&f.repo, &["add", "."]);
        git(&f.repo, &["commit", "-q", "-m", "second"]);
        let pushed = f.ok("git_push", json!({ "workspaceId": f.workspace_id }));
        assert_eq!(pushed["setUpstream"], false);
        assert_eq!(pushed["branch"], branch);

        let state = f.ok("git_commit_state", json!({ "workspaceId": f.workspace_id }));
        assert!(state["published"].as_bool().unwrap());
        assert_eq!(state["upstream"], format!("origin/{branch}"));
        assert_eq!(state["ahead"], 0);
    }

    #[test]
    fn create_pr_maps_a_published_github_branch() {
        let f = fixture();
        f.ok("git_publish", json!({ "workspaceId": f.workspace_id }));
        git(
            &f.repo,
            &["remote", "set-url", "origin", "git@github.com:o/r.git"],
        );

        let out = f.ok("git_create_pr", json!({ "workspaceId": f.workspace_id }));
        assert_eq!(out["pushed"], false);
        assert_eq!(
            out["url"],
            format!("https://github.com/o/r/compare/{}?expand=1", f.branch())
        );
    }

    #[test]
    fn add_remote_validates_and_rejects_duplicates() {
        let f = fixture();
        f.ok(
            "git_add_remote",
            json!({ "workspaceId": f.workspace_id, "name": "upstream", "url": "/tmp/elsewhere.git" }),
        );
        assert!(git_out(&f.repo, &["remote"]).contains("upstream"));

        let err = f.err(
            "git_add_remote",
            json!({ "workspaceId": f.workspace_id, "name": "upstream", "url": "/x" }),
        );
        assert!(err.contains("already exists"), "got: {err}");

        let err = f.err(
            "git_add_remote",
            json!({ "workspaceId": f.workspace_id, "name": "bad name", "url": "/x" }),
        );
        assert!(err.contains("invalid remote name"), "got: {err}");
    }

    #[test]
    fn project_commands_resolve_the_project_checkout() {
        let f = fixture();
        // Non-GitHub remote → the UI hides the affordance.
        assert_eq!(
            f.ok("project_github_url", json!({ "projectId": f.project_id })),
            Value::Null
        );
        git(
            &f.repo,
            &["remote", "set-url", "origin", "git@github.com:o/r.git"],
        );
        assert_eq!(
            f.ok("project_github_url", json!({ "projectId": f.project_id })),
            json!("https://github.com/o/r")
        );

        // Unknown project keeps its exact message.
        assert_eq!(
            f.err("project_github_url", json!({ "projectId": "nope" })),
            "project not found: nope"
        );
        assert_eq!(
            f.err("project_git_pull", json!({ "projectId": "nope" })),
            "project not found: nope"
        );
    }

    #[test]
    fn project_git_pull_fast_forwards_the_checkout() {
        let f = fixture();
        f.ok("git_publish", json!({ "workspaceId": f.workspace_id }));
        let branch = f.branch();

        // Advance the remote through a second clone, then ff the project.
        let clone = f.tmp.path().join("clone");
        let bare = f.tmp.path().join("bare.git");
        Command::new("git")
            .args(["clone", "-q"])
            .arg(&bare)
            .arg(&clone)
            .status()
            .unwrap();
        git(&clone, &["config", "user.name", "Test"]);
        git(&clone, &["config", "user.email", "test@fartCode.dev"]);
        std::fs::write(clone.join("remote-only.txt"), "r\n").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "-q", "-m", "remote ahead"]);
        git(&clone, &["push", "-q"]);

        f.ok("git_fetch", json!({ "workspaceId": f.workspace_id }));
        let state = f.ok("git_commit_state", json!({ "workspaceId": f.workspace_id }));
        assert_eq!(state["behind"], 1);

        f.ok("project_git_pull", json!({ "projectId": f.project_id }));
        assert!(f.repo.join("remote-only.txt").exists());
        assert_eq!(
            git_out(&f.repo, &["rev-parse", "HEAD"]),
            git_out(&f.repo, &["rev-parse", &format!("origin/{branch}")])
        );
    }

    #[test]
    fn workspace_resolution_errors_are_unchanged() {
        let f = fixture();
        assert_eq!(
            f.err("git_status", json!({ "workspaceId": "missing" })),
            "workspace not found: missing"
        );

        {
            let app = f.webview.state::<Arc<App>>();
            let conn = app.db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, path) VALUES ('w_nopath', NULL)",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            f.err("git_status", json!({ "workspaceId": "w_nopath" })),
            "workspace has no local path: w_nopath"
        );
    }

    #[test]
    fn off_main_thread_leaves_the_caller_and_propagates_both_outcomes() {
        let caller = std::thread::current().id();
        let ran_on = tauri::async_runtime::block_on(off_main_thread(move || {
            Ok::<ThreadId, String>(std::thread::current().id())
        }))
        .unwrap();
        assert_ne!(caller, ran_on, "closure ran on the calling thread");

        let err = tauri::async_runtime::block_on(off_main_thread(|| {
            Err::<(), String>("verbatim failure".into())
        }))
        .unwrap_err();
        assert_eq!(err, "verbatim failure");

        // A panic must become an error, not a promise the UI waits on forever.
        let err = tauri::async_runtime::block_on(off_main_thread(|| {
            panic!("boom");
            #[allow(unreachable_code)]
            Ok::<(), String>(())
        }))
        .unwrap_err();
        assert!(err.starts_with("git task did not complete"), "got: {err}");
    }
}
