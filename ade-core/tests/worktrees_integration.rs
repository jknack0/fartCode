//! E1-07 — Preserve-pattern file copying into worktrees.
//!
//! The copy logic itself landed with E2-02 (`WorktreeManager::ensure_worktree`
//! → `copy_preserved_files`); these tests pin the E1-07 acceptance criteria:
//! untracked matches appear in new worktrees, tracked files and `.ade.json`
//! never do, and repo `.ade.json` patterns override project defaults.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use ade_core::db::{Db, SqliteDb};
use ade_core::events::BroadcastEventBus;
use ade_core::projects::worktrees::WorktreeManager;
use ade_core::projects::{DbProjectStore, ProjectStore};
use ade_core::settings::{DbSettingsStore, LocalProjectGroup, LOCAL_PROJECT};
use ade_core::tasks::operations::{
    CreateTaskParams, GitSetup, SourceBranchRef, TaskConfigParams, TaskCreationService,
};
use ade_core::tasks::WorkspaceTarget;
use ade_git::CliGit;

struct Fixture {
    _tmp: tempfile::TempDir,
    db: Arc<SqliteDb>,
    service: TaskCreationService,
    project_id: String,
    repo: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self::new_with_repo_prep(|_| {})
    }

    /// Builds the fixture, running `prep` on the repo BEFORE it is added —
    /// a repo that already carries `.ade.json` when added gets the file's
    /// patterns seeded (share-with-team flow); a file written afterwards is
    /// ignored until the local DB value is cleared (reference parity).
    fn new_with_repo_prep(prep: impl FnOnce(&std::path::Path)) -> Self {
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
        let store =
            DbProjectStore::new(db.clone(), settings.clone(), Arc::new(CliGit), bus.clone());

        let repo = make_repo(&tmp, "demo");
        prep(&repo);
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
            service,
            project_id: project.id.clone(),
            repo,
        }
    }

    fn params(&self, branch: &str) -> CreateTaskParams {
        CreateTaskParams {
            id: None,
            project_id: self.project_id.clone(),
            task_config: TaskConfigParams {
                name: "preserve copy".into(),
                initial_status: None,
                linked_issue: None,
                initial_conversation: None,
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

    /// Creates the task + worktree and returns the worktree path.
    fn create_task_with_worktree(&self, branch: &str) -> PathBuf {
        let result = self
            .service
            .create_with_provision(self.params(branch))
            .unwrap();
        let row: (String, String, String, Option<String>) = self
            .db
            .conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT type, kind, path, config FROM workspaces WHERE id = ?1",
                [&result.task.workspace_id.clone().unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.1, "worktree", "expected a worktree workspace");
        PathBuf::from(row.2)
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
            "user.email=t@ade.dev",
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

fn git_commit(repo: &std::path::Path, msg: &str) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@ade.dev",
            "commit",
            "-m",
            msg,
        ])
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git commit failed");
}

/// Default patterns copy untracked matches; tracked files and `.ade.json` never
/// reach the worktree (acceptance: ".env/.env.local/docker-compose.override.yml
/// appear; tracked files and .ade.json never copied").
#[test]
fn preserve_copy_defaults_skip_tracked_and_ade_json() {
    let fx = Fixture::new();

    // A tracked file that matches a default pattern: committed as "tracked",
    // then modified in the working tree. The copy must skip it, so the
    // worktree gets the COMMITTED content, not the modified one.
    std::fs::write(fx.repo.join(".env"), "TRACKED=1\n").unwrap();
    git_ok(&fx.repo, ["add", ".env"]);
    git_commit(&fx.repo, "track .env");
    std::fs::write(fx.repo.join(".env"), "LOCAL=modified\n").unwrap();

    // Untracked matches + the share file itself.
    std::fs::write(fx.repo.join(".env.local"), "LOCAL_ENV=1\n").unwrap();
    std::fs::write(
        fx.repo.join("docker-compose.override.yml"),
        "version: '3'\n",
    )
    .unwrap();
    std::fs::write(
        fx.repo.join(".ade.json"),
        r#"{"preservePatterns": [".env", ".env.local"]}"#,
    )
    .unwrap();

    let wt = fx.create_task_with_worktree("ade/preserve-1");

    // Untracked matches appear.
    assert!(wt.join(".env.local").exists(), ".env.local copied");
    assert!(
        wt.join("docker-compose.override.yml").exists(),
        "docker-compose.override.yml copied (default pattern)"
    );
    // Tracked file came from git, not the copy.
    assert_eq!(
        std::fs::read_to_string(wt.join(".env")).unwrap(),
        "TRACKED=1\n",
        "tracked .env must not be overwritten by the working-tree copy"
    );
    // The share file itself never lands in the worktree.
    assert!(!wt.join(".ade.json").exists(), ".ade.json never copied");
}

/// Repo `.ade.json` patterns override project defaults (acceptance: "patterns
/// from task `.ade.json` override project defaults"). The file must exist when
/// the repo is added — that is exactly the share-with-team flow (the file is
/// committed and cloned down).
#[test]
fn preserve_copy_ade_json_patterns_override_defaults() {
    let fx = Fixture::new_with_repo_prep(|repo| {
        // Share file replaces the default pattern set entirely.
        std::fs::write(
            repo.join(".ade.json"),
            r#"{"preservePatterns": [".env", "custom.env"]}"#,
        )
        .unwrap();
        std::fs::write(repo.join("custom.env"), "CUSTOM=1\n").unwrap();
        std::fs::write(repo.join(".env.local"), "SHOULD_NOT_COPY=1\n").unwrap();
    });

    let wt = fx.create_task_with_worktree("ade/preserve-2");

    assert!(
        wt.join("custom.env").exists(),
        "custom.env copied (override)"
    );
    assert!(
        !wt.join(".env.local").exists(),
        ".env.local must NOT be copied once .ade.json overrides the defaults"
    );
    assert!(
        !wt.join("docker-compose.override.yml").exists(),
        "default pattern dropped by the override"
    );
}

/// Cross-check the workspace row the fixture uses is a real worktree (kind
/// 'worktree'), so the two tests above exercise the actual copy path.
#[test]
fn fixture_provisions_a_worktree() {
    let fx = Fixture::new();
    let wt = fx.create_task_with_worktree("ade/preserve-3");
    assert!(wt.join("README.md").exists(), "branch checked out");
    let kind: String = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT kind FROM workspaces WHERE id = (SELECT workspace_id FROM tasks LIMIT 1)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "worktree");
}
