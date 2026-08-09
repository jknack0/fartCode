//! E19-01 (#70; ADR-0038 items 1–2) feature dossiers: the file is born
//! with the worktree at the card's first `agent_step` entry, carries a
//! backfilled header, and thereafter collects machine breadcrumbs under
//! `## Timeline` without ever touching the agent-written sections below it.
//!
//! These are real provisioning runs (git init, `worktree add`) inside a
//! tempdir — the dossier's whole premise is that it lands INSIDE the
//! worktree, which a mocked filesystem could not prove.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_app_lib::app::App;
use fartcode_app_lib::dispatch::issue_dispatch_core;
use fartcode_app_lib::dossiers::TimelineAppender;
use fartcode_app_lib::step_engine;
use fartcode_core::dossiers;
use fartcode_core::events::{EventBus, InternalEvent};
use fartcode_core::issues::columns::{ColumnKind, ColumnStore, NewColumn, OnEnter, OnSettle};
use fartcode_core::issues::{Issue, NewIssue};
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{LocalProjectGroup, LOCAL_PROJECT};
use fartcode_core::tasks::TaskStore;

fn git_ok(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed in {dir:?}");
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

impl Fixture {
    fn new_issue(&self, title: &str) -> Issue {
        self.app
            .issues
            .create(NewIssue {
                project_id: self.project_id.clone(),
                title: title.into(),
                body: Some("the body".into()),
                acceptance: vec!["it works".into()],
                lane: None,
                provider: None,
                model: None,
                prd_path: Some("docs/prds/oauth.md".into()),
                prd_section: Some("## Flow".into()),
                external_ref: None,
                dossier_path: None,
            })
            .unwrap()
    }

    fn column(&self, name: &str) -> fartcode_core::issues::columns::BoardColumn {
        self.app
            .columns
            .list_for_project(&self.project_id)
            .unwrap()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no column named {name}"))
    }

    /// A second `agent_step` column, so "the FIRST step entry births the
    /// dossier" can be distinguished from "every step entry does".
    fn extra_step(&self, name: &str) -> fartcode_core::issues::columns::BoardColumn {
        ColumnStore::new(self.app.db.clone())
            .create(NewColumn {
                project_id: self.project_id.clone(),
                name: name.into(),
                kind: ColumnKind::AgentStep,
                counts_as_done: false,
                is_landing: false,
                on_enter: Some(OnEnter::Run),
                on_settle: Some(OnSettle::Hold),
                advance_to: None,
                step_prompt: None,
                step_provider: None,
                step_model: None,
                step_effort: None,
                step_tools: None,
            })
            .unwrap()
    }

    fn set_consent(&self, value: Option<bool>) {
        let project = self.app.projects.get(&self.project_id).unwrap().unwrap();
        let repo = PathBuf::from(&project.path);
        let mut settings = self
            .app
            .settings
            .get_project_settings(&self.project_id, &repo)
            .unwrap();
        settings.feature_dossiers = value;
        self.app
            .settings
            .update_project_settings(&self.project_id, &repo, &settings)
            .unwrap();
    }

    fn worktree_of(&self, task_id: &str) -> PathBuf {
        let task = self.app.tasks.get(task_id).unwrap().unwrap();
        let workspace_id = task.workspace_id.expect("provisioned task has a workspace");
        let path: String = self
            .app
            .db
            .conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT path FROM workspaces WHERE id = ?1",
                [&workspace_id],
                |row| row.get(0),
            )
            .unwrap();
        PathBuf::from(path)
    }

    fn dossier_text(&self, issue_id: &str) -> String {
        let issue = self.app.issues.get(issue_id).unwrap().unwrap();
        let rel = issue.dossier_path.expect("card has a dossier");
        let task_id = issue.linked_task_id.expect("card has a task");
        std::fs::read_to_string(self.worktree_of(&task_id).join(rel)).unwrap()
    }
}

// ---------------------------------------------------------------------------
// 1. Birth with the worktree
// ---------------------------------------------------------------------------

#[test]
fn first_agent_step_entry_writes_the_dossier_inside_the_worktree() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    let in_progress = fx.column("In Progress");

    let outcome = step_engine::enter_column(&fx.app, &issue.id, &in_progress.id, None).unwrap();
    let launch = outcome.launch.expect("run-mode step launches");

    let stored = fx.app.issues.get(&issue.id).unwrap().unwrap();
    assert_eq!(
        stored.dossier_path.as_deref(),
        Some("docs/features/implement-oauth-login.md"),
        "dossier_path records the repo-relative path"
    );

    // INSIDE the worktree, not the main checkout.
    let worktree = fx.worktree_of(&launch.task.id);
    let project = fx.app.projects.get(&fx.project_id).unwrap().unwrap();
    assert_ne!(worktree, PathBuf::from(&project.path));
    assert!(worktree
        .join("docs/features/implement-oauth-login.md")
        .is_file());
    assert!(
        !Path::new(&project.path)
            .join("docs/features/implement-oauth-login.md")
            .exists(),
        "the main checkout is never written to"
    );

    // The header is backfilled from what the app already held.
    let text = fx.dossier_text(&issue.id);
    assert!(text.starts_with("# Implement OAuth login\n"));
    assert!(text.contains("the body"), "issue body");
    assert!(text.contains("- it works"), "acceptance criteria");
    assert!(
        text.contains("- PRD: `docs/prds/oauth.md` — ## Flow"),
        "PRD link + section"
    );
    assert!(
        text.contains("- source: proposal · docs/prds/oauth.md"),
        "provenance"
    );
    assert!(text.contains("## Timeline"));
    assert!(
        text.contains("· created · proposal"),
        "pre-worktree history"
    );
    assert!(text.contains("dossier created with the worktree · In Progress"));
}

#[test]
fn a_second_step_column_reuses_the_dossier_and_writes_no_second_file() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    let review_step = fx.extra_step("Review");

    step_engine::enter_column(&fx.app, &issue.id, &fx.column("In Progress").id, None).unwrap();
    let first = fx.app.issues.get(&issue.id).unwrap().unwrap();
    let task_id = first.linked_task_id.clone().unwrap();

    // Second agent_step entry: same task, same worktree, same dossier.
    step_engine::enter_column(&fx.app, &issue.id, &review_step.id, None).unwrap();
    let second = fx.app.issues.get(&issue.id).unwrap().unwrap();
    assert_eq!(second.linked_task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(second.dossier_path, first.dossier_path);

    let dir = fx.worktree_of(&task_id).join("docs/features");
    let files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(files.len(), 1, "exactly one dossier, got {files:?}");
}

#[test]
fn a_failed_write_leaves_dispatch_succeeding_with_a_null_dossier_path() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");

    // Make `docs/` a read-only FILE in the worktree source: `create_dir_all`
    // then fails inside every worktree cut from this branch, which is the
    // realistic shape of "the repo would not accept the write".
    let project = fx.app.projects.get(&fx.project_id).unwrap().unwrap();
    std::fs::write(Path::new(&project.path).join("docs"), "not a directory\n").unwrap();
    git_ok(Path::new(&project.path), &["add", "docs"]);
    git_ok(
        Path::new(&project.path),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@fartCode.dev",
            "commit",
            "-m",
            "docs is a file",
        ],
    );

    let outcome = issue_dispatch_core(&fx.app, &issue.id).expect("dispatch still succeeds");
    assert!(!outcome.reattached);
    let stored = fx.app.issues.get(&issue.id).unwrap().unwrap();
    assert_eq!(
        stored.dossier_path, None,
        "a write failure leaves the path NULL"
    );
    // And the actual work of the dispatch happened anyway.
    assert_eq!(
        stored.linked_task_id.as_deref(),
        Some(outcome.task.id.as_str())
    );
}

// ---------------------------------------------------------------------------
// 2. Consent gate (ADR-0038 item 3; the card itself is #74)
// ---------------------------------------------------------------------------

#[test]
fn consent_off_writes_nothing_and_still_dispatches() {
    let fx = fixture();
    fx.set_consent(Some(false));
    let issue = fx.new_issue("Implement OAuth login");

    let outcome = issue_dispatch_core(&fx.app, &issue.id).expect("declining still dispatches");
    let stored = fx.app.issues.get(&issue.id).unwrap().unwrap();
    assert_eq!(stored.dossier_path, None);
    assert!(!fx
        .worktree_of(&outcome.task.id)
        .join("docs/features")
        .exists());
}

#[test]
fn consent_unset_writes_the_interim_default() {
    let fx = fixture();
    fx.set_consent(None);
    let issue = fx.new_issue("Implement OAuth login");

    issue_dispatch_core(&fx.app, &issue.id).unwrap();
    let stored = fx.app.issues.get(&issue.id).unwrap().unwrap();
    assert_eq!(
        stored.dossier_path.as_deref(),
        Some("docs/features/implement-oauth-login.md"),
        "unset (not yet asked) writes until #74 lands the consent card"
    );
}

#[test]
fn consent_on_writes() {
    let fx = fixture();
    fx.set_consent(Some(true));
    let issue = fx.new_issue("Implement OAuth login");

    issue_dispatch_core(&fx.app, &issue.id).unwrap();
    let text = fx.dossier_text(&issue.id);
    assert!(text.contains("## Timeline"));
    // The legacy dispatch path provisions BEFORE it moves the card, so
    // the card is still on a shelf: the birth line must not name it as
    // the step column it isn't.
    assert!(
        text.contains("dossier created with the worktree\n"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// 3. Timeline appender
// ---------------------------------------------------------------------------

#[test]
fn step_events_append_one_timeline_line_each() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    let in_progress = fx.column("In Progress");
    step_engine::enter_column(&fx.app, &issue.id, &in_progress.id, None).unwrap();
    let task_id = fx
        .app
        .issues
        .get(&issue.id)
        .unwrap()
        .unwrap()
        .linked_task_id
        .unwrap();

    let appender = TimelineAppender::new(fx.app.clone());
    appender.seed();

    appender.handle(&InternalEvent::StepLaunch {
        issue_id: issue.id.clone(),
        project_id: fx.project_id.clone(),
        column_id: in_progress.id.clone(),
        task_id: task_id.clone(),
        prompt: String::new(),
        provider: "claude".into(),
        model: Some("haiku".into()),
        effort: None,
        reattached: false,
    });
    appender.handle(&InternalEvent::StepSettled {
        issue_id: issue.id.clone(),
        project_id: fx.project_id.clone(),
        column_id: in_progress.id.clone(),
        task_id: task_id.clone(),
    });

    let text = fx.dossier_text(&issue.id);
    assert_eq!(
        text.matches("In Progress · launched · claude · haiku")
            .count(),
        1,
        "one launch line with column, provider and model:\n{text}"
    );
    assert_eq!(text.matches("In Progress · settled").count(), 1);
}

#[test]
fn a_reattach_is_not_a_launch() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    let in_progress = fx.column("In Progress");
    step_engine::enter_column(&fx.app, &issue.id, &in_progress.id, None).unwrap();
    let task_id = fx
        .app
        .issues
        .get(&issue.id)
        .unwrap()
        .unwrap()
        .linked_task_id
        .unwrap();

    let appender = TimelineAppender::new(fx.app.clone());
    appender.handle(&InternalEvent::StepLaunch {
        issue_id: issue.id.clone(),
        project_id: fx.project_id.clone(),
        column_id: in_progress.id.clone(),
        task_id,
        prompt: String::new(),
        provider: "claude".into(),
        model: None,
        effort: None,
        reattached: true,
    });
    assert!(!fx.dossier_text(&issue.id).contains("launched"));
}

#[test]
fn agent_written_sections_survive_an_append_untouched() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    let in_progress = fx.column("In Progress");
    step_engine::enter_column(&fx.app, &issue.id, &in_progress.id, None).unwrap();
    let stored = fx.app.issues.get(&issue.id).unwrap().unwrap();
    let task_id = stored.linked_task_id.clone().unwrap();
    let file = fx
        .worktree_of(&task_id)
        .join(stored.dossier_path.clone().unwrap());

    // An agent adds its section below the Timeline (#71's step prompts).
    const AGENT_SECTION: &str = "\n## Plan — 2026-08-09\n\n\
        Chose token rotation over long-lived sessions.\n\n\
        Rejected: a shared refresh endpoint (fans out failure).\n";
    let before = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, format!("{before}{AGENT_SECTION}")).unwrap();

    let appender = TimelineAppender::new(fx.app.clone());
    appender.handle(&InternalEvent::StepSettled {
        issue_id: issue.id.clone(),
        project_id: fx.project_id.clone(),
        column_id: in_progress.id.clone(),
        task_id,
    });

    let after = std::fs::read_to_string(&file).unwrap();
    assert!(
        after.contains(AGENT_SECTION.trim_start_matches('\n')),
        "the agent section is byte-identical:\n{after}"
    );
    let section_start = after.find("## Plan — 2026-08-09").unwrap();
    assert!(
        after[..section_start].contains("In Progress · settled"),
        "the new line landed UNDER Timeline, above the agent section:\n{after}"
    );
}

#[test]
fn a_column_move_appends_a_move_line() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    step_engine::enter_column(&fx.app, &issue.id, &fx.column("In Progress").id, None).unwrap();

    let appender = TimelineAppender::new(fx.app.clone());
    appender.seed(); // knows the card sits In Progress

    let review = fx.column("In Review");
    fx.app
        .issues
        .enter_column(&issue.id, &review.id, None)
        .unwrap();
    appender.handle(&InternalEvent::IssueUpdated {
        id: issue.id.clone(),
        project_id: fx.project_id.clone(),
    });

    let text = fx.dossier_text(&issue.id);
    assert_eq!(text.matches("column → In Review").count(), 1, "{text}");

    // A non-column update (title edit) fires IssueUpdated too and must add
    // nothing — the appender diffs columns, it does not echo events.
    fx.app
        .issues
        .update(
            &issue.id,
            fartcode_core::issues::IssuePatch {
                title: Some("Implement OAuth login v2".into()),
                ..Default::default()
            },
        )
        .unwrap();
    appender.handle(&InternalEvent::IssueUpdated {
        id: issue.id.clone(),
        project_id: fx.project_id.clone(),
    });
    assert_eq!(
        fx.dossier_text(&issue.id).matches("column → ").count(),
        1,
        "no line for a non-move update"
    );
}

#[test]
fn a_card_whose_worktree_is_gone_appends_nothing() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    let in_progress = fx.column("In Progress");
    step_engine::enter_column(&fx.app, &issue.id, &in_progress.id, None).unwrap();
    let stored = fx.app.issues.get(&issue.id).unwrap().unwrap();
    let task_id = stored.linked_task_id.clone().unwrap();
    let worktree = fx.worktree_of(&task_id);

    // Teardown: the worktree (and the dossier with it) is gone, but the
    // issue row survives — exactly ADR-0038's post-teardown case.
    std::fs::remove_dir_all(&worktree).unwrap();

    let appender = TimelineAppender::new(fx.app.clone());
    appender.handle(&InternalEvent::StepSettled {
        issue_id: issue.id.clone(),
        project_id: fx.project_id.clone(),
        column_id: in_progress.id.clone(),
        task_id,
    });
    assert!(!worktree.exists(), "nothing was recreated");
    // dossier_path is still recorded — the card remembers where it lived.
    assert!(fx
        .app
        .issues
        .get(&issue.id)
        .unwrap()
        .unwrap()
        .dossier_path
        .is_some());
}

#[test]
fn a_card_with_no_dossier_appends_nothing() {
    let fx = fixture();
    fx.set_consent(Some(false));
    let issue = fx.new_issue("Implement OAuth login");
    let in_progress = fx.column("In Progress");
    step_engine::enter_column(&fx.app, &issue.id, &in_progress.id, None).unwrap();
    let task_id = fx
        .app
        .issues
        .get(&issue.id)
        .unwrap()
        .unwrap()
        .linked_task_id
        .unwrap();

    let appender = TimelineAppender::new(fx.app.clone());
    appender.handle(&InternalEvent::StepSettled {
        issue_id: issue.id.clone(),
        project_id: fx.project_id.clone(),
        column_id: in_progress.id.clone(),
        task_id: task_id.clone(),
    });
    assert!(!fx
        .worktree_of(&task_id)
        .join(dossiers::DOSSIER_DIR)
        .exists());
}

#[test]
fn the_bus_subscription_reaches_the_dossier() {
    // The wiring itself, not just `handle`: an event published on the bus
    // must land in the file without the emitter waiting on the write.
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    let in_progress = fx.column("In Progress");
    step_engine::enter_column(&fx.app, &issue.id, &in_progress.id, None).unwrap();
    let task_id = fx
        .app
        .issues
        .get(&issue.id)
        .unwrap()
        .unwrap()
        .linked_task_id
        .unwrap();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let app = fx.app.clone();
    let (issue_id, column_id) = (issue.id.clone(), in_progress.id.clone());
    let project_id = fx.project_id.clone();
    runtime.block_on(async move {
        let appender = Arc::new(TimelineAppender::new(app.clone()));
        let mut rx = app.event_bus.subscribe();
        app.event_bus.send(InternalEvent::StepSettled {
            issue_id,
            project_id,
            column_id,
            task_id,
        });
        let event = rx.recv().await.unwrap();
        tokio::task::spawn_blocking(move || appender.handle(&event))
            .await
            .unwrap();
    });

    assert!(fx.dossier_text(&issue.id).contains("In Progress · settled"));
}
