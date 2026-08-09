//! #59: the create-task dialog's workspace/branch mapping — `create_task`
//! turns the dialog strings into the right GitSetup/WorkspaceTarget.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_app_lib::app::App;
use fartcode_app_lib::commands::tasks::create_task_params;
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{LocalProjectGroup, LOCAL_PROJECT};
use fartcode_core::tasks::operations::{GitSetup, TaskConfigParams};
use fartcode_core::tasks::WorkspaceTarget;

fn git_ok(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed in {:?}", args, dir);
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
    _tmp: tempfile::TempDir,
    app: Arc<App>,
    project_id: String,
}

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
    let repo = make_repo(&tmp);
    let project = app.projects.create_local(&repo, false).unwrap();
    Fixture {
        _tmp: tmp,
        app,
        project_id: project.id,
    }
}

fn params(fx: &Fixture, workspace: Option<&str>, branch: Option<&str>) -> GitSetupTarget {
    let p = create_task_params(
        &fx.app,
        &fx.project_id,
        "demo task",
        workspace,
        branch,
        TaskConfigParams {
            name: "demo task".into(),
            initial_status: None,
            linked_issue: None,
            initial_conversation: None,
        },
    )
    .unwrap();
    GitSetupTarget {
        git: p.git,
        workspace: p.workspace,
    }
}

struct GitSetupTarget {
    git: GitSetup,
    workspace: WorkspaceTarget,
}

#[test]
fn default_is_new_worktree_with_fresh_branch() {
    let fx = fixture();
    let p = params(&fx, None, None);
    assert!(matches!(p.git, GitSetup::CreateBranch { .. }));
    assert_eq!(p.workspace, WorkspaceTarget::NewWorktree);
}

#[test]
fn project_root_touches_no_branches() {
    let fx = fixture();
    let p = params(&fx, Some("project-root"), None);
    assert_eq!(p.git, GitSetup::None);
    assert_eq!(p.workspace, WorkspaceTarget::ProjectRoot);
}

#[test]
fn existing_branch_uses_use_branch_in_new_worktree() {
    let fx = fixture();
    let p = params(&fx, Some("new-worktree"), Some("main"));
    assert_eq!(
        p.git,
        GitSetup::UseBranch {
            branch_name: "main".into()
        }
    );
    assert_eq!(p.workspace, WorkspaceTarget::NewWorktree);
}

#[test]
fn invalid_workspace_target_errors() {
    let fx = fixture();
    let err = create_task_params(
        &fx.app,
        &fx.project_id,
        "demo task",
        Some("the-moon"),
        None,
        TaskConfigParams {
            name: "demo task".into(),
            initial_status: None,
            linked_issue: None,
            initial_conversation: None,
        },
    )
    .unwrap_err();
    assert!(err.contains("invalid workspace target"), "got: {err}");
}
