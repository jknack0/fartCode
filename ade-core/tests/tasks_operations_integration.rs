//! E2-04 Add Task flow integration tests (acceptance criteria).
//!
//! All tests use `tempfile::tempdir()` + real local git repos — never the
//! real app data path and never `$HOME`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use ade_core::db::{Db, SqliteDb};
use ade_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use ade_core::projects::worktrees::{WorkspaceKind, WorktreeManager};
use ade_core::projects::{DbProjectStore, ProjectStore};
use ade_core::settings::{DbSettingsStore, LocalProjectGroup, TaskGroup, LOCAL_PROJECT, TASKS};
use ade_core::tasks::operations::{
    CreateTaskParams, GitSetup, InitialConversationConfig, SourceBranchRef, TaskConfigParams,
    TaskCreationService,
};
use ade_core::tasks::{TaskType, WorkspaceTarget};
use ade_git::{CliGit, GitOps};

struct Fixture {
    _tmp: tempfile::TempDir,
    db: Arc<SqliteDb>,
    settings: Arc<DbSettingsStore>,
    bus: Arc<BroadcastEventBus>,
    git: CliGit,
    service: TaskCreationService,
    project: ade_core::projects::model::Project,
    worktrees_dir: PathBuf,
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
        let git = CliGit;
        let store =
            DbProjectStore::new(db.clone(), settings.clone(), Arc::new(CliGit), bus.clone());

        let repo = make_repo(&tmp, "demo");
        let project = store.create_local(&repo, false).unwrap();

        let worktrees = WorktreeManager::new(db.clone(), settings.clone(), Arc::new(CliGit));
        let service = TaskCreationService::new(
            db.clone(),
            settings.clone(),
            Arc::new(CliGit),
            worktrees,
            bus.clone(),
        );

        Self {
            _tmp: tmp,
            db,
            settings,
            bus,
            git,
            service,
            project,
            worktrees_dir,
        }
    }

    fn params(&self) -> CreateTaskParams {
        CreateTaskParams {
            id: None,
            project_id: self.project.id.clone(),
            task_config: TaskConfigParams {
                name: "fix the bug".into(),
                initial_status: None,
                linked_issue: None,
                initial_conversation: None,
            },
            git: GitSetup::CreateBranch {
                branch_name: "ade/fix-the-bug-ab12c".into(),
                from_branch: SourceBranchRef::local("main"),
                push_branch: false,
            },
            workspace: WorkspaceTarget::NewWorktree,
            automation_run_id: None,
        }
    }

    fn workspace_row(&self, id: &str) -> (String, String, String, Option<String>) {
        // (type, kind, location, config)
        self.db
            .conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT type, kind, location, config FROM workspaces WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .unwrap()
    }

    fn conversation_config(&self, id: &str) -> Option<serde_json::Value> {
        self.db
            .conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT config FROM conversations WHERE id = ?1",
                [id],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
            .and_then(|c| serde_json::from_str(&c).ok())
    }
}

fn make_repo(tmp: &tempfile::TempDir, name: &str) -> PathBuf {
    let repo = tmp.path().join(name);
    std::fs::create_dir_all(&repo).unwrap();
    Command::new("git")
        .args(["init", "-q"])
        .arg(&repo)
        .status()
        .unwrap();
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
    std::fs::canonicalize(&repo).unwrap()
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

// -- acceptance 1: new task provisions a worktree, persists rows, and fires
// the agent start placeholder with the initial prompt ------------------------

#[test]
fn create_with_new_worktree_provisions_rows_worktree_and_events() {
    let fx = Fixture::new();

    let mut params = fx.params();
    params.task_config.initial_conversation = Some(InitialConversationConfig {
        id: "conv-1".into(),
        provider: Some("claude".into()),
        title: "Fix the bug".into(),
        r#type: Some("pty".into()),
        auto_approve: None,
        initial_prompt: Some("Fix the bug in main.rs".into()),
        model: Some("sonnet".into()),
        initial_queue: None,
    });
    params.id = Some("task-1".into());

    // Subscribe BEFORE create so the post-commit events are observed.
    let mut events = fx.bus.subscribe();
    let result = fx.service.create_with_provision(params).unwrap();

    // Task row: workspace linked, created_by 'user', type 'task'.
    let task = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT created_by, type, workspace_id FROM tasks WHERE id = 'task-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(task.0, "user");
    assert_eq!(task.1, TaskType::Task.as_str());
    let ws_id = task.2;

    // Workspace row: worktree kind, local, config stored.
    let (wtype, kind, location, config) = fx.workspace_row(&ws_id);
    assert_eq!(wtype, "local");
    assert_eq!(kind, WorkspaceKind::Worktree.as_str());
    assert_eq!(location, "local");
    let config = serde_json::from_str::<serde_json::Value>(&config.unwrap()).unwrap();
    assert_eq!(config["version"], "2");
    assert_eq!(config["git"]["kind"], "create-branch");
    assert_eq!(config["workspace"]["kind"], "new-worktree");

    // Worktree provisioned on disk with the branch checked out.
    let wt_path = fx.worktrees_dir.join("demo").join("ade/fix-the-bug-ab12c");
    assert!(wt_path.join("README.md").exists(), "worktree checked out");
    assert_eq!(
        fx.git.current_branch(&wt_path).unwrap().as_deref(),
        Some("ade/fix-the-bug-ab12c")
    );
    assert_eq!(result.warning, None);

    // Conversation row: versioned pty config with model + trimmed prompt.
    let conv_cfg = fx.conversation_config("conv-1").unwrap();
    assert_eq!(conv_cfg["version"], "1");
    assert_eq!(conv_cfg["type"], "pty");
    assert_eq!(conv_cfg["model"], "sonnet");
    assert_eq!(conv_cfg["initialPrompt"], "Fix the bug in main.rs");

    // Events: created, conversation created, provisioned, agent start.
    let mut seen = [false; 4];
    while let Ok(ev) = events.try_recv() {
        match ev {
            InternalEvent::TaskCreated { id, .. } if id == "task-1" => seen[0] = true,
            InternalEvent::ConversationCreated { id, .. } if id == "conv-1" => seen[1] = true,
            InternalEvent::TaskProvisioned { id, .. } if id == "task-1" => seen[2] = true,
            InternalEvent::AgentStart {
                conversation_id, ..
            } if conversation_id == "conv-1" => seen[3] = true,
            _ => {}
        }
    }
    assert!(seen.iter().all(|s| *s), "expected events: {seen:?}");
}

// -- acceptance 3: cancel/validation returns typed errors, never panics ------

#[test]
fn create_with_missing_project_returns_typed_error() {
    let fx = Fixture::new();
    let mut params = fx.params();
    params.project_id = "no-such-project".into();

    let err = fx.service.create(params).unwrap_err();
    assert!(err.to_string().contains("project not found"));
    // No rows leaked.
    let count: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn create_with_invalid_inputs_returns_typed_error() {
    let fx = Fixture::new();

    // Empty name.
    let mut params = fx.params();
    params.task_config.name = "  ".into();
    let err = fx.service.create(params).unwrap_err();
    assert!(err.to_string().contains("invalid task input"));

    // Invalid conversation type.
    let mut params = fx.params();
    params.task_config.initial_conversation = Some(InitialConversationConfig {
        id: "c".into(),
        provider: Some("claude".into()),
        title: "t".into(),
        r#type: Some("xterm".into()),
        auto_approve: None,
        initial_prompt: None,
        model: None,
        initial_queue: None,
    });
    let err = fx.service.create(params).unwrap_err();
    assert!(err.to_string().contains("invalid task input"));

    // repository-instance pointing at a missing workspace row.
    let mut params = fx.params();
    params.workspace = WorkspaceTarget::RepositoryInstance {
        workspace_id: "no-ws".into(),
    };
    let err = fx.service.create(params).unwrap_err();
    assert!(err.to_string().contains("invalid task input"));
}

// -- workspace targets -------------------------------------------------------

#[test]
fn create_repository_instance_reuses_workspace_row() {
    let fx = Fixture::new();
    // Seed a workspace row (e.g. the repository workspace).
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO workspaces (id, type, kind, location) VALUES ('repo-ws', 'local', 'project-root', 'local')",
            [],
        )
        .unwrap();

    let mut params = fx.params();
    params.workspace = WorkspaceTarget::RepositoryInstance {
        workspace_id: "repo-ws".into(),
    };
    let before: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
        .unwrap();
    let result = fx.service.create(params).unwrap();
    assert_eq!(result.task.workspace_id.as_deref(), Some("repo-ws"));

    let after: i64 = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        after, before,
        "no new workspace row for repository-instance"
    );

    // provision() on a reused repo workspace (kind 'project-root', NO config)
    // must be a no-op — the old code failed with "no versioned config".
    let provisioned = fx.service.provision(&result.task.id).unwrap();
    assert_eq!(provisioned.workspace_id, "repo-ws");
    assert!(
        provisioned.path.is_none(),
        "reuse must not create a worktree"
    );
    assert!(provisioned.warning.is_none());
}

#[test]
fn create_project_root_runs_in_project_root_with_warning() {
    let fx = Fixture::new();
    let mut params = fx.params();
    params.workspace = WorkspaceTarget::ProjectRoot;

    let result = fx.service.create_with_provision(params).unwrap();
    assert!(result.warning.is_some(), "isolation warning expected");
    assert!(
        result.warning.as_deref().unwrap().contains("project root"),
        "warning: {:?}",
        result.warning
    );

    let ws_id = result.task.workspace_id.as_deref().unwrap();
    let (_t, kind, _l, _c) = fx.workspace_row(ws_id);
    assert_eq!(kind, WorkspaceKind::ProjectRoot.as_str());

    // No worktree directory created for the branch.
    let wt_path = fx.worktrees_dir.join("demo").join("ade/fix-the-bug-ab12c");
    assert!(!wt_path.exists());
}

#[test]
fn create_byoi_stores_remote_workspace_row() {
    let fx = Fixture::new();
    let mut params = fx.params();
    params.workspace = WorkspaceTarget::Byoi {
        remote_workspace_id: Some("host-1".into()),
    };

    let result = fx.service.create_with_provision(params).unwrap();
    let ws_id = result.task.workspace_id.as_deref().unwrap();
    let (wtype, kind, location, _c) = fx.workspace_row(ws_id);
    assert_eq!(wtype, "byoi");
    assert_eq!(kind, WorkspaceKind::Byoi.as_str());
    assert_eq!(location, "remote");
}

// -- provision is idempotent and restart-safe --------------------------------

#[test]
fn provision_is_idempotent_and_reads_intent_from_config() {
    let fx = Fixture::new();
    let mut params = fx.params();
    params.task_config.name = "provision twice".into();

    // create() only commits rows — no worktree yet.
    let created = fx.service.create(params).unwrap();
    let task_id = created.task.id.clone();
    let wt_path = fx.worktrees_dir.join("demo").join("ade/fix-the-bug-ab12c");
    assert!(!wt_path.exists(), "create() alone must not provision");

    // provision() reads the intent from the workspace config and ensures the
    // worktree (restart-safe: intent is re-derived from the row, not params).
    let first = fx.service.provision(&task_id).unwrap();
    assert!(first
        .path
        .as_deref()
        .unwrap()
        .ends_with("ade/fix-the-bug-ab12c"));
    assert!(wt_path.exists());

    // Idempotent: re-provision reuses the worktree, no duplicate.
    let second = fx.service.provision(&task_id).unwrap();
    assert!(second.path.is_some());
    assert!(
        fx.git.worktree_list(&fx.project.path).unwrap().len() == 2,
        "no duplicate worktrees after re-provision"
    );
}

// -- use-branch: existing branch, no branch creation -------------------------

#[test]
fn create_use_branch_checks_out_existing_branch() {
    let fx = Fixture::new();
    // Create the branch up front.
    let repo = &fx.project.path;
    fx.git
        .branch_create(repo, "ade/existing-fix", "main")
        .unwrap();

    let mut params = fx.params();
    params.git = GitSetup::UseBranch {
        branch_name: "ade/existing-fix".into(),
    };
    params.task_config.name = "reuse branch".into();

    let result = fx.service.create_with_provision(params).unwrap();
    let wt_path = fx.worktrees_dir.join("demo").join("ade/existing-fix");
    assert!(wt_path.exists(), "worktree on the existing branch");
    assert_eq!(
        fx.git.current_branch(&wt_path).unwrap().as_deref(),
        Some("ade/existing-fix")
    );
    assert!(result.warning.is_none());
}

// -- trust + branch listing --------------------------------------------------

#[test]
fn trust_decision_follows_setting_and_force() {
    let fx = Fixture::new();
    // Default: autoTrustWorktrees = true.
    assert!(fx.service.should_auto_trust(false).unwrap());
    // Forced (autoApprove) always trusts.
    assert!(fx.service.should_auto_trust(true).unwrap());

    // Setting off → no auto-trust; force still trusts.
    fx.settings
        .set(
            &TASKS,
            TaskGroup {
                auto_trust_worktrees: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!fx.service.should_auto_trust(false).unwrap());
    assert!(fx.service.should_auto_trust(true).unwrap());
}

#[test]
fn list_branches_returns_repo_refs_for_picker() {
    let fx = Fixture::new();
    fx.git
        .branch_create(&fx.project.path, "feature/x", "main")
        .unwrap();
    let branches = fx.service.list_branches(&fx.project.id).unwrap();
    assert!(
        branches.iter().any(|b| b.name == "main"),
        "branches: {branches:?}"
    );
    assert!(
        branches.iter().any(|b| b.name == "feature/x"),
        "branches: {branches:?}"
    );
}
