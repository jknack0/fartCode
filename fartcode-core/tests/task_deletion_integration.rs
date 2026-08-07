//! E2-09 — Task deletion/teardown integration tests (ticket acceptance
//! criteria):
//!
//! 1. Delete removes worktree + rows + view state; running sessions reaped.
//! 2. Sibling-task scenario: worktree kept; task row removed.
//! 3. Double-delete and delete-during-provision are safe (idempotent).
//!
//! Plus the reference guards: project-root workspaces outlive tasks, branch
//! deletion only for created branches that differ from their source.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use fartcode_core::conversations::DbConversationStore;
use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::events::BroadcastEventBus;
use fartcode_core::projects::worktrees::WorktreeManager;
use fartcode_core::projects::{DbProjectStore, ProjectStore};
use fartcode_core::pty::sessions::SessionRegistry;
use fartcode_core::settings::{DbSettingsStore, LocalProjectGroup, LOCAL_PROJECT};
use fartcode_core::tasks::deletion::{DeleteTaskOptions, TaskDeletionService};
use fartcode_core::tasks::operations::{
    CreateTaskParams, GitSetup, InitialConversationConfig, SourceBranchRef, TaskConfigParams,
    TaskCreationService,
};
use fartcode_core::tasks::{CreateTaskOptions, DbTaskStore, TaskStore, WorkspaceTarget};
use fartcode_core::view_state;
use fartcode_git::CliGit;

struct Fixture {
    _tmp: tempfile::TempDir,
    db: Arc<SqliteDb>,
    tasks: Arc<DbTaskStore>,
    creation: TaskCreationService,
    deletion: TaskDeletionService,
    sessions: Arc<SessionRegistry>,
    project_id: String,
    repo: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db = SqliteDb::init_in_memory().unwrap();
        let settings = Arc::new(DbSettingsStore::new(db.clone()));

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

        let bus = Arc::new(BroadcastEventBus::new(32));
        let projects = Arc::new(DbProjectStore::new(
            db.clone(),
            settings.clone(),
            Arc::new(CliGit),
            bus.clone(),
        ));
        let repo = make_repo(&tmp, "demo");
        let project = projects.create_local(&repo, false).unwrap();

        let creation = TaskCreationService::new(
            db.clone(),
            settings.clone(),
            Arc::new(CliGit),
            WorktreeManager::new(db.clone(), settings.clone(), Arc::new(CliGit)),
            bus.clone(),
        );

        let tasks = Arc::new(DbTaskStore::new(db.clone(), bus.clone()));
        let conversations = Arc::new(DbConversationStore::new(db.clone(), bus.clone()));
        let sessions = Arc::new(SessionRegistry::new());
        let deletion = TaskDeletionService::new(
            db.clone(),
            tasks.clone(),
            conversations.clone(),
            projects.clone(),
            WorktreeManager::new(db.clone(), settings.clone(), Arc::new(CliGit)),
            sessions.clone(),
        );

        Self {
            _tmp: tmp,
            db,
            tasks,
            creation,
            deletion,
            sessions,
            project_id: project.id.clone(),
            repo,
        }
    }

    fn db_dyn(&self) -> Arc<dyn Db> {
        self.db.clone()
    }

    fn params(&self, name: &str, branch: &str) -> CreateTaskParams {
        CreateTaskParams {
            id: None,
            project_id: self.project_id.clone(),
            task_config: TaskConfigParams {
                name: name.into(),
                initial_status: None,
                linked_issue: None,
                // Every real task carries a conversation — the teardown path
                // needs the leaf ids (session registry + tmux kill).
                initial_conversation: Some(InitialConversationConfig::new(
                    format!("conv-{}", name.replace(' ', "-")),
                    "claude",
                    name,
                )),
            },
            git: GitSetup::CreateBranch {
                branch_name: branch.into(),
                from_branch: SourceBranchRef::local("main"),
                push_branch: false,
            },
            workspace: WorkspaceTarget::NewWorktree,
            automation_run_id: None,
        }
    }

    /// Creates + provisions a task; returns (task_id, worktree_path).
    fn create_provisioned(&self, name: &str, branch: &str) -> (String, PathBuf) {
        let result = self
            .creation
            .create_with_provision(self.params(name, branch))
            .unwrap();
        let path: String = self
            .db
            .conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT path FROM workspaces WHERE id = ?1",
                [&result.task.workspace_id.clone().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        (result.task.id, PathBuf::from(path))
    }

    /// Workspaces excluding the project's repository workspace (which is
    /// seeded by `create_local` and intentionally outlives every task).
    fn task_workspace_count(&self) -> i64 {
        self.db
            .conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM workspaces WHERE id NOT IN \
                 (SELECT repository_workspace_id FROM projects \
                  WHERE repository_workspace_id IS NOT NULL)",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn count(&self, table: &str) -> i64 {
        self.db
            .conn()
            .lock()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    fn branch_exists(&self, name: &str) -> bool {
        Command::new("git")
            .args(["rev-parse", "--verify", &format!("refs/heads/{name}")])
            .current_dir(&self.repo)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
}

fn make_repo(tmp: &tempfile::TempDir, name: &str) -> PathBuf {
    let repo = tmp.path().join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git_ok(&repo, ["init", "-q"]);
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

fn git_ok<const N: usize>(repo: &std::path::Path, args: [&str; N]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

// ---------------------------------------------------------------------------
// Acceptance 1: delete removes worktree + rows + view state; session killed
// ---------------------------------------------------------------------------

#[test]
fn delete_removes_worktree_rows_view_state_and_reaps_session() {
    let fx = Fixture::new();
    let (task_id, worktree) = fx.create_provisioned("ship it", "fartCode/ship-it");
    assert!(worktree.exists(), "worktree provisioned");

    // View state seeded by the UI (E1-08).
    let db = fx.db_dyn();
    view_state::save(
        &db,
        &format!("view-state:task:{task_id}"),
        &serde_json::json!({"panes": []}),
    )
    .unwrap();
    view_state::save(
        &db,
        &format!("view-state:task:{task_id}:tabs"),
        &serde_json::json!([]),
    )
    .unwrap();

    // A "running" session: the registry holds it; a launcher-shaped thread
    // watches the cancel flag and reaps (sets exited).
    let conv_id: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT id FROM conversations WHERE task_id = ?1",
            [&task_id],
            |r| r.get(0),
        )
        .unwrap();
    let session_id = format!("{}:{task_id}:{conv_id}", fx.project_id);
    let flags = fx.sessions.register(&session_id, Some("claude".into()));
    let waiter_flags = flags.clone();
    let waiter = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while !waiter_flags
            .cancel
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        waiter_flags
            .exited
            .store(true, std::sync::atomic::Ordering::SeqCst);
        true
    });

    fx.deletion
        .delete_task(&fx.project_id, &task_id, &DeleteTaskOptions::default())
        .unwrap();

    // Process reaped (the simulated launcher observed the cancel).
    assert!(waiter.join().unwrap(), "session was not cancelled");
    assert!(!fx.sessions.is_active(&session_id), "session unregistered");

    // Worktree gone (metadata pruned too).
    assert!(!worktree.exists(), "worktree removed");
    let wt_list = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&fx.repo)
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&wt_list.stdout).contains(worktree.to_str().unwrap()),
        "worktree metadata pruned"
    );

    // Rows gone.
    assert_eq!(fx.count("tasks"), 0, "task row deleted");
    assert_eq!(fx.task_workspace_count(), 0, "workspace row deleted");
    assert_eq!(fx.count("conversations"), 0, "conversations cascade");

    // View state dropped.
    assert!(view_state::get(&db, &format!("view-state:task:{task_id}"))
        .unwrap()
        .is_none());
    assert!(
        view_state::get(&db, &format!("view-state:task:{task_id}:tabs"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn delete_branch_only_when_created_from_different_branch() {
    let fx = Fixture::new();
    let (task_id, _) = fx.create_provisioned("branching", "fartCode/branching");
    assert!(fx.branch_exists("fartCode/branching"));

    // Default options: worktree removed, branch KEPT.
    fx.deletion
        .delete_task(&fx.project_id, &task_id, &DeleteTaskOptions::default())
        .unwrap();
    assert!(
        fx.branch_exists("fartCode/branching"),
        "branch survives by default"
    );

    // deleteBranch=true: the created branch differs from its source → gone.
    let (task_id2, _) = fx.create_provisioned("branching 2", "fartCode/branching-2");
    fx.deletion
        .delete_task(
            &fx.project_id,
            &task_id2,
            &DeleteTaskOptions {
                delete_worktree: true,
                delete_branch: true,
            },
        )
        .unwrap();
    assert!(
        !fx.branch_exists("fartCode/branching-2"),
        "created branch deleted"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 2: sibling tasks keep the shared worktree; the row still goes
// ---------------------------------------------------------------------------

#[test]
fn sibling_task_keeps_worktree_but_row_is_deleted() {
    let fx = Fixture::new();
    let (task_a, worktree) = fx.create_provisioned("first", "fartCode/shared");

    // Second task reuses task A's workspace row (reference dedupe onto one
    // workspace per resolved path).
    let ws_id: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT workspace_id FROM tasks WHERE id = ?1",
            [&task_a],
            |r| r.get(0),
        )
        .unwrap();
    let task_b = fx
        .tasks
        .create(CreateTaskOptions {
            workspace_target: Some(WorkspaceTarget::RepositoryInstance {
                workspace_id: ws_id.clone(),
            }),
            ..CreateTaskOptions::new(&fx.project_id, "second")
        })
        .unwrap();
    assert_eq!(task_b.workspace_id.as_deref(), Some(ws_id.as_str()));

    fx.deletion
        .delete_task(&fx.project_id, &task_a, &DeleteTaskOptions::default())
        .unwrap();

    // Task row gone; sibling intact.
    assert!(fx.tasks.get(&task_a).unwrap().is_none());
    assert!(fx.tasks.get(&task_b.id).unwrap().is_some());
    // Shared worktree + workspace row kept for the sibling.
    assert!(worktree.exists(), "sibling worktree kept");
    assert_eq!(fx.task_workspace_count(), 1, "shared workspace row kept");
}

// ---------------------------------------------------------------------------
// Acceptance 3: double-delete + delete-during-provision are safe
// ---------------------------------------------------------------------------

#[test]
fn double_delete_is_idempotent() {
    let fx = Fixture::new();
    let (task_id, worktree) = fx.create_provisioned("once", "fartCode/once");

    fx.deletion
        .delete_task(&fx.project_id, &task_id, &DeleteTaskOptions::default())
        .unwrap();
    assert!(!worktree.exists());

    // Second delete: clean no-op, no panic, rows still gone.
    fx.deletion
        .delete_task(&fx.project_id, &task_id, &DeleteTaskOptions::default())
        .unwrap();
    assert_eq!(fx.count("tasks"), 0);
    assert_eq!(fx.task_workspace_count(), 0);
}

#[test]
fn delete_during_provision_is_safe() {
    let fx = Fixture::new();
    // Commit rows WITHOUT provisioning — the workspace row exists but has no
    // path yet (a delete racing the provision step).
    let created = fx
        .creation
        .create(fx.params("racing", "fartCode/racing"))
        .unwrap();
    let ws_id = created.task.workspace_id.clone().unwrap();
    let ws_path: Option<String> = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT path FROM workspaces WHERE id = ?1", [&ws_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(ws_path.is_none(), "unprovisioned workspace has no path");

    fx.deletion
        .delete_task(
            &fx.project_id,
            &created.task.id,
            &DeleteTaskOptions::default(),
        )
        .unwrap();

    assert_eq!(fx.count("tasks"), 0);
    assert_eq!(fx.task_workspace_count(), 0);
}

// ---------------------------------------------------------------------------
// Guards: project-root workspaces outlive every task
// ---------------------------------------------------------------------------

#[test]
fn project_root_workspace_is_never_deleted() {
    let fx = Fixture::new();
    let task = fx
        .tasks
        .create(CreateTaskOptions {
            workspace_target: Some(WorkspaceTarget::ProjectRoot),
            ..CreateTaskOptions::new(&fx.project_id, "root task")
        })
        .unwrap();
    let ws_id = task.workspace_id.clone().unwrap();

    fx.deletion
        .delete_task(&fx.project_id, &task.id, &DeleteTaskOptions::default())
        .unwrap();

    assert!(fx.tasks.get(&task.id).unwrap().is_none(), "task deleted");
    let kind: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT kind FROM workspaces WHERE id = ?1", [&ws_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(kind, "project-root", "project-root workspace survives");
}
