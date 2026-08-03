//! Projects integration tests (ticket E1-03 acceptance criteria).
//!
//! All tests use `tempfile::tempdir()` + real local git repos — never the
//! real app data path and never `$HOME` (the `localProject` worktree/projects
//! dirs are overridden per fixture).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use ade_core::db::{Db, SqliteDb};
use ade_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use ade_core::projects::{
    provider::worktree_pool_path, DbProjectStore, ProjectStore, WorkspaceProviderKind,
};
use ade_core::settings::{DbSettingsStore, LocalProjectGroup, LOCAL_PROJECT};
use ade_git::{CliGit, GitOps};

struct Fixture {
    _tmp: tempfile::TempDir,
    db: Arc<SqliteDb>,
    settings: Arc<DbSettingsStore>,
    store: DbProjectStore,
    git: CliGit,
    bus: Arc<BroadcastEventBus>,
    projects_dir: PathBuf,
    worktrees_dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
        let settings = Arc::new(DbSettingsStore::new(db.clone()));

        // Point localProject dirs at the temp dir so nothing touches $HOME.
        // projects_dir is intentionally NOT pre-created: create_clone must
        // create it (fresh-install path).
        let projects_dir = tmp.path().join("repositories");
        let worktrees_dir = tmp.path().join("worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();
        settings
            .set(
                &LOCAL_PROJECT,
                LocalProjectGroup {
                    default_projects_directory: projects_dir.to_string_lossy().into_owned(),
                    default_worktree_directory: worktrees_dir.to_string_lossy().into_owned(),
                    write_agent_config_to_git_ignore: true,
                },
            )
            .unwrap();

        let bus = Arc::new(BroadcastEventBus::new(16));
        let git = CliGit;
        let store =
            DbProjectStore::new(db.clone(), settings.clone(), Arc::new(CliGit), bus.clone());
        Self {
            _tmp: tmp,
            db,
            settings,
            store,
            git,
            bus,
            projects_dir,
            worktrees_dir,
        }
    }

    /// Creates a real git repo at `tmp/<name>` with one commit on `main`.
    fn make_repo(&self, name: &str) -> PathBuf {
        let repo = self._tmp.path().join(name);
        std::fs::create_dir_all(&repo).unwrap();
        self.git.init(&repo).unwrap();
        std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
        git_ok(&repo, ["add", "."]);
        git_ok(
            &repo,
            [
                "-c",
                "user.name=Test",
                "-c",
                "user.email=t@ade.dev",
                "commit",
                "-m",
                "init",
            ],
        );
        git_ok(&repo, ["branch", "-M", "main"]);
        // git reports realpaths (/private/var vs /var); canonicalize so
        // comparisons against project.path (show_toplevel) are exact.
        std::fs::canonicalize(&repo).unwrap()
    }
}

fn git_ok<I, S>(repo: &Path, args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git failed in {repo:?}");
}

fn exclude_file(repo: &Path) -> String {
    std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Acceptance 1: add local dir → row + base ref + .ade/ excluded + event +
// duplicate opens existing
// ---------------------------------------------------------------------------

#[test]
fn test_create_local_creates_project_row() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");
    let mut rx = fx.bus.subscribe();

    let project = fx.store.create_local(&repo, false).unwrap();

    assert_eq!(project.name, "demo");
    assert_eq!(project.path, repo);
    assert_eq!(project.workspace_provider, WorkspaceProviderKind::Local);
    assert_eq!(
        project.base_ref(),
        "main",
        "base ref must resolve to the default branch"
    );

    // Row persisted.
    let fetched = fx.store.get(&project.id).unwrap().unwrap();
    assert_eq!(fetched, project);

    // project:added emitted.
    match rx.try_recv().unwrap() {
        InternalEvent::ProjectAdded { id, name, path } => {
            assert_eq!(id, project.id);
            assert_eq!(name, "demo");
            assert_eq!(path, repo.to_string_lossy());
        }
        other => panic!("expected ProjectAdded, got {other:?}"),
    }
}

#[test]
fn test_ade_git_excluded_on_create() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");
    fx.store.create_local(&repo, false).unwrap();

    let excludes = exclude_file(&repo);
    assert!(
        excludes.lines().any(|l| l.trim() == ".ade/"),
        ".git/info/exclude must contain .ade/: {excludes:?}"
    );
}

#[test]
fn test_duplicate_path_opens_existing() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");

    let first = fx.store.create_local(&repo, false).unwrap();
    let second = fx.store.create_local(&repo, false).unwrap();

    assert_eq!(
        first.id, second.id,
        "duplicate add must return the existing project"
    );
    let count: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "exactly one row after duplicate add");
}

// ---------------------------------------------------------------------------
// Acceptance 2: initialize git repository when absent
// ---------------------------------------------------------------------------

#[test]
fn test_create_local_initializes_git_repo() {
    let fx = Fixture::new();
    let dir = fx._tmp.path().join("fresh");
    std::fs::create_dir_all(&dir).unwrap();

    // Without init_if_missing → error.
    let err = fx.store.create_local(&dir, false).unwrap_err();
    assert!(err.to_string().contains("not a git repository"));

    // With init_if_missing → repo created + project row.
    let project = fx.store.create_local(&dir, true).unwrap();
    assert!(fx.git.is_git_repo(&dir).unwrap());
    // An unborn repo resolves to the branch git is about to create (typically
    // "main" or "master" depending on init.defaultBranch) — the point is the
    // ref was resolved, not hardcoded.
    let detected = fx.git.current_branch(&dir).unwrap().unwrap();
    assert_eq!(
        project.base_ref(),
        detected,
        "base ref must match the detected branch"
    );
    assert!(!project.base_ref().is_empty());
}

// ---------------------------------------------------------------------------
// Acceptance 3: close/open cycle re-detects worktrees
// ---------------------------------------------------------------------------

#[test]
fn test_close_open_redetects_worktrees() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");
    let project = fx.store.create_local(&repo, false).unwrap();

    // A task worktree appears (as E2-02 would create it).
    fx.git.branch_create(&repo, "ade/feature", "main").unwrap();
    let wt_raw = fx.worktrees_dir.join("demo").join("ade-feature");
    std::fs::create_dir_all(wt_raw.parent().unwrap()).unwrap();
    fx.git.worktree_add(&repo, &wt_raw, "ade/feature").unwrap();
    let wt = std::fs::canonicalize(&wt_raw).unwrap();

    fx.store.close(&project.id).unwrap();
    let opened = fx.store.open(&project.id).unwrap();

    assert!(
        opened.worktrees.iter().any(|w| w.path == wt),
        "worktree must be re-detected on open: {:?}",
        opened.worktrees
    );
}

// ---------------------------------------------------------------------------
// Repository workspace + pool path
// ---------------------------------------------------------------------------

#[test]
fn test_repository_workspace_created() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");
    let project = fx.store.create_local(&repo, false).unwrap();

    let (workspace_id, key): (Option<String>, String) = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT w.id, w.key FROM workspaces w WHERE w.kind = 'project-root' AND w.path = ?1",
            [repo.to_string_lossy()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    // projects.repository_workspace_id points at it.
    let stored = fx.store.get(&project.id).unwrap().unwrap();
    assert_eq!(
        stored.repository_workspace_id.as_deref(),
        workspace_id.as_deref()
    );
    // Key is sha256("local:<path>").
    let expected = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(format!("local:{}", repo.display()).as_bytes());
        format!("{:x}", h.finalize())
    };
    assert_eq!(key, expected);
}

#[test]
fn test_worktree_pool_path_uses_safe_segment() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");
    let project = fx.store.create_local(&repo, false).unwrap();

    let pool = worktree_pool_path(fx.settings.as_ref(), &project).unwrap();
    assert_eq!(pool, fx.worktrees_dir.join("demo"));
}

// ---------------------------------------------------------------------------
// Clone flow
// ---------------------------------------------------------------------------

#[test]
fn test_create_clone_clones_into_projects_dir() {
    let fx = Fixture::new();
    // A bare "remote" with a main branch.
    let bare = fx._tmp.path().join("ade.git");
    Command::new("git")
        .args(["init", "--bare", bare.to_str().unwrap()])
        .status()
        .unwrap();
    let seed = fx._tmp.path().join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git_ok(&seed, ["init"]);
    std::fs::write(seed.join("README.md"), "# ade\n").unwrap();
    git_ok(&seed, ["add", "."]);
    git_ok(
        &seed,
        [
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "commit",
            "-m",
            "init",
        ],
    );
    git_ok(&seed, ["branch", "-M", "main"]);
    git_ok(&seed, ["remote", "add", "origin", bare.to_str().unwrap()]);
    git_ok(&seed, ["push", "-u", "origin", "main"]);
    // Point the bare repo's HEAD at main so clones resolve origin/HEAD.
    Command::new("git")
        .args([
            "--git-dir",
            bare.to_str().unwrap(),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ])
        .status()
        .unwrap();

    let url = bare.to_str().unwrap().to_string();
    let project = fx.store.create_clone(&url).unwrap();

    let target = fx.projects_dir.join("ade");
    assert!(
        target.exists(),
        "clone must land in the configured projects dir"
    );
    // git reports the clone at the path it was given (no symlink resolution).
    assert_eq!(project.path, target);
    assert_eq!(project.name, "ade");
    assert_eq!(
        project.base_ref(),
        "origin/main",
        "clone base ref resolves via remote HEAD"
    );
}

#[test]
fn test_gitflow_branch_keeps_bare_base_ref() {
    let fx = Fixture::new();
    let repo = fx.make_repo("gitflow");
    // Push main to a remote so origin/HEAD resolves, then work on a gitflow
    // branch. The reference computeBaseRef keeps slash branches bare.
    let bare = fx._tmp.path().join("gitflow.git");
    Command::new("git")
        .args(["init", "--bare", bare.to_str().unwrap()])
        .status()
        .unwrap();
    git_ok(&repo, ["remote", "add", "origin", bare.to_str().unwrap()]);
    git_ok(&repo, ["push", "-u", "origin", "main"]);
    Command::new("git")
        .args([
            "--git-dir",
            bare.to_str().unwrap(),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ])
        .status()
        .unwrap();
    git_ok(&repo, ["checkout", "-b", "feature/login"]);

    let project = fx.store.create_local(&repo, false).unwrap();
    assert_eq!(
        project.base_ref(),
        "feature/login",
        "slash-containing branches must stay bare (reference normalize())"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 4: deletion is explicit + close is a no-op stub (teardown mode
// documented in provider.rs — session teardown arrives with E2-05/E2-02)
// ---------------------------------------------------------------------------

#[test]
fn test_delete_removes_project_only_when_requested() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");
    let project = fx.store.create_local(&repo, false).unwrap();

    // A close does not remove anything.
    fx.store.close(&project.id).unwrap();
    assert!(fx.store.get(&project.id).unwrap().is_some());

    let mut rx = fx.bus.subscribe();
    fx.store.delete(&project.id).unwrap();
    assert!(fx.store.get(&project.id).unwrap().is_none());
    match rx.try_recv().unwrap() {
        InternalEvent::ProjectDeleted { id } => assert_eq!(id, project.id),
        other => panic!("expected ProjectDeleted, got {other:?}"),
    }
}

#[test]
fn test_list_returns_projects() {
    let fx = Fixture::new();
    fx.store.create_local(&fx.make_repo("a"), false).unwrap();
    fx.store.create_local(&fx.make_repo("b"), false).unwrap();
    let projects = fx.store.list().unwrap();
    assert_eq!(projects.len(), 2);
}
