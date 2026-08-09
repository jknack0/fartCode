//! E19-02 (#71; ADR-0038 items 2–3): the seeded feature-log skill, the
//! `AGENTS.md` pointer, and the step-prompt append instruction.
//!
//! The scaffold surgery (idempotence, never-clobber, symlinks, fences,
//! version bumps) is proved in `fartcode_core::skills`. What can only be
//! proved here is the part that needs a wired App and a real worktree:
//!
//! - consent gates every write, on every path;
//! - the files land INSIDE the worktree, on the feature branch, in the
//!   user's pull request — not in the checkout they are standing in;
//! - deleting the scaffold STICKS, which is what makes the removal
//!   instructions printed inside it true;
//! - a version bump still heals, including for a feature already in flight.
//!
//! Everything drives the production entry points (`seed_for_task` via a
//! real dispatch, `with_append_instruction` via a real launch). There is no
//! test-only seeding seam — one existed in the first round, and it meant
//! the path production actually used was unpinned.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_app_lib::app::App;
use fartcode_app_lib::dispatch::issue_dispatch_core;
use fartcode_app_lib::skills as skills_app;
use fartcode_app_lib::step_engine;
use fartcode_core::issues::columns::{
    BoardColumn, ColumnKind, ColumnStore, NewColumn, OnEnter, OnSettle,
};
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

fn git_commit(dir: &Path, message: &str) {
    git_ok(dir, &["add", "."]);
    git_ok(
        dir,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@fartCode.dev",
            "commit",
            "-m",
            message,
        ],
    );
}

fn make_repo(tmp: &tempfile::TempDir) -> PathBuf {
    let repo = tmp.path().join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git_ok(&repo, &["init", "-q"]);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    git_commit(&repo, "init");
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

/// A card that has been dispatched onto an agent-step column: its worktree
/// exists and the production seeding path has already run.
struct Dispatched {
    issue_id: String,
    task_id: String,
    worktree: PathBuf,
    prompt: String,
}

impl Fixture {
    fn set_consent(&self, value: Option<bool>) {
        self.patch_settings(|s| s.feature_dossiers = value);
    }

    /// Forces the app's memory of what it seeded — the gate that makes a
    /// user's deletion stick. `None` = "never seeded", which is also how a
    /// version bump looks from the gate's point of view.
    fn set_seeded_version(&self, value: Option<u32>) {
        self.patch_settings(|s| s.feature_log_seeded_version = value);
    }

    fn seeded_version(&self) -> Option<u32> {
        self.app
            .settings
            .get_project_settings(&self.project_id, &self.repo)
            .unwrap()
            .feature_log_seeded_version
    }

    fn patch_settings(&self, edit: impl FnOnce(&mut fartcode_core::settings::ProjectSettings)) {
        let mut settings = self
            .app
            .settings
            .get_project_settings(&self.project_id, &self.repo)
            .unwrap();
        edit(&mut settings);
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

    fn column(&self, name: &str) -> BoardColumn {
        self.app
            .columns
            .list_for_project(&self.project_id)
            .unwrap()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no column named {name}"))
    }

    /// A second `agent_step` column — the "already has a worktree" path,
    /// which does NOT re-provision and so has its own seeding call site.
    fn extra_step(&self, name: &str) -> BoardColumn {
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

    /// The real board gesture: drop a card on an agent-step column.
    fn dispatch(&self, title: &str) -> Dispatched {
        let issue = self.new_issue(title);
        self.enter(&issue.id, &self.column("In Progress"))
    }

    fn enter(&self, issue_id: &str, column: &BoardColumn) -> Dispatched {
        let outcome = step_engine::enter_column(&self.app, issue_id, &column.id, None).unwrap();
        let launch = outcome.launch.expect("run-mode step launches");
        Dispatched {
            issue_id: issue_id.to_string(),
            task_id: launch.task.id.clone(),
            worktree: self.worktree_of(&launch.task.id),
            prompt: launch.prompt,
        }
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

fn scaffold_present(dir: &Path) -> bool {
    dir.join(skills::SKILL_FILE).is_file() || dir.join(skills::AGENTS_FILE).exists()
}

/// Overwrites the scaffold with an OLDER version, so a subsequent seed has
/// real work to do. Used to prove a gate is what stopped a rewrite, rather
/// than there being nothing to rewrite.
fn plant_stale_scaffold(worktree: &Path) {
    std::fs::write(worktree.join(skills::SKILL_FILE), skills::skill_body(0)).unwrap();
    std::fs::write(
        worktree.join(skills::AGENTS_FILE),
        format!("{}\n", skills::pointer_line(0)),
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// 1. Consent gates every write
// ---------------------------------------------------------------------------

/// The state of every project until #74's consent card ships. Unasked fails
/// closed: the dispatch still runs, and not one byte lands in the repo.
#[test]
fn consent_unasked_seeds_nothing() {
    let fx = fixture_unasked();
    let d = fx.dispatch("Implement OAuth login");
    assert!(
        !scaffold_present(&d.worktree),
        "an unasked project is never written to"
    );
    assert_eq!(fx.seeded_version(), None, "and nothing is recorded");
}

#[test]
fn consent_declined_seeds_nothing() {
    let fx = fixture();
    fx.set_consent(Some(false));
    let d = fx.dispatch("Implement OAuth login");
    assert!(
        !scaffold_present(&d.worktree),
        "a declined project is never written to"
    );
    assert_eq!(fx.seeded_version(), None);
}

/// Consent is what stops the write — not the absence of work to do. The
/// scaffold on disk is deliberately STALE and the app's memory of it
/// cleared, so a seed would rewrite both files if the gate let it through.
#[test]
fn withdrawn_consent_stops_a_rewrite_that_would_otherwise_happen() {
    let fx = fixture();
    let d = fx.dispatch("Implement OAuth login");
    assert!(scaffold_present(&d.worktree));

    plant_stale_scaffold(&d.worktree);
    fx.set_seeded_version(None); // only consent stands in the way now
    let stale = std::fs::read_to_string(d.worktree.join(skills::AGENTS_FILE)).unwrap();

    fx.set_consent(Some(false));
    skills_app::seed_for_task(&fx.app, &fx.project_id, &d.task_id);
    assert_eq!(
        std::fs::read_to_string(d.worktree.join(skills::AGENTS_FILE)).unwrap(),
        stale,
        "consent off: the stale scaffold is left exactly as it was"
    );

    // …and the same call with consent restored DOES rewrite it, which is
    // what proves the setup above was rewritable in the first place.
    fx.set_consent(Some(true));
    skills_app::seed_for_task(&fx.app, &fx.project_id, &d.task_id);
    let healed = std::fs::read_to_string(d.worktree.join(skills::AGENTS_FILE)).unwrap();
    assert_ne!(healed, stale);
    assert!(healed.contains(&format!(
        "{}{}",
        skills::POINTER_MARKER,
        skills::FEATURE_LOG_VERSION
    )));
}

// ---------------------------------------------------------------------------
// 2. The scaffold lands in the worktree, on the feature branch
// ---------------------------------------------------------------------------

#[test]
fn a_dispatched_step_seeds_the_convention_inside_the_worktree() {
    let fx = fixture();
    let d = fx.dispatch("Implement OAuth login");

    let skill = std::fs::read_to_string(d.worktree.join(skills::SKILL_FILE)).unwrap();
    assert!(skill.contains("written by fartCode (ADR-0038)"));
    assert!(skill.contains(&format!(
        "{}{}",
        skills::SKILL_MARKER,
        skills::FEATURE_LOG_VERSION
    )));
    let agents = std::fs::read_to_string(d.worktree.join(skills::AGENTS_FILE)).unwrap();
    assert!(agents.contains("written by fartCode (ADR-0038)"));
    assert_eq!(
        agents
            .lines()
            .filter(|l| l.contains(skills::POINTER_MARKER))
            .count(),
        1
    );

    // The user's checkout is never touched — a write there would be a
    // silent mutation of the tree they are standing in.
    assert!(
        !fx.repo.join(skills::SKILL_DIR).exists(),
        "the main checkout is left alone"
    );
    assert!(!fx.repo.join(skills::AGENTS_FILE).exists());

    // …and the app now remembers, at the current version.
    assert_eq!(fx.seeded_version(), Some(skills::FEATURE_LOG_VERSION));
}

/// A repo that already has a hand-written `AGENTS.md` keeps it whole and
/// gains one line — through the real dispatch path.
#[test]
fn a_hand_written_agents_file_in_the_repo_survives_dispatch() {
    let fx = fixture();
    const HAND_WRITTEN: &str = "# AGENTS.md\n\n## Build\n\nRun `make`.\n";
    std::fs::write(fx.repo.join("AGENTS.md"), HAND_WRITTEN).unwrap();
    git_commit(&fx.repo, "agents");

    let d = fx.dispatch("Dark mode");
    let after = std::fs::read_to_string(d.worktree.join("AGENTS.md")).unwrap();
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
// 3. Deleting the scaffold sticks; a version bump still heals
// ---------------------------------------------------------------------------

/// The scaffold tells the user "delete this to remove the convention".
/// Seeding runs on every launch, so that sentence is only true because the
/// app records what it wrote and stops looking. Without this the files
/// resurrect on the next card, forever, in every future pull request.
#[test]
fn a_deleted_scaffold_stays_deleted() {
    let fx = fixture();
    let d = fx.dispatch("Implement OAuth login");
    assert!(scaffold_present(&d.worktree));

    // The user removes it, exactly as the file's own instructions say.
    std::fs::remove_dir_all(d.worktree.join(skills::SKILL_DIR)).unwrap();
    std::fs::remove_file(d.worktree.join(skills::AGENTS_FILE)).unwrap();

    skills_app::seed_for_task(&fx.app, &fx.project_id, &d.task_id);
    assert!(
        !scaffold_present(&d.worktree),
        "it does not come back on the next launch"
    );

    // A genuine format change still heals — that is what the version is
    // for. (A bump is indistinguishable from "recorded an older version".)
    fx.set_seeded_version(Some(0));
    skills_app::seed_for_task(&fx.app, &fx.project_id, &d.task_id);
    assert!(
        d.worktree.join(skills::SKILL_FILE).is_file(),
        "a version bump reseeds"
    );
    assert_eq!(fx.seeded_version(), Some(skills::FEATURE_LOG_VERSION));
}

/// A second step reuses the existing worktree, so the provisioning path —
/// the other seeding site — never runs. Without a call in `launch_step` a
/// version bump would never reach a feature already in flight.
#[test]
fn a_second_step_in_an_existing_worktree_heals_a_stale_scaffold() {
    let fx = fixture();
    let d = fx.dispatch("Implement OAuth login");
    let review = fx.extra_step("Review");

    plant_stale_scaffold(&d.worktree);
    fx.set_seeded_version(Some(0)); // as a version bump would look

    let second = fx.enter(&d.issue_id, &review);
    assert_eq!(
        second.worktree, d.worktree,
        "same worktree — no re-provision, so provisioning cannot be what seeded"
    );
    let skill = std::fs::read_to_string(d.worktree.join(skills::SKILL_FILE)).unwrap();
    assert!(
        skill.contains(&format!(
            "{}{}",
            skills::SKILL_MARKER,
            skills::FEATURE_LOG_VERSION
        )),
        "the in-flight feature's scaffold was refreshed"
    );
    assert_eq!(fx.seeded_version(), Some(skills::FEATURE_LOG_VERSION));
}

// ---------------------------------------------------------------------------
// 4. The prompt half
// ---------------------------------------------------------------------------

/// The strongest form of the ADR-0038 item 2 claim: the prompt an agent
/// actually receives ends with the append instruction, naming the column it
/// is running as.
#[test]
fn the_step_prompt_ends_with_the_append_instruction_and_names_the_column() {
    let fx = fixture();
    let prompt = fx.dispatch("Implement OAuth login").prompt;

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

/// The legacy board dispatch builds its packet on a different path, before
/// provisioning — so the instruction has to be appended after the card is
/// re-read. Untested in the first round.
#[test]
fn the_legacy_dispatch_prompt_carries_the_append_instruction() {
    let fx = fixture();
    let issue = fx.new_issue("Implement OAuth login");

    let outcome = issue_dispatch_core(&fx.app, &issue.id).unwrap();
    assert!(!outcome.reattached);
    let prompt = outcome.prompt;

    assert!(prompt.contains("# Feature log"), "{prompt}");
    assert!(
        prompt.contains("## In Progress — <YYYY-MM-DD>"),
        "names the seeded dispatch column:\n{prompt}"
    );
    assert!(
        prompt.contains("docs/features/implement-oauth-login.md"),
        "names the card's real dossier — which only exists after \
         provisioning, so ordering matters:\n{prompt}"
    );
    assert!(prompt.find("# Issue").unwrap() < prompt.find("# Feature log").unwrap());
    // …and this path seeds the scaffold too.
    let worktree = fx.worktree_of(&outcome.task.id);
    assert!(worktree.join(skills::SKILL_FILE).is_file());
}

/// The inverse, and the one that matters most: an agent in a project that
/// declined is never told to write a dossier. It would create exactly the
/// file the app refused to create.
#[test]
fn a_declined_project_gets_no_append_instruction() {
    let fx = fixture_unasked();
    let prompt = fx.dispatch("Implement OAuth login").prompt;

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
