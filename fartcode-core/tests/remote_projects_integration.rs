//! Remote project integration tests (E12-04).
//!
//! No SSH here: [`FakeHost`] implements `RemoteHost` over in-memory state and
//! answers the exact git argv the flow issues. That keeps the *contract* under
//! test (validation order, duplicate handling, path construction, teardown
//! idempotence) instead of a live host's mood.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use fartcode_core::projects::provider::repository_workspace_key;
use fartcode_core::projects::remote::{
    is_contained, remote_worktree_path, remote_worktree_root, RemoteEntry, RemoteFileKind,
    RemoteHost, RemoteOutput, RemoteProjectStore,
};
use fartcode_core::projects::WorkspaceProviderKind;
use fartcode_core::ssh_connections::{NewSshConnection, SshConnectionStore};

const CONN: &str = "conn-1";

// ── Fake remote host ─────────────────────────────────────────

#[derive(Default)]
struct State {
    dirs: HashSet<String>,
    files: HashSet<String>,
    repos: HashSet<String>,
    /// repo path -> checked-out branch (worktrees included).
    branches: HashMap<String, String>,
    remotes: HashMap<String, String>,
    commands: Vec<Vec<String>>,
}

#[derive(Default)]
struct FakeHost {
    state: Mutex<State>,
}

impl FakeHost {
    fn with_repo(path: &str, remote: Option<&str>, branch: &str) -> Self {
        let host = FakeHost::default();
        {
            let mut s = host.state.lock().unwrap();
            s.dirs.insert(path.to_string());
            s.repos.insert(path.to_string());
            s.branches.insert(path.to_string(), branch.to_string());
            if let Some(r) = remote {
                s.remotes.insert(path.to_string(), r.to_string());
            }
        }
        host
    }

    fn commands(&self) -> Vec<Vec<String>> {
        self.state.lock().unwrap().commands.clone()
    }

    fn exists(&self, path: &str) -> bool {
        let s = self.state.lock().unwrap();
        s.dirs.contains(path) || s.files.contains(path)
    }
}

fn out(code: i32, stdout: &str) -> RemoteOutput {
    RemoteOutput {
        exit_code: code,
        stdout: stdout.to_string(),
        stderr: String::new(),
    }
}

#[async_trait::async_trait]
impl RemoteHost for FakeHost {
    async fn realpath(&self, path: &str) -> Result<String, fartcode_core::Error> {
        Ok(path.trim_end_matches('/').to_string())
    }

    async fn list_dir(
        &self,
        path: &str,
        _include_hidden: bool,
    ) -> Result<Vec<RemoteEntry>, fartcode_core::Error> {
        let s = self.state.lock().unwrap();
        Ok(s.dirs
            .iter()
            .filter(|d| d.starts_with(&format!("{path}/")))
            .map(|d| RemoteEntry {
                path: d.clone(),
                name: d.rsplit('/').next().unwrap_or(d).to_string(),
                kind: RemoteFileKind::Dir,
            })
            .collect())
    }

    async fn stat(&self, path: &str) -> Result<Option<RemoteFileKind>, fartcode_core::Error> {
        let s = self.state.lock().unwrap();
        if s.dirs.contains(path) {
            Ok(Some(RemoteFileKind::Dir))
        } else if s.files.contains(path) {
            Ok(Some(RemoteFileKind::File))
        } else {
            Ok(None)
        }
    }

    async fn mkdir_all(&self, path: &str) -> Result<(), fartcode_core::Error> {
        self.state.lock().unwrap().dirs.insert(path.to_string());
        Ok(())
    }

    async fn remove_dir_all(&self, path: &str) -> Result<(), fartcode_core::Error> {
        let mut s = self.state.lock().unwrap();
        s.dirs
            .retain(|d| d != path && !d.starts_with(&format!("{path}/")));
        s.branches.remove(path);
        Ok(())
    }

    async fn run(
        &self,
        argv: &[&str],
        _cwd: Option<&str>,
    ) -> Result<RemoteOutput, fartcode_core::Error> {
        let mut s = self.state.lock().unwrap();
        s.commands
            .push(argv.iter().map(|a| a.to_string()).collect());

        Ok(match argv {
            ["git", "-C", path, "rev-parse", "--show-toplevel"] => {
                match s.repos.iter().find(|r| *r == path).cloned() {
                    Some(repo) => out(0, &format!("{repo}\n")),
                    None => out(128, ""),
                }
            }
            ["git", "-C", path, "remote"] => out(
                0,
                s.remotes.get(*path).cloned().unwrap_or_default().as_str(),
            ),
            ["git", "-C", path, "symbolic-ref", "--short", "HEAD"] => match s.branches.get(*path) {
                Some(b) => out(0, &format!("{b}\n")),
                None => out(128, ""),
            },
            ["git", "-C", path, "rev-parse", "--abbrev-ref", "HEAD"] => {
                match s.branches.get(*path) {
                    Some(b) => out(0, &format!("{b}\n")),
                    None => out(128, ""),
                }
            }
            ["git", "clone", _url, target] => {
                s.dirs.insert(target.to_string());
                s.repos.insert(target.to_string());
                s.branches.insert(target.to_string(), "main".into());
                s.remotes.insert(target.to_string(), "origin".into());
                out(0, "")
            }
            ["git", "-C", _repo, "worktree", "add", "-B", branch, path] => {
                s.dirs.insert(path.to_string());
                s.branches.insert(path.to_string(), branch.to_string());
                out(0, "")
            }
            ["git", "-C", _repo, "worktree", "prune"] => out(0, ""),
            ["pwd"] => out(0, "/home/dev\n"),
            _ => out(0, ""),
        })
    }
}

// ── Fixture ─────────────────────────────────────────────────

struct Fixture {
    _tmp: tempfile::TempDir,
    db: Arc<SqliteDb>,
    bus: Arc<BroadcastEventBus>,
    store: RemoteProjectStore,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db = SqliteDb::init(Some(tmp.path().join("test.db").to_str().unwrap())).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        let store = RemoteProjectStore::new(db.clone(), bus.clone());
        Self {
            _tmp: tmp,
            db,
            bus,
            store,
        }
    }

    fn project_count(&self) -> i64 {
        let conn = self.db.conn().lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap()
    }

    fn workspace_row(&self, id: &str) -> (String, String, Option<String>) {
        let conn = self.db.conn().lock().unwrap();
        conn.query_row(
            "SELECT type, location, ssh_connection_id FROM workspaces WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }
}

// ── Tests ──────────────────────────────────────────────────

#[tokio::test]
async fn create_remote_adds_an_ssh_project_and_emits_project_added() {
    let fx = Fixture::new();
    let mut events = fx.bus.subscribe();
    let host = FakeHost::with_repo("/srv/repos/app", Some("origin"), "main");

    let project = fx
        .store
        .create_remote(&host, CONN, "/srv/repos/app")
        .await
        .unwrap();

    assert_eq!(project.workspace_provider, WorkspaceProviderKind::Ssh);
    assert_eq!(project.ssh_connection_id.as_deref(), Some(CONN));
    assert_eq!(project.path.to_string_lossy(), "/srv/repos/app");
    // Plain branch takes the remote prefix, exactly like the local flow.
    assert_eq!(project.base_ref(), "origin/main");

    // The repository workspace is remote and carries the connection back.
    let workspace_id = project.repository_workspace_id.clone().unwrap();
    let (kind, location, conn) = fx.workspace_row(&workspace_id);
    assert_eq!(
        (kind.as_str(), location.as_str()),
        ("project-ssh", "remote")
    );
    assert_eq!(conn.as_deref(), Some(CONN));

    match events.try_recv() {
        Ok(InternalEvent::ProjectAdded { id, .. }) => assert_eq!(id, project.id),
        other => panic!("expected ProjectAdded, got {other:?}"),
    }
}

#[tokio::test]
async fn create_remote_is_idempotent_per_connection_and_path() {
    let fx = Fixture::new();
    let host = FakeHost::with_repo("/srv/repos/app", Some("origin"), "main");

    let first = fx
        .store
        .create_remote(&host, CONN, "/srv/repos/app")
        .await
        .unwrap();
    let second = fx
        .store
        .create_remote(&host, CONN, "/srv/repos/app/")
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(
        fx.project_count(),
        1,
        "duplicate path must reopen, not insert"
    );
}

#[tokio::test]
async fn same_path_on_a_different_connection_is_a_different_project() {
    let fx = Fixture::new();
    let host = FakeHost::with_repo("/srv/repos/app", Some("origin"), "main");

    let a = fx
        .store
        .create_remote(&host, "conn-a", "/srv/repos/app")
        .await
        .unwrap();
    let b = fx
        .store
        .create_remote(&host, "conn-b", "/srv/repos/app")
        .await
        .unwrap();

    assert_ne!(a.id, b.id);
    // #81: the workspace key (and therefore the worktree pool) is per
    // connection — two hosts with the same path must not share worktrees.
    assert_ne!(repository_workspace_key(&a), repository_workspace_key(&b));
    assert_ne!(remote_worktree_root(&a), remote_worktree_root(&b));
}

#[tokio::test]
async fn a_missing_path_and_a_non_repo_are_both_rejected_before_insert() {
    let fx = Fixture::new();
    let host = FakeHost::default();
    host.mkdir_all("/srv/plain").await.unwrap();

    let missing = fx
        .store
        .create_remote(&host, CONN, "/srv/nope")
        .await
        .unwrap_err();
    assert!(missing.to_string().contains("/srv/nope"), "{missing}");

    let not_repo = fx
        .store
        .create_remote(&host, CONN, "/srv/plain")
        .await
        .unwrap_err();
    assert!(
        not_repo.to_string().contains("not a git repository"),
        "{not_repo}"
    );
    assert_eq!(fx.project_count(), 0, "no row may survive a failed create");
}

#[tokio::test]
async fn clone_creates_the_projects_dir_and_refuses_an_occupied_target() {
    let fx = Fixture::new();
    let host = FakeHost::default();

    let project = fx
        .store
        .create_remote_clone(
            &host,
            CONN,
            "https://example.test/org/app.git",
            "/home/dev/fartCode",
        )
        .await
        .unwrap();
    assert_eq!(project.path.to_string_lossy(), "/home/dev/fartCode/app");
    assert!(host.exists("/home/dev/fartCode"), "parent dir is created");

    // Same URL again: the target now exists on the remote and is already a
    // project — reopen rather than clone over it.
    let again = fx
        .store
        .create_remote_clone(
            &host,
            CONN,
            "https://example.test/org/app.git",
            "/home/dev/fartCode",
        )
        .await
        .unwrap();
    assert_eq!(again.id, project.id);

    // A target that exists on disk but is NOT a project is an error, not an
    // overwrite.
    host.mkdir_all("/home/dev/fartCode/other").await.unwrap();
    let err = fx
        .store
        .create_remote_clone(
            &host,
            CONN,
            "https://example.test/org/other.git",
            "/home/dev/fartCode",
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("clone target already exists: /home/dev/fartCode/other"),
        "{err}"
    );
}

#[tokio::test]
async fn worktrees_live_under_the_project_and_teardown_is_idempotent() {
    let fx = Fixture::new();
    let host = FakeHost::with_repo("/srv/repos/app", Some("origin"), "main");
    let project = fx
        .store
        .create_remote(&host, CONN, "/srv/repos/app")
        .await
        .unwrap();

    let path = fx
        .store
        .ensure_remote_worktree(&host, &project, "feature-x")
        .await
        .unwrap();

    let root = remote_worktree_root(&project);
    assert!(root.starts_with("/srv/repos/app/.fartCode/worktrees/"));
    assert!(is_contained(&root, &path), "{path} must sit inside {root}");
    assert_eq!(path, remote_worktree_path(&project, "feature-x").unwrap());
    assert!(host.exists(&path));

    // Re-ensure reuses the existing checkout (no second `worktree add`).
    let again = fx
        .store
        .ensure_remote_worktree(&host, &project, "feature-x")
        .await
        .unwrap();
    assert_eq!(again, path);
    let adds = host
        .commands()
        .into_iter()
        .filter(|c| c.get(4).map(String::as_str) == Some("add"))
        .count();
    assert_eq!(adds, 1, "ensure_remote_worktree must be idempotent");

    fx.store
        .remove_remote_worktree(&host, &project, "feature-x")
        .await
        .unwrap();
    assert!(!host.exists(&path));
    // Second teardown: already gone, still Ok.
    fx.store
        .remove_remote_worktree(&host, &project, "feature-x")
        .await
        .unwrap();
}

#[tokio::test]
async fn every_remote_git_call_passes_the_path_as_an_argument() {
    let fx = Fixture::new();
    // A repo path built to break a shell: quotes, a semicolon, a space.
    let path = "/srv/re po/a';touch pwned;'";
    let host = FakeHost::with_repo(path, None, "feature/x");

    let project = fx.store.create_remote(&host, CONN, path).await.unwrap();
    // No remote + a slash-carrying branch: the base ref stays bare.
    assert_eq!(project.base_ref(), "feature/x");

    for command in host.commands() {
        // The path arrives as ONE argv element — never spliced into a string.
        if let Some(index) = command.iter().position(|a| a == "-C") {
            assert_eq!(command[index + 1], path);
        }
    }
}

#[tokio::test]
async fn a_connection_with_remote_projects_cannot_be_deleted() {
    let fx = Fixture::new();
    let connections = SshConnectionStore::new(fx.db.clone());
    let connection = connections
        .create(NewSshConnection {
            name: "box".into(),
            host: "example.test".into(),
            username: "dev".into(),
            ..Default::default()
        })
        .unwrap();
    let host = FakeHost::with_repo("/srv/repos/app", Some("origin"), "main");

    fx.store
        .create_remote(&host, &connection.id, "/srv/repos/app")
        .await
        .unwrap();

    // E12-03's refcount guard covers the new writer: the project row (and the
    // remote workspace row) both point at the profile.
    assert!(connections.reference_count(&connection.id).unwrap() >= 1);
    let err = connections.delete(&connection.id).unwrap_err();
    assert!(
        matches!(err, fartcode_core::Error::SshConnectionInUse { .. }),
        "{err}"
    );
}
