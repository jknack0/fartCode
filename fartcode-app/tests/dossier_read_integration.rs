//! E19-06 (#75; handoff v3 §8f + §8h) — the card detail's dossier read and
//! the ⌘K feature row's card.
//!
//! The parsing is unit-tested in `fartcode_core::dossier_view`. What can
//! only be proven here is the RESOLUTION: that the read finds the card's
//! own dossier in a real worktree, that it refuses a file at the same path
//! that is not ours (`docs/features/` is a common hand-written convention —
//! that bug was found twice in this epic), and — the E19-06 fix round —
//! that a REAL settle driven through `settle_issues_for_task` leaves a
//! settle breadcrumb, on the `on_settle: advance` arm the seeded board
//! actually uses.
//!
//! Real provisioning runs (git init, `worktree add`) inside a tempdir, for
//! the same reason E19-01's and E19-03's suites do: the two-copies premise
//! is a filesystem fact.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_app_lib::app::App;
use fartcode_app_lib::commands::dossiers::{feature_rows, read_dossier};
use fartcode_app_lib::dossiers::TimelineAppender;
use fartcode_app_lib::step_engine;
use fartcode_core::dossier_index as core_index;
use fartcode_core::dossiers;
use fartcode_core::events::InternalEvent;
use fartcode_core::issues::{Issue, NewIssue};
use fartcode_core::projects::ProjectStore;
use fartcode_core::settings::{LocalProjectGroup, LOCAL_PROJECT};
use fartcode_core::tasks::TaskStore;

/// What an agent appends before settling (ADR-0038 item 2).
const AGENT_SECTIONS: &str = concat!(
    "\n## Plan — 2026-08-09\n\n",
    "Gate the send path, not accept.\n\n",
    "### Rejected\n\n",
    "A per-org allowlist: goes stale faster than the queue it replaces.\n\n",
    "## Implement — 2026-08-09\n\n",
    "Vetting lives in the send interceptor.\n",
);

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

/// A project that has CONSENTED to dossiers — without consent no file is
/// written, and there is nothing to read.
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
    let fx = Fixture {
        _tmp: tmp,
        app,
        project_id: project.id,
    };
    fx.set_consent(true);
    fx
}

impl Fixture {
    fn set_consent(&self, value: bool) {
        let project = self.app.projects.get(&self.project_id).unwrap().unwrap();
        let mut settings = self
            .app
            .settings
            .get_project_settings(&self.project_id, &project.path)
            .unwrap();
        settings.feature_dossiers = Some(value);
        self.app
            .settings
            .update_project_settings(&self.project_id, &project.path, &settings)
            .unwrap();
    }

    fn new_issue(&self, title: &str) -> Issue {
        self.app
            .issues
            .create(NewIssue {
                project_id: self.project_id.clone(),
                title: title.into(),
                body: Some("resend crashes on an active invite".into()),
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

    fn issue(&self, id: &str) -> Issue {
        self.app.issues.get(id).unwrap().unwrap()
    }

    fn column(&self, name: &str) -> String {
        self.app
            .columns
            .list_for_project(&self.project_id)
            .unwrap()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no column named {name}"))
            .id
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

    fn project_root(&self) -> PathBuf {
        self.app
            .projects
            .get(&self.project_id)
            .unwrap()
            .unwrap()
            .path
    }

    /// A card in a step column: worktree provisioned, dossier born, the
    /// launch breadcrumb written by the app's own appender.
    fn card_in_step(&self, title: &str) -> (Issue, PathBuf) {
        let issue = self.new_issue(title);
        let column = self.column("Implement");
        step_engine::enter_column(&self.app, &issue.id, &column, None).unwrap();
        let stored = self.issue(&issue.id);
        let rel = stored.dossier_path.clone().expect("dossier born");
        let task_id = stored.linked_task_id.clone().expect("task linked");
        let worktree = self.worktree_of(&task_id);

        // The launch line the TimelineAppender writes off `StepLaunch`
        // (the subscriber loop is a boot wiring, so the handler is driven
        // directly here — the same shape E19-01's suite uses).
        TimelineAppender::new(self.app.clone()).handle(&InternalEvent::StepLaunch {
            issue_id: issue.id.clone(),
            project_id: self.project_id.clone(),
            column_id: column.clone(),
            task_id: task_id.clone(),
            prompt: String::new(),
            provider: "claude".into(),
            model: None,
            effort: None,
            reattached: false,
        });
        (self.issue(&issue.id), worktree.join(&rel))
    }

    /// A card whose step really SETTLED: driven through
    /// `settle_issues_for_task`, so the breadcrumb under test is the one
    /// production writes.
    ///
    /// This is deliberately not a hand-injected `· settled` line. The
    /// seeded `In Progress` is `on_settle: advance`, and that arm emits no
    /// `StepSettled` — a fixture that writes the line itself proves the
    /// appender is wired when it is not, which is exactly how "every step
    /// renders as permanently running" shipped.
    fn card_settled(&self, title: &str) -> (Issue, PathBuf) {
        let (issue, path) = self.card_in_step(title);
        let task_id = issue.linked_task_id.clone().unwrap();

        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(AGENT_SECTIONS);
        std::fs::write(&path, text).unwrap();

        assert_eq!(
            step_engine::settle_issues_for_task(&self.app, &task_id, Some("pty:test")),
            1,
            "the step settled"
        );
        (self.issue(&issue.id), path)
    }
}

// ---------------------------------------------------------------------------
// 1. The read
// ---------------------------------------------------------------------------

#[test]
fn the_read_returns_the_path_the_folded_timeline_and_the_agent_sections() {
    let fx = fixture();
    let (issue, path) = fx.card_settled("Admin resend on an active invite crashes");

    let dossier = read_dossier(&fx.app, &issue.id).expect("the card has a dossier");
    assert_eq!(dossier.path, issue.dossier_path.unwrap());
    assert_eq!(
        PathBuf::from(&dossier.host_path),
        path,
        "the live worktree copy is the one an agent is writing"
    );

    // §8f timeline: the backfilled creation line, the birth line, and the
    // launch folded into the settle the ENGINE wrote.
    let texts: Vec<&str> = dossier.timeline.iter().map(|e| e.text.as_str()).collect();
    assert!(
        texts.iter().any(|t| t.starts_with("created · ")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("Implement · claude · launched → settled")),
        "the launch/settle pair folds into one line: {texts:?}"
    );
    assert!(
        !texts.contains(&"Implement · settled"),
        "the settle is folded, not listed twice: {texts:?}"
    );
    // THE regression: the seeded step is `on_settle: advance`, which emits
    // no `StepSettled` — before this fix round nothing wrote a settle
    // breadcrumb at all and every step rendered as permanently running.
    assert!(
        dossier.timeline.iter().all(|e| !e.running),
        "a settled step must not read as running: {texts:?}"
    );
    // And the breadcrumb is really ON DISK, written by the advance arm
    // itself — not merely inferred by the reader from the column move that
    // follows it (that inference is the repair path for older files).
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        on_disk.contains("Implement · settled"),
        "the advance arm writes the settle line:\n{on_disk}"
    );
    assert!(
        on_disk.find("Implement · launched").unwrap()
            < on_disk.find("Implement · settled").unwrap(),
        "the settle lands after the launch it closes"
    );

    // §8f inset card: the AGENT's sections only — never the app's own
    // header or Timeline (same line the ⌘K indexer draws).
    let headings: Vec<&str> = dossier
        .sections
        .iter()
        .map(|s| s.heading.as_str())
        .collect();
    assert_eq!(
        headings,
        vec!["Plan — 2026-08-09", "Implement — 2026-08-09"]
    );
    assert!(dossier.sections[0].body.contains("Gate the send path"));
    assert!(
        dossier.sections[0].body.contains("### Rejected"),
        "a subheading belongs to its section"
    );
}

/// §8f: "a skipped append = timeline intact, no inset section."
#[test]
fn a_skipped_append_leaves_the_timeline_and_no_sections() {
    let fx = fixture();
    let issue = fx.new_issue("Nobody wrote a section");
    step_engine::enter_column(&fx.app, &issue.id, &fx.column("Implement"), None).unwrap();

    let dossier = read_dossier(&fx.app, &fx.issue(&issue.id).id).expect("dossier born");
    assert!(
        !dossier.timeline.is_empty(),
        "the facts survive the missing reasoning"
    );
    assert!(dossier.sections.is_empty(), "and nothing is invented");
}

/// The defect found twice in this epic: `docs/features/` is a common
/// hand-written convention, so a file EXISTING at the card's dossier path
/// is not evidence that it is the card's dossier. Resolution goes through
/// `dossiers::inspect`, never the path.
#[test]
fn a_foreign_file_at_the_same_path_is_refused() {
    let fx = fixture();
    let (issue, path) = fx.card_settled("Admin resend on an active invite crashes");
    assert!(read_dossier(&fx.app, &issue.id).is_some());

    const HUMAN_DOC: &str = "# Invite vetting\n\nOur design notes. Not a dossier.\n";
    std::fs::write(&path, HUMAN_DOC).unwrap();
    assert!(
        read_dossier(&fx.app, &issue.id).is_none(),
        "a stranger's document must never render as the card's dossier"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        HUMAN_DOC,
        "and reading it changes nothing"
    );

    // Another card's dossier at the same path is refused for the same
    // reason: one feature's reasoning must not surface on another's card.
    let other = fx.new_issue("Someone else's feature");
    let theirs = dossiers::backfilled_header(&other, None);
    assert!(
        theirs.contains(&dossiers::card_marker(&other.id)),
        "the file really is the other card's"
    );
    std::fs::write(&path, &theirs).unwrap();
    assert!(read_dossier(&fx.app, &issue.id).is_none());
}

#[test]
fn a_card_with_no_dossier_reads_nothing() {
    let fx = fixture();
    // Pre-E19 / declined consent: no dossier_path at all.
    let bare = fx.new_issue("Never dispatched");
    assert!(bare.dossier_path.is_none());
    assert!(read_dossier(&fx.app, &bare.id).is_none());

    // A path recorded, but the file left with a deleted branch.
    let (issue, path) = fx.card_in_step("Gone with the branch");
    std::fs::remove_file(&path).unwrap();
    assert!(read_dossier(&fx.app, &issue.id).is_none());

    assert!(read_dossier(&fx.app, "iss_does_not_exist").is_none());
}

// ---------------------------------------------------------------------------
// 2. The ⌘K feature row (§8h)
// ---------------------------------------------------------------------------

#[test]
fn a_feature_row_names_its_card() {
    let fx = fixture();
    let (issue, _) = fx.card_settled("Admin resend on an active invite crashes");
    let item_id = core_index::item_id(&issue.id, "Plan — 2026-08-09", 0);

    let rows = feature_rows(&fx.app, std::slice::from_ref(&item_id));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].item_id, item_id);
    assert_eq!(rows[0].issue_id, issue.id, "↵ opens the card detail");
    assert_eq!(rows[0].title, "Admin resend on an active invite crashes");

    // A row whose card is gone resolves to nothing rather than to a hit
    // that opens a dead card detail.
    fx.app.issues.delete(&issue.id).unwrap();
    assert!(feature_rows(&fx.app, &[item_id]).is_empty());
    assert!(feature_rows(&fx.app, &["not-a-feature-row".to_string()]).is_empty());
}

/// One dossier is many `feature` rows, and the palette asks about all of
/// them on every keystroke — so the card is resolved once and fanned out,
/// not looked up per section.
#[test]
fn many_sections_of_one_feature_resolve_to_one_card() {
    let fx = fixture();
    let (issue, _) = fx.card_settled("Admin resend on an active invite crashes");
    let ids: Vec<String> = ["Plan — 2026-08-09", "Implement — 2026-08-09"]
        .iter()
        .map(|h| core_index::item_id(&issue.id, h, 0))
        .collect();

    let rows = feature_rows(&fx.app, &ids);
    assert_eq!(rows.len(), 2, "every asked-about id comes back");
    assert!(rows.iter().all(|r| r.issue_id == issue.id));
    assert_eq!(rows[0].item_id, ids[0]);
    assert_eq!(rows[1].item_id, ids[1]);
}

/// The card detail keeps reading the WORKTREE copy while one exists, even
/// when a stranger's file sits at the same repo path in the checkout.
#[test]
fn a_foreign_file_in_the_checkout_does_not_shadow_the_worktree_copy() {
    let fx = fixture();
    let (issue, _) = fx.card_settled("Admin resend on an active invite crashes");

    let checkout_path = fx.project_root().join(issue.dossier_path.clone().unwrap());
    std::fs::create_dir_all(checkout_path.parent().unwrap()).unwrap();
    std::fs::write(&checkout_path, "# Invite vetting\n\nSomebody's notes.\n").unwrap();

    let dossier = read_dossier(&fx.app, &issue.id).expect("the worktree copy is still ours");
    assert_eq!(dossier.sections.len(), 2);
    assert!(!dossier
        .host_path
        .starts_with(fx.project_root().to_string_lossy().as_ref()));
}
