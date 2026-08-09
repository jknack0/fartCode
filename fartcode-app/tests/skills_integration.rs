//! E19-02 (#71; ADR-0038 items 2–3): the seeded feature-log skill, the
//! `AGENTS.md` pointer, and the step-prompt append instruction.
//!
//! The unit-level scaffold surgery (idempotence, never-clobber, version
//! bumps) is proved in `fartcode_core::skills`. What can only be proved
//! here is the part that needs a wired App and a real worktree: the consent
//! gate on every write, and that the files land INSIDE the worktree — on
//! the feature branch, in the user's pull request — rather than in the
//! checkout they are standing in.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_app_lib::app::App;
use fartcode_app_lib::skills as skills_app;
use fartcode_app_lib::step_engine;
use fartcode_core::issues::{Issue, IssuePatch, NewIssue};
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{LocalProjectGroup, LOCAL_PROJECT};
use fartcode_core::skills;
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
    repo: PathBuf,
}

/// A project in its REAL default state: consent never asked, so nothing
/// may be written (`feature_dossiers: None` fails closed).
fn fixture_unasked() -> Fixture {
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
        repo,
    }
}

/// A project that has consented.
fn fixture() -> Fixture {
    let fx = fixture_unasked();
    fx.set_consent(Some(true));
    fx
}

impl Fixture {
    fn set_consent(&self, value: Option<bool>) {
        let mut settings = self
            .app
            .settings
            .get_project_settings(&self.project_id, &self.repo)
            .unwrap();
        settings.feature_dossiers = value;
        self.app
            .settings
            .update_project_settings(&self.project_id, &self.repo, &settings)
            .unwrap();
    }

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
                prd_path: None,
                prd_section: None,
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
}

fn seeded(dir: &Path) -> bool {
    dir.join(skills::SKILL_FILE).is_file() || dir.join(skills::AGENTS_FILE).exists()
}

// ---------------------------------------------------------------------------
// 1. Consent gates every write
// ---------------------------------------------------------------------------

/// The state of every project until #74's consent card ships. Unasked
/// fails closed: not one byte lands in the repo.
#[test]
fn consent_unasked_seeds_nothing() {
    let fx = fixture_unasked();
    let target = fx._tmp.path().join("wt-unasked");
    std::fs::create_dir_all(&target).unwrap();

    skills_app::seed_for_worktree(&fx.app, &fx.project_id, &target);
    assert!(!seeded(&target), "an unasked project is never written to");
}

#[test]
fn consent_declined_seeds_nothing() {
    let fx = fixture();
    fx.set_consent(Some(false));
    let target = fx._tmp.path().join("wt-declined");
    std::fs::create_dir_all(&target).unwrap();

    skills_app::seed_for_worktree(&fx.app, &fx.project_id, &target);
    assert!(!seeded(&target), "a declined project is never written to");
}

#[test]
fn consent_granted_seeds_a_provenance_tagged_scaffold() {
    let fx = fixture();
    let target = fx._tmp.path().join("wt-yes");
    std::fs::create_dir_all(&target).unwrap();

    skills_app::seed_for_worktree(&fx.app, &fx.project_id, &target);

    let skill = std::fs::read_to_string(target.join(skills::SKILL_FILE)).unwrap();
    assert!(skill.contains("written by fartCode (ADR-0038)"));
    assert!(skill.contains(&format!(
        "{}{}",
        skills::SKILL_MARKER,
        skills::FEATURE_LOG_VERSION
    )));
    let agents = std::fs::read_to_string(target.join(skills::AGENTS_FILE)).unwrap();
    assert!(agents.contains("written by fartCode (ADR-0038)"));
    assert_eq!(
        agents
            .lines()
            .filter(|l| l.contains(skills::POINTER_MARKER))
            .count(),
        1
    );
}

/// Consent withdrawn between two dispatches: the scaffold that already
/// landed stays (it is on a branch, deleting it would be a second
/// unrequested write), but nothing new is written — including the reseed a
/// version bump would otherwise perform.
#[test]
fn withdrawn_consent_stops_reseeding() {
    let fx = fixture();
    let target = fx._tmp.path().join("wt-withdrawn");
    std::fs::create_dir_all(&target).unwrap();
    skills_app::seed_for_worktree(&fx.app, &fx.project_id, &target);
    let before = std::fs::read_to_string(target.join(skills::AGENTS_FILE)).unwrap();

    fx.set_consent(Some(false));
    skills_app::seed_for_worktree(&fx.app, &fx.project_id, &target);
    assert_eq!(
        std::fs::read_to_string(target.join(skills::AGENTS_FILE)).unwrap(),
        before
    );
}

// ---------------------------------------------------------------------------
// 2. The scaffold lands in the worktree, on the feature branch
// ---------------------------------------------------------------------------

#[test]
fn a_dispatched_step_seeds_the_convention_inside_the_worktree() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");

    let outcome =
        step_engine::enter_column(&fx.app, &issue.id, &fx.column("In Progress").id, None).unwrap();
    let launch = outcome.launch.expect("run-mode step launches");
    let worktree = fx.worktree_of(&launch.task.id);

    assert!(
        worktree.join(skills::SKILL_FILE).is_file(),
        "the skill rides the feature branch"
    );
    assert!(worktree.join(skills::AGENTS_FILE).is_file());
    // The user's checkout is never touched — a write there would be a
    // silent mutation of the tree they are standing in.
    assert!(
        !fx.repo.join(skills::SKILL_DIR).exists(),
        "the main checkout is left alone"
    );
    assert!(!fx.repo.join(skills::AGENTS_FILE).exists());
}

/// A repo that already has a hand-written `AGENTS.md` keeps it whole and
/// gains one line — through the real dispatch path, not a direct call.
#[test]
fn a_hand_written_agents_file_in_the_repo_survives_dispatch() {
    let fx = fixture();
    const HAND_WRITTEN: &str = "# AGENTS.md\n\n## Build\n\nRun `make`.\n";
    std::fs::write(fx.repo.join("AGENTS.md"), HAND_WRITTEN).unwrap();
    git_ok(&fx.repo, &["add", "AGENTS.md"]);
    git_ok(
        &fx.repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@fartCode.dev",
            "commit",
            "-m",
            "agents",
        ],
    );

    let outcome = step_engine::enter_column(
        &fx.app,
        &fx.new_issue("Dark mode").id,
        &fx.column("In Progress").id,
        None,
    )
    .unwrap();
    let worktree = fx.worktree_of(&outcome.launch.expect("launch").task.id);

    let after = std::fs::read_to_string(worktree.join("AGENTS.md")).unwrap();
    assert!(after.starts_with(HAND_WRITTEN), "every byte kept:\n{after}");
    assert_eq!(
        after
            .lines()
            .filter(|l| l.contains(skills::POINTER_MARKER))
            .count(),
        1
    );
    // The committed copy in the user's checkout is untouched.
    assert_eq!(
        std::fs::read_to_string(fx.repo.join("AGENTS.md")).unwrap(),
        HAND_WRITTEN
    );
}

// ---------------------------------------------------------------------------
// 3. The prompt half
// ---------------------------------------------------------------------------

/// The strongest form of the ADR-0038 item 2 claim: the prompt an agent
/// actually receives ends with the append instruction, naming the column it
/// is running as.
#[test]
fn the_step_prompt_ends_with_the_append_instruction_and_names_the_column() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");

    let outcome =
        step_engine::enter_column(&fx.app, &issue.id, &fx.column("In Progress").id, None).unwrap();
    let prompt = outcome.launch.expect("launch").prompt;

    assert!(prompt.contains("# Feature log"), "{prompt}");
    assert!(
        prompt.contains("## In Progress — <YYYY-MM-DD>"),
        "names the actual column:\n{prompt}"
    );
    assert!(
        prompt.contains("docs/features/implement-oauth-login.md"),
        "names the card's real dossier:\n{prompt}"
    );
    assert!(prompt.contains("Tradeoffs"));
    assert!(prompt.contains("Rejected"));
    // A skipped append is never a failure (ADR-0038 item 2).
    assert!(prompt.contains("skip it"), "{prompt}");
    // Ends with it — the reference packet still comes first.
    assert!(prompt.find("# Issue").unwrap() < prompt.find("# Feature log").unwrap());
    assert!(prompt
        .trim_end()
        .ends_with(&format!("`{}`.)", skills::SKILL_FILE)));
}

/// The inverse, and the one that matters most: an agent in a project that
/// declined is never told to write a dossier. It would create exactly the
/// file the app refused to create.
#[test]
fn a_declined_project_gets_no_append_instruction() {
    let fx = fixture_unasked();
    let issue = fx.new_issue("Implement OAuth login");

    let outcome =
        step_engine::enter_column(&fx.app, &issue.id, &fx.column("In Progress").id, None).unwrap();
    let prompt = outcome.launch.expect("launch").prompt;

    assert!(!prompt.contains("# Feature log"), "{prompt}");
    assert!(!prompt.contains("docs/features/"), "{prompt}");
    // …and the packet is otherwise unchanged.
    assert!(prompt.contains("# Issue"));
    assert!(prompt.contains("Implement OAuth login"));
}

/// Consent ON but no dossier on the card (creation refused, or the slug was
/// occupied by a human's document): no instruction. Naming a path the app
/// itself declined to write is how an agent clobbers that document.
#[test]
fn a_card_without_a_dossier_gets_no_instruction() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    assert!(issue.dossier_path.is_none());

    let prompt =
        skills_app::with_append_instruction(&fx.app, &issue, "In Progress", "PACKET".to_string());
    assert_eq!(prompt, "PACKET");
}

/// Revoking consent stops the instruction on the very next launch — it is
/// re-read per prompt, not captured when the dossier was created.
#[test]
fn revoking_consent_stops_the_instruction_on_a_card_that_has_a_dossier() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");
    let issue = fx
        .app
        .issues
        .update(
            &issue.id,
            IssuePatch {
                dossier_path: Some(Some("docs/features/implement-oauth-login.md".into())),
                ..Default::default()
            },
        )
        .unwrap();

    assert!(
        skills_app::with_append_instruction(&fx.app, &issue, "In Progress", "P".into())
            .contains("# Feature log")
    );
    fx.set_consent(Some(false));
    assert_eq!(
        skills_app::with_append_instruction(&fx.app, &issue, "In Progress", "P".into()),
        "P"
    );
}
