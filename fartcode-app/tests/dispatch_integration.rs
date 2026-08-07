//! E17-03 (#57) dispatch engine integration: drag-into-In-Progress creates
//! the linked task (worktree + prompt packet + linked_issue local variant),
//! re-dispatch reattaches, task deletion unlinks and re-dispatch spawns
//! fresh, and agent exit auto-flips the card to In Review.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_app_lib::app::App;
use fartcode_app_lib::dispatch::{flip_issues_for_task, issue_dispatch_core};
use fartcode_app_lib::terminals::{TerminalManager, TerminalSpec};
use fartcode_core::issues::{Lane, NewIssue};
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{LocalProjectGroup, LOCAL_PROJECT};
use fartcode_core::tasks::TaskStore;
use fartcode_core::terminals::lifecycle::LifecycleScriptType;
use tauri::Manager as _;

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
    // Worktrees land inside the tempdir, never the real default dir.
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

impl Fixture {
    fn new_issue(&self, title: &str) -> fartcode_core::issues::Issue {
        self.app
            .issues
            .create(NewIssue {
                project_id: self.project_id.clone(),
                title: title.into(),
                body: Some("the body".into()),
                acceptance: vec!["it works".into()],
                lane: Some(Lane::Ready),
                provider: None,
                model: None,
                prd_path: Some("docs/prds/x.md".into()),
                prd_section: None,
                external_ref: None,
            })
            .unwrap()
    }
}

#[test]
fn dispatch_creates_task_worktree_link_and_moves_card() {
    let fx = fixture();
    let issue = fx.new_issue("Implement the thing");

    let outcome = issue_dispatch_core(&fx.app, &issue.id).unwrap();
    assert!(!outcome.reattached);
    assert_eq!(outcome.task.name, "Implement the thing");
    assert_eq!(outcome.issue.lane, Lane::InProgress);
    assert_eq!(
        outcome.issue.linked_task_id.as_deref(),
        Some(outcome.task.id.as_str())
    );
    // The prompt packet carries title, body, AC, and the PRD reference.
    assert!(outcome.prompt.contains("Implement the thing"));
    assert!(outcome.prompt.contains("the body"));
    assert!(outcome.prompt.contains("- it works"));
    assert!(outcome.prompt.contains("PRD: docs/prds/x.md"));
    // The worktree materialized on disk inside the tempdir.
    let task = fx.app.tasks.get(&outcome.task.id).unwrap().unwrap();
    let workspace_id = task.workspace_id.expect("dispatch provisions a worktree");
    let worktree_path: String = fx
        .app
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT path FROM workspaces WHERE id = ?1",
            [workspace_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(Path::new(&worktree_path).is_dir());
    // linked_issue carries the local variant (ADR-0032).
    let li = task.linked_issue.expect("linked_issue set");
    assert_eq!(li.provider, "local");
    assert_eq!(li.identifier, issue.id);
}

#[test]
fn redispatch_reattaches_and_deleted_task_respawns() {
    let fx = fixture();
    let issue = fx.new_issue("Reattach me");

    let first = issue_dispatch_core(&fx.app, &issue.id).unwrap();
    let second = issue_dispatch_core(&fx.app, &issue.id).unwrap();
    assert!(second.reattached);
    assert_eq!(second.task.id, first.task.id);
    assert!(second.prompt.is_empty());

    // Task deletion clears the link (ON DELETE SET NULL) — the card shows
    // unlinked state and the next dispatch spawns a FRESH task.
    let first_task_id = first.task.id.clone();
    fx.app
        .db
        .conn()
        .lock()
        .unwrap()
        .execute("DELETE FROM tasks WHERE id = ?1", [&first_task_id])
        .unwrap();
    let unlinked = fx.app.issues.get(&issue.id).unwrap().unwrap();
    assert!(unlinked.linked_task_id.is_none());

    let third = issue_dispatch_core(&fx.app, &issue.id).unwrap();
    assert!(!third.reattached);
    assert_ne!(third.task.id, first_task_id);
}

#[test]
fn flip_moves_only_in_progress_cards() {
    let fx = fixture();
    let in_progress = fx.new_issue("running");
    let dispatched = issue_dispatch_core(&fx.app, &in_progress.id).unwrap();
    let ready = fx.new_issue("still ready");

    let flipped = flip_issues_for_task(&fx.app, &dispatched.task.id);
    assert_eq!(flipped, 1);
    assert_eq!(
        fx.app.issues.get(&in_progress.id).unwrap().unwrap().lane,
        Lane::InReview
    );
    // The ready card is untouched even though it shares the project.
    assert_eq!(
        fx.app.issues.get(&ready.id).unwrap().unwrap().lane,
        Lane::Ready
    );
    // Idempotent: a second exit signal flips nothing.
    assert_eq!(flip_issues_for_task(&fx.app, &dispatched.task.id), 0);
}

#[test]
fn agent_terminal_exit_flips_the_card() {
    let fx = fixture();
    let issue = fx.new_issue("pump hook");
    let dispatched = issue_dispatch_core(&fx.app, &issue.id).unwrap();

    let tapp = tauri::test::mock_app();
    tapp.handle().manage(fx.app.clone());
    let manager = TerminalManager::new(tapp.handle().clone());

    let cwd = std::env::temp_dir();
    // A plain shell exit must NOT flip (only agent terminals).
    let plain = manager
        .open(TerminalSpec {
            task_id: &dispatched.task.id,
            project_id: &fx.project_id,
            agent: None,
            tmux: false,
            program: "/bin/sh",
            args: &["-c".into(), "exit 0".into()],
            env: &[],
            cwd: &cwd,
            rows: 24,
            cols: 80,
            lifecycle: None::<LifecycleScriptType>,
        })
        .unwrap();
    let _ = plain;
    // An agent terminal exiting flips the linked card.
    manager
        .open(TerminalSpec {
            task_id: &dispatched.task.id,
            project_id: &fx.project_id,
            agent: Some("claude"),
            tmux: false,
            program: "/bin/sh",
            args: &["-c".into(), "exit 0".into()],
            env: &[],
            cwd: &cwd,
            rows: 24,
            cols: 80,
            lifecycle: None::<LifecycleScriptType>,
        })
        .unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let lane = fx.app.issues.get(&issue.id).unwrap().unwrap().lane;
        if lane == Lane::InReview {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "card never flipped");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
