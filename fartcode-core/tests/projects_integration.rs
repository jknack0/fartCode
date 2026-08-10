//! Projects integration tests (ticket E1-03 acceptance criteria).
//!
//! All tests use `tempfile::tempdir()` + real local git repos — never the
//! real app data path and never `$HOME` (the `localProject` worktree/projects
//! dirs are overridden per fixture).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use fartcode_core::projects::{
    provider::{new_pool_segment, worktree_pool_path},
    worktrees::{EnsureWorktreeOptions, WorktreeManager},
    DbProjectStore, Project, ProjectStore, WorkspaceProviderKind,
};
use fartcode_core::settings::{DbSettingsStore, LocalProjectGroup, ProjectSettings, LOCAL_PROJECT};
use fartcode_git::{CliGit, GitOps};

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
                "user.email=t@fartCode.dev",
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
// Acceptance 1: add local dir → row + base ref + .fartCode/ excluded + event +
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
fn test_fartcode_git_excluded_on_create() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");
    fx.store.create_local(&repo, false).unwrap();

    let excludes = exclude_file(&repo);
    assert!(
        excludes.lines().any(|l| l.trim() == ".fartCode/"),
        ".git/info/exclude must contain .fartCode/: {excludes:?}"
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
    fx.git
        .branch_create(&repo, "fartCode/feature", "main")
        .unwrap();
    let wt_raw = fx.worktrees_dir.join("demo").join("fartCode-feature");
    std::fs::create_dir_all(wt_raw.parent().unwrap()).unwrap();
    fx.git
        .worktree_add(&repo, &wt_raw, "fartCode/feature")
        .unwrap();
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

    // #81 scheme: `<safe_path_segment>-<hash8(sha256(stored path))>`.
    let expected = format!("demo-{}", &sha256_hex(&repo.to_string_lossy())[..8]);
    let pool = worktree_pool_path(fx.db.as_ref(), fx.settings.as_ref(), &project).unwrap();
    assert_eq!(pool, fx.worktrees_dir.join(&expected));

    // The resolver stamps + persists the segment; a second resolution with
    // the stored segment returns the same pool.
    let stored: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT worktree_pool_segment FROM projects WHERE id = ?1",
            [&project.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, expected);
    let fetched = fx.store.get(&project.id).unwrap().unwrap();
    assert_eq!(
        fetched.worktree_pool_segment.as_deref(),
        Some(expected.as_str())
    );
    let again = worktree_pool_path(fx.db.as_ref(), fx.settings.as_ref(), &fetched).unwrap();
    assert_eq!(pool, again);
}

#[test]
fn test_pool_path_honors_project_override_and_falls_back() {
    let fx = Fixture::new();
    let repo = fx.make_repo("demo");
    let project = fx.store.create_local(&repo, false).unwrap();
    let segment = new_pool_segment(&project);

    // Per-project worktree_directory override wins over the app default (#81).
    let override_dir = fx._tmp.path().join("custom-pools");
    let mut ps = fx
        .settings
        .get_project_settings(&project.id, &repo)
        .unwrap();
    ps.worktree_directory = Some(override_dir.to_string_lossy().into_owned());
    fx.settings
        .update_project_settings(&project.id, &repo, &ps)
        .unwrap();
    let pool = worktree_pool_path(fx.db.as_ref(), fx.settings.as_ref(), &project).unwrap();
    assert_eq!(pool, override_dir.join(&segment));

    // Invalid override → normalization drops it on read → app default.
    ps.worktree_directory = Some("relative/path".into());
    fx.settings
        .update_project_settings(&project.id, &repo, &ps)
        .unwrap();
    let fetched = fx.store.get(&project.id).unwrap().unwrap();
    let pool = worktree_pool_path(fx.db.as_ref(), fx.settings.as_ref(), &fetched).unwrap();
    assert_eq!(pool, fx.worktrees_dir.join(&segment));
}

// ---------------------------------------------------------------------------
// #81: pool segments are unique per project (FIRST-58 regression)
// ---------------------------------------------------------------------------

/// Gives the project a real task worktree + workspace row via ensure_worktree.
fn add_task_worktree(fx: &Fixture, project: &Project, branch: &str) -> PathBuf {
    let wm = WorktreeManager::new(fx.db.clone(), fx.settings.clone(), Arc::new(CliGit));
    let workspace_id = format!("ws-{branch}");
    let task_id = format!("task-{branch}");
    {
        let conn = fx.db.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO workspaces (id, type, kind, location) VALUES (?1, 'local', 'worktree', 'local')",
            [&workspace_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, name, status, workspace_id)
             VALUES (?1, ?2, ?3, 'todo', ?4)",
            rusqlite::params![task_id, project.id, branch, workspace_id],
        )
        .unwrap();
    }
    wm.ensure_worktree(&EnsureWorktreeOptions {
        project,
        task_id: &task_id,
        workspace_id: &workspace_id,
        branch_name: branch,
        source_ref: Some("main"),
        worktree_enabled: true,
    })
    .unwrap()
    .path
}

#[test]
fn test_delete_same_basename_projects_keeps_sibling_worktrees() {
    // FIRST-58 regression (#81): two projects sharing a basename used to share
    // one pool, so deleting one destroyed the other's on-disk worktrees.
    let fx = Fixture::new();
    let repo_a = fx.make_repo("work/ade");
    let repo_b = fx.make_repo("archive/ade");
    let a = fx.store.create_local(&repo_a, false).unwrap();
    let b = fx.store.create_local(&repo_b, false).unwrap();
    assert_eq!(a.name, "ade");
    assert_eq!(b.name, "ade");

    let wt_a = add_task_worktree(&fx, &a, "task-a");
    let wt_b = add_task_worktree(&fx, &b, "task-b");
    assert_ne!(
        wt_a.parent().unwrap(),
        wt_b.parent().unwrap(),
        "pools must be distinct"
    );

    fx.store.delete(&a.id).unwrap();

    // A's worktree gone with its pool; B's worktree + recorded path intact.
    assert!(!wt_a.exists());
    assert!(wt_b.exists(), "sibling worktree must survive");
    git_ok(&wt_b, ["status"]);
    let recorded: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT path FROM workspaces WHERE id IN (SELECT workspace_id FROM tasks WHERE project_id = ?1)", [&b.id], |r| r.get(0))
        .unwrap();
    assert_eq!(recorded, wt_b.to_string_lossy().into_owned());
}

/// Standalone git repo helper for the adoption tests (rows are inserted
/// BEFORE the store exists, so the fixture's create flow doesn't apply).
fn init_repo(tmp: &tempfile::TempDir, name: &str) -> PathBuf {
    let repo = tmp.path().join(name);
    std::fs::create_dir_all(&repo).unwrap();
    let git = CliGit;
    git.init(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git_ok(&repo, ["add", "."]);
    git_ok(
        &repo,
        [
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@fartCode.dev",
            "commit",
            "-m",
            "init",
        ],
    );
    git_ok(&repo, ["branch", "-M", "main"]);
    std::fs::canonicalize(&repo).unwrap()
}

/// Pre-#81 fixture: migrated DB, two same-basename project rows with no
/// segment, worktrees in the shared legacy pool. Returns the pieces.
fn legacy_collision_fixture(
    tmp: &tempfile::TempDir,
) -> (
    Arc<SqliteDb>,
    Arc<DbSettingsStore>,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let db = SqliteDb::init(Some(tmp.path().join("test.db").to_str().unwrap())).unwrap();
    let settings = Arc::new(DbSettingsStore::new(db.clone()));
    let worktrees_dir = tmp.path().join("worktrees");
    std::fs::create_dir_all(&worktrees_dir).unwrap();
    settings
        .set(
            &LOCAL_PROJECT,
            LocalProjectGroup {
                default_projects_directory: tmp
                    .path()
                    .join("repositories")
                    .to_string_lossy()
                    .into_owned(),
                default_worktree_directory: worktrees_dir.to_string_lossy().into_owned(),
                write_agent_config_to_git_ignore: true,
            },
        )
        .unwrap();

    let repo1 = init_repo(tmp, "work/ade");
    let repo2 = init_repo(tmp, "archive/ade");

    // Pre-upgrade rows: no worktree_pool_segment.
    let conn = db.conn().lock().unwrap();
    conn.execute(
        "INSERT INTO projects (id, name, path, workspace_provider, created_at)
         VALUES ('p1', 'ade', ?1, 'local', '2024-01-01 00:00:00')",
        [repo1.to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO projects (id, name, path, workspace_provider, created_at)
         VALUES ('p2', 'ade', ?1, 'local', '2024-01-02 00:00:00')",
        [repo2.to_string_lossy()],
    )
    .unwrap();
    drop(conn);

    // Legacy shared pool: one worktree per project. Branch names are NESTED
    // (`<branch_prefix>/<branch>`, production layout — see naming.rs) so the
    // adoption move's nested-target dir creation is exercised (F1).
    let shared = worktrees_dir.join("ade");
    let git = CliGit;
    git.branch_create(&repo1, "fartCode/b1", "main").unwrap();
    git.worktree_add(&repo1, &shared.join("fartCode/b1"), "fartCode/b1")
        .unwrap();
    git.branch_create(&repo2, "fartCode/b2", "main").unwrap();
    git.worktree_add(&repo2, &shared.join("fartCode/b2"), "fartCode/b2")
        .unwrap();
    let conn = db.conn().lock().unwrap();
    conn.execute(
        "INSERT INTO workspaces (id, kind, path) VALUES ('w1', 'worktree', ?1)",
        [shared.join("fartCode/b1").to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO workspaces (id, kind, path) VALUES ('w2', 'worktree', ?1)",
        [shared.join("fartCode/b2").to_string_lossy()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tasks (id, project_id, name, status, workspace_id)
         VALUES ('t1', 'p1', 'task', 'todo', 'w1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tasks (id, project_id, name, status, workspace_id)
         VALUES ('t2', 'p2', 'task', 'todo', 'w2')",
        [],
    )
    .unwrap();
    drop(conn);
    (db, settings, worktrees_dir, repo1, repo2)
}

#[test]
fn test_adoption_moves_colliding_legacy_pool_worktrees() {
    let tmp = tempfile::tempdir().unwrap();
    let (db, settings, worktrees_dir, _repo1, repo2) = legacy_collision_fixture(&tmp);
    let shared = worktrees_dir.join("ade");

    // Store construction runs the one-shot adoption pass (#81).
    let store = DbProjectStore::new(
        db.clone(),
        settings.clone(),
        Arc::new(CliGit),
        Arc::new(BroadcastEventBus::new(16)),
    );
    assert_eq!(
        db.kv_get("worktree_pool_adoption_v1").unwrap().as_deref(),
        Some("done")
    );

    // Both had worktrees on disk → tiebreak by earliest created_at: p1 keeps
    // the legacy dir, p2 moves to its new-scheme pool.
    let p1 = store.get("p1").unwrap().unwrap();
    let p2 = store.get("p2").unwrap().unwrap();
    assert_eq!(p1.worktree_pool_segment.as_deref(), Some("ade"));
    assert_eq!(p2.worktree_pool_segment, Some(new_pool_segment(&p2)));

    let pool1 = worktree_pool_path(db.as_ref(), settings.as_ref(), &p1).unwrap();
    let pool2 = worktree_pool_path(db.as_ref(), settings.as_ref(), &p2).unwrap();
    assert_eq!(pool1, shared, "sole keeper keeps the legacy dir");
    assert_ne!(pool1, pool2, "pools must be distinct after adoption");

    // p2's worktree moved + repaired: valid on disk, DB row repointed.
    let moved = pool2.join("fartCode/b2");
    assert!(!shared.join("fartCode/b2").exists());
    assert!(moved.exists());
    git_ok(&moved, ["status"]);
    let recorded: String = db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT path FROM workspaces WHERE id = 'w2'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(recorded, moved.to_string_lossy().into_owned());

    // ensure_worktree REUSE finds the moved worktree.
    let wm = WorktreeManager::new(db.clone(), settings.clone(), Arc::new(CliGit));
    let result = wm
        .ensure_worktree(&EnsureWorktreeOptions {
            project: &p2,
            task_id: "t2",
            workspace_id: "w2",
            branch_name: "fartCode/b2",
            source_ref: Some("main"),
            worktree_enabled: true,
        })
        .unwrap();
    assert!(result.reused);
    // git reports realpath (/private/var vs /var) — compare canonicalized.
    assert_eq!(result.path, std::fs::canonicalize(&moved).unwrap());

    // Re-run is a no-op: another store construction changes nothing.
    let store2 = DbProjectStore::new(
        db.clone(),
        settings.clone(),
        Arc::new(CliGit),
        Arc::new(BroadcastEventBus::new(16)),
    );
    assert_eq!(
        store2.get("p2").unwrap().unwrap().worktree_pool_segment,
        Some(new_pool_segment(&store2.get("p2").unwrap().unwrap()))
    );
    let _ = repo2;
}

#[test]
fn test_adoption_sole_project_keeps_legacy_segment() {
    let tmp = tempfile::tempdir().unwrap();
    let db = SqliteDb::init(Some(tmp.path().join("test.db").to_str().unwrap())).unwrap();
    let settings = Arc::new(DbSettingsStore::new(db.clone()));
    let worktrees_dir = tmp.path().join("worktrees");
    settings
        .set(
            &LOCAL_PROJECT,
            LocalProjectGroup {
                default_projects_directory: tmp
                    .path()
                    .join("repositories")
                    .to_string_lossy()
                    .into_owned(),
                default_worktree_directory: worktrees_dir.to_string_lossy().into_owned(),
                write_agent_config_to_git_ignore: true,
            },
        )
        .unwrap();
    let repo = init_repo(&tmp, "demo");
    db.conn()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO projects (id, name, path, workspace_provider) VALUES ('p1', 'demo', ?1, 'local')",
            [repo.to_string_lossy()],
        )
        .unwrap();

    let store = DbProjectStore::new(
        db.clone(),
        settings.clone(),
        Arc::new(CliGit),
        Arc::new(BroadcastEventBus::new(16)),
    );

    // Sole claimant adopts the legacy segment in place — no dir churn.
    let p = store.get("p1").unwrap().unwrap();
    assert_eq!(p.worktree_pool_segment.as_deref(), Some("demo"));
    let pool = worktree_pool_path(db.as_ref(), settings.as_ref(), &p).unwrap();
    assert_eq!(pool, worktrees_dir.join("demo"));
}

#[test]
fn test_adoption_interrupted_run_mover_gets_distinct_segment() {
    // F6 regression: a crash between keeper stamp and mover completion leaves
    // one project stamped with the legacy segment and the other unstamped. On
    // re-run the sole-member group must NOT adopt the (now taken) legacy
    // segment in place — it becomes a mover with a distinct segment.
    let tmp = tempfile::tempdir().unwrap();
    let (db, settings, worktrees_dir, _repo1, _repo2) = legacy_collision_fixture(&tmp);
    db.conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET worktree_pool_segment = 'ade' WHERE id = 'p1'",
            [],
        )
        .unwrap();

    let store = DbProjectStore::new(
        db.clone(),
        settings.clone(),
        Arc::new(CliGit),
        Arc::new(BroadcastEventBus::new(16)),
    );

    let p2 = store.get("p2").unwrap().unwrap();
    assert_ne!(
        p2.worktree_pool_segment.as_deref(),
        Some("ade"),
        "duplicate legacy segment forbidden"
    );
    assert_eq!(p2.worktree_pool_segment, Some(new_pool_segment(&p2)));

    let pool2 = worktree_pool_path(db.as_ref(), settings.as_ref(), &p2).unwrap();
    let moved = pool2.join("fartCode/b2");
    assert!(!worktrees_dir.join("ade/fartCode/b2").exists());
    assert!(moved.exists());
    git_ok(&moved, ["status"]);

    // The moved worktree resolves and is reusable.
    let wm = WorktreeManager::new(db.clone(), settings.clone(), Arc::new(CliGit));
    let result = wm
        .ensure_worktree(&EnsureWorktreeOptions {
            project: &p2,
            task_id: "t2",
            workspace_id: "w2",
            branch_name: "fartCode/b2",
            source_ref: Some("main"),
            worktree_enabled: true,
        })
        .unwrap();
    assert!(result.reused);
    // Keeper's worktree untouched.
    assert!(worktrees_dir.join("ade/fartCode/b1").exists());
}

#[test]
fn test_adoption_moves_legacy_pool_to_override_root() {
    // F3 regression: pre-#81 the per-project worktree_directory override was
    // dead, so the legacy pool sits under the app default even for
    // override-having projects. Adoption must relocate it to the override
    // root (keeping the legacy segment) or the resolver would orphan it.
    let tmp = tempfile::tempdir().unwrap();
    let (db, settings, worktrees_dir, repo1, _repo2) = legacy_collision_fixture(&tmp);
    // Make p1 a sole claimant.
    db.conn()
        .lock()
        .unwrap()
        .execute_batch(
            "DELETE FROM tasks WHERE project_id = 'p2';
             DELETE FROM workspaces WHERE id = 'w2';
             DELETE FROM projects WHERE id = 'p2';",
        )
        .unwrap();
    let override_dir = tmp.path().join("override");
    std::fs::create_dir_all(&override_dir).unwrap();
    let ps = ProjectSettings {
        worktree_directory: Some(override_dir.to_string_lossy().into_owned()),
        ..Default::default()
    };
    settings.update_project_settings("p1", &repo1, &ps).unwrap();

    let store = DbProjectStore::new(
        db.clone(),
        settings.clone(),
        Arc::new(CliGit),
        Arc::new(BroadcastEventBus::new(16)),
    );

    let p1 = store.get("p1").unwrap().unwrap();
    assert_eq!(p1.worktree_pool_segment.as_deref(), Some("ade"));
    let pool = worktree_pool_path(db.as_ref(), settings.as_ref(), &p1).unwrap();
    assert_eq!(pool, override_dir.join("ade"));

    let moved = pool.join("fartCode/b1");
    assert!(!worktrees_dir.join("ade/fartCode/b1").exists());
    assert!(moved.exists());
    git_ok(&moved, ["status"]);
    let recorded: String = db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT path FROM workspaces WHERE id = 'w1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(recorded, moved.to_string_lossy().into_owned());
}

#[test]
fn test_delete_skips_pool_containing_foreign_worktrees() {
    // F2b regression: a half-finished adoption can leave two projects sharing
    // one pool dir. Deleting one must skip teardown when another project's
    // worktrees live inside the pool.
    let fx = Fixture::new();
    let a = fx
        .store
        .create_local(&fx.make_repo("alpha"), false)
        .unwrap();
    let b = fx.store.create_local(&fx.make_repo("beta"), false).unwrap();
    let wt_b = add_task_worktree(&fx, &b, "task-b");

    // Force a's pool onto b's pool dir (shared by construction).
    let seg_b = fx
        .store
        .get(&b.id)
        .unwrap()
        .unwrap()
        .worktree_pool_segment
        .clone()
        .unwrap();
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE projects SET worktree_pool_segment = ?1 WHERE id = ?2",
            rusqlite::params![seg_b, a.id],
        )
        .unwrap();

    fx.store.delete(&a.id).unwrap();

    // a's row gone, but the shared pool dir and b's worktree survive.
    assert!(fx.store.get(&a.id).unwrap().is_none());
    let pool = fx.worktrees_dir.join(&seg_b);
    assert!(pool.exists(), "shared pool dir must NOT be removed");
    assert!(wt_b.exists(), "foreign worktree must survive");
    assert!(wt_b.join("README.md").exists());
    git_ok(&wt_b, ["status"]);
}

#[test]
fn test_ensure_worktree_refuses_dirty_stale_path() {
    // F5b regression: a stale (broken-linkage) worktree path holding
    // uncommitted work must NOT be removed — ensure_worktree fails loud and
    // the directory survives with its contents.
    let fx = Fixture::new();
    let repo = fx.make_repo("stale");
    let project = fx.store.create_local(&repo, false).unwrap();
    let wt = add_task_worktree(&fx, &project, "task-x");
    std::fs::write(wt.join("WIP.txt"), "precious uncommitted work\n").unwrap();

    // Break the git linkage (what a failed adoption repair + prune leaves).
    let admin = repo.join(".git/worktrees");
    for entry in std::fs::read_dir(&admin).unwrap() {
        std::fs::remove_dir_all(entry.unwrap().path()).unwrap();
    }

    let wm = WorktreeManager::new(fx.db.clone(), fx.settings.clone(), Arc::new(CliGit));
    let err = wm
        .ensure_worktree(&EnsureWorktreeOptions {
            project: &project,
            task_id: "task-task-x",
            workspace_id: "ws-task-x",
            branch_name: "task-x",
            source_ref: Some("main"),
            worktree_enabled: true,
        })
        .expect_err("stale path with unverifiable cleanliness must refuse");
    assert!(
        err.to_string().contains("refusing to remove"),
        "unexpected error: {err}"
    );
    assert!(
        wt.join("WIP.txt").exists(),
        "dirty stale worktree must survive"
    );
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input.as_bytes());
    format!("{:x}", h.finalize())
}

// ---------------------------------------------------------------------------
// Clone flow
// ---------------------------------------------------------------------------

#[test]
fn test_create_clone_clones_into_projects_dir() {
    let fx = Fixture::new();
    // A bare "remote" with a main branch.
    let bare = fx._tmp.path().join("fartCode.git");
    Command::new("git")
        .args(["init", "--bare", bare.to_str().unwrap()])
        .status()
        .unwrap();
    let seed = fx._tmp.path().join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    git_ok(&seed, ["init"]);
    std::fs::write(seed.join("README.md"), "# fartCode\n").unwrap();
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

    let target = fx.projects_dir.join("fartCode");
    assert!(
        target.exists(),
        "clone must land in the configured projects dir"
    );
    // git reports the clone at the path it was given (no symlink resolution).
    assert_eq!(project.path, target);
    assert_eq!(project.name, "fartCode");
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
