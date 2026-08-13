//! E19-03 (#72; ADR-0038 item 4) — dossier sections as ⌘K `feature` rows.
//!
//! The parsing and set arithmetic are unit-tested in
//! `fartcode_core::dossier_index`. What can only be proven here is the
//! wiring: that a real settle indexes the WORKTREE copy, that a project
//! pull indexes the MAIN-BRANCH copy, and that deleting a card or a project
//! takes its rows with it — no orphan row whose Enter would open the card
//! detail of a dead issue.
//!
//! Real provisioning runs (git init, `worktree add`) inside a tempdir, for
//! the same reason E19-01's suite does: the two-copies premise is a
//! filesystem fact.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use fartcode_app_lib::app::App;
use fartcode_app_lib::dossier_index;
use fartcode_app_lib::step_engine;
use fartcode_core::dossier_index as core_index;
use fartcode_core::events::{EventBus, InternalEvent};
use fartcode_core::issues::{Issue, NewIssue};
use fartcode_core::projects::ProjectStore;
use fartcode_core::search::{self, SearchResult};
use fartcode_core::settings::{LocalProjectGroup, LOCAL_PROJECT};
use fartcode_core::tasks::TaskStore;

/// What an agent appends before settling (ADR-0038 item 2), including the
/// hostile shapes E19-01 hardened the append path against: a `## ` heading
/// inside a fenced sample, a `###` subheading, and CRLF line endings.
const AGENT_SECTIONS: &str = concat!(
    "\r\n## Plan — 2026-08-09\r\n\r\n",
    "Chose PKCE over the implicit flow.\r\n\r\n",
    "### Rejected\r\n\r\n",
    "A session cookie: the mobile client cannot hold one.\r\n\r\n",
    "The seeded skill documents the format:\r\n\r\n",
    "```md\r\n## Verify — <date>\r\n\r\nnot a real section\r\n```\r\n\r\n",
    "## Implement — 2026-08-09\r\n\r\n",
    "Token refresh lives in the interceptor.\r\n",
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

/// A project that has CONSENTED to dossiers — indexing follows the data, so
/// without consent there is no file to index and nothing to test.
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
                body: Some("we need login".into()),
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

    fn issue(&self, id: &str) -> Issue {
        self.app.issues.get(id).unwrap().unwrap()
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

    /// A card sitting in a step column with its dossier born in the
    /// worktree, plus the agent's sections appended — the state a settle
    /// finds.
    fn card_mid_step(&self, title: &str) -> (Issue, String) {
        let issue = self.new_issue(title);
        step_engine::enter_column(&self.app, &issue.id, &self.column("In Progress").id, None)
            .unwrap();
        let stored = self.issue(&issue.id);
        let rel = stored.dossier_path.clone().expect("dossier born");
        let task_id = stored.linked_task_id.clone().expect("task linked");
        let path = self.worktree_of(&task_id).join(&rel);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(AGENT_SECTIONS);
        std::fs::write(&path, text).unwrap();
        (self.issue(&issue.id), task_id)
    }

    fn feature_hits(&self, q: &str) -> Vec<SearchResult> {
        search::query(&self.app.db, q, 20)
            .unwrap()
            .into_iter()
            .filter(|h| h.item_type == core_index::ITEM_TYPE)
            .collect()
    }

    fn feature_rows(&self) -> i64 {
        self.app
            .db
            .conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM search_index WHERE item_type = ?1",
                [core_index::ITEM_TYPE],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// Every `feature` item_id, sorted — the identity of the row set, for
    /// asserting that a wipe-and-restore round trip is lossless.
    fn feature_item_ids(&self) -> Vec<String> {
        let conn = self.app.db.conn().lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT item_id FROM search_index WHERE item_type = ?1")
            .unwrap();
        let mut ids: Vec<String> = stmt
            .query_map([core_index::ITEM_TYPE], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        ids.sort();
        ids
    }

    /// Everything the bus has emitted since `rx` was taken, fed through the
    /// indexer's teardown arm — so the test exercises the events production
    /// actually emits, not hand-built ones.
    fn drain_into_indexer(&self, rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) {
        while let Ok(event) = rx.try_recv() {
            dossier_index::handle_event(&self.app, &event);
        }
    }
}

// ---------------------------------------------------------------------------
// 1. Settle indexes the worktree copy
// ---------------------------------------------------------------------------

#[test]
fn a_settle_indexes_the_agent_sections_and_search_opens_the_card() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("Implement OAuth login");
    assert_eq!(fx.feature_rows(), 0, "nothing indexed before the settle");

    assert_eq!(
        step_engine::settle_issues_for_task(&fx.app, &task_id, None),
        1
    );

    // Exactly the two agent-written sections. The app's own skeleton —
    // Context / Acceptance / References / Timeline — is not search material,
    // and neither is the `## Verify` line inside the fenced sample.
    let mut titles: Vec<String> = fx
        .feature_hits("2026-08-09")
        .into_iter()
        .map(|h| h.title)
        .collect();
    titles.sort();
    assert_eq!(
        titles,
        vec!["Implement — 2026-08-09", "Plan — 2026-08-09"],
        "CRLF parsed, `###` did not split, the fenced heading is not a section"
    );
    assert_eq!(fx.feature_rows(), 2);

    // Body text is searchable, and the subheading did not become its own row.
    let hits = fx.feature_hits("cookie");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].title, "Plan — 2026-08-09");

    // §8h: Enter opens the card detail, so every row resolves to its issue.
    for hit in fx.feature_hits("2026-08-09") {
        assert_eq!(
            core_index::issue_id_of(&hit.item_id),
            Some(issue.id.as_str())
        );
        assert_eq!(hit.project_id.as_deref(), Some(fx.project_id.as_str()));
        assert!(
            fx.app.issues.get(&issue.id).unwrap().is_some(),
            "the id the palette would navigate with is live"
        );
    }
}

#[test]
fn resettling_does_not_duplicate_rows_and_a_removed_section_loses_its_row() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &task_id, None);
    assert_eq!(fx.feature_rows(), 2);

    // Same file again: the deterministic rowid replaces, never appends.
    dossier_index::reindex_issue(&fx.app, &fx.issue(&issue.id));
    dossier_index::reindex_issue(&fx.app, &fx.issue(&issue.id));
    assert_eq!(fx.feature_rows(), 2, "reindex is idempotent");

    // Drop a section from the file (an agent rewriting history, or a merge
    // resolution that lost one) — its row must go.
    let stored = fx.issue(&issue.id);
    let path = fx
        .worktree_of(&task_id)
        .join(stored.dossier_path.clone().unwrap());
    let text = std::fs::read_to_string(&path).unwrap();
    let trimmed = text.split("## Implement — 2026-08-09").next().unwrap();
    std::fs::write(&path, trimmed).unwrap();

    dossier_index::reindex_issue(&fx.app, &stored);
    assert_eq!(fx.feature_rows(), 1);
    assert!(
        fx.feature_hits("interceptor").is_empty(),
        "a section that no longer exists must not be findable"
    );
}

// ---------------------------------------------------------------------------
// 2. Project pull indexes the main-branch copy
// ---------------------------------------------------------------------------

#[test]
fn a_project_pull_indexes_the_landed_copy_of_a_torn_down_feature() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &task_id, None);
    assert_eq!(fx.feature_rows(), 2);

    // The feature lands: the dossier appears in the main checkout, with one
    // more section than the worktree copy had.
    let rel = fx.issue(&issue.id).dossier_path.unwrap();
    let landed = fx.project_root().join(&rel);
    std::fs::create_dir_all(landed.parent().unwrap()).unwrap();
    let mut text = std::fs::read_to_string(fx.worktree_of(&task_id).join(&rel)).unwrap();
    text.push_str("\n## Review — 2026-08-10\n\nShipped behind a flag.\n");
    std::fs::write(&landed, text).unwrap();

    // …and the worktree is torn down, so only the landed copy remains.
    std::fs::remove_dir_all(fx.worktree_of(&task_id)).unwrap();

    dossier_index::reindex_project(&fx.app, &fx.project_id);
    assert_eq!(fx.feature_rows(), 3, "the landed copy is the source now");
    let hits = fx.feature_hits("flag");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].title, "Review — 2026-08-10");
    assert_eq!(
        core_index::issue_id_of(&hits[0].item_id),
        Some(issue.id.as_str())
    );
}

#[test]
fn a_dossier_that_exists_in_neither_copy_loses_its_rows() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &task_id, None);
    assert_eq!(fx.feature_rows(), 2);

    // The unmerged branch is deleted with its worktree: the dossier goes
    // with it (ADR-0038 item 5) and never reached the main checkout.
    std::fs::remove_dir_all(fx.worktree_of(&task_id)).unwrap();
    dossier_index::reindex_issue(&fx.app, &fx.issue(&issue.id));

    assert_eq!(fx.feature_rows(), 0, "no rows pointing at a vanished file");
}

// ---------------------------------------------------------------------------
// 3. Rows die with the dossier's owner
// ---------------------------------------------------------------------------

#[test]
fn deleting_the_card_drops_its_feature_rows_and_leaves_its_neighbours() {
    let fx = fixture();
    let (doomed, doomed_task) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &doomed_task, None);
    let (survivor, survivor_task) = fx.card_mid_step("Add dark mode");
    step_engine::settle_issues_for_task(&fx.app, &survivor_task, None);
    assert_eq!(fx.feature_rows(), 4);

    let mut rx = fx.app.event_bus.subscribe();
    fx.app.issues.delete(&doomed.id).unwrap();
    fx.drain_into_indexer(&mut rx);

    assert_eq!(fx.feature_rows(), 2, "only the deleted card's rows went");
    for hit in fx.feature_hits("2026-08-09") {
        assert_eq!(
            core_index::issue_id_of(&hit.item_id),
            Some(survivor.id.as_str()),
            "no hit may open the card detail of a deleted issue"
        );
    }
}

#[test]
fn deleting_the_project_drops_every_feature_row_it_owned() {
    let fx = fixture();
    let (_, task_id) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &task_id, None);
    assert_eq!(fx.feature_rows(), 2);

    // A project delete cascades its issues in SQL, emitting one
    // `ProjectDeleted` and no `IssueDeleted` — the case a per-card sweep
    // would miss entirely.
    let mut rx = fx.app.event_bus.subscribe();
    fx.app.projects.delete(&fx.project_id).unwrap();
    fx.drain_into_indexer(&mut rx);

    assert_eq!(fx.feature_rows(), 0);
}

// ---------------------------------------------------------------------------
// 4. Only OUR file, and only the FRESHER copy
// ---------------------------------------------------------------------------

/// `docs/features/` is a common hand-written convention, so a file at the
/// card's slug path in the main checkout is not evidence that it is the
/// card's dossier. Indexing it on existence alone turned a stranger's prose
/// into ⌘K hits that opened an unrelated card — the same adopt-any-file
/// defect E19-01's review fixed for the write path.
#[test]
fn a_foreign_file_at_the_landed_path_is_never_indexed() {
    const HUMAN_DOC: &str =
        "# OAuth login\n\n## Design spec — v2\n\nNot a dossier. Mentions fenugreek.\n";
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &task_id, None);
    assert_eq!(fx.feature_rows(), 2);

    let rel = fx.issue(&issue.id).dossier_path.unwrap();
    let landed = fx.project_root().join(&rel);
    std::fs::create_dir_all(landed.parent().unwrap()).unwrap();
    std::fs::write(&landed, HUMAN_DOC).unwrap();
    // Tear the worktree down so the foreign file is the only candidate.
    std::fs::remove_dir_all(fx.worktree_of(&task_id)).unwrap();

    dossier_index::reindex_issue(&fx.app, &fx.issue(&issue.id));

    assert!(
        fx.feature_hits("fenugreek").is_empty(),
        "someone's document must not become this card's search rows"
    );
    assert_eq!(fx.feature_rows(), 0, "and the vanished dossier's rows went");
    assert_eq!(
        std::fs::read_to_string(&landed).unwrap(),
        HUMAN_DOC,
        "byte-identical"
    );
}

/// Preferring the worktree copy whenever it EXISTS pinned the index to a
/// stale file after a merge: the pull writes the newer copy into the
/// checkout while the worktree is still live.
#[test]
fn the_fresher_copy_wins_so_a_merge_and_pull_is_not_ignored() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &task_id, None);
    assert_eq!(fx.feature_rows(), 2);

    let rel = fx.issue(&issue.id).dossier_path.unwrap();
    let in_worktree = fx.worktree_of(&task_id).join(&rel);
    let landed = fx.project_root().join(&rel);
    std::fs::create_dir_all(landed.parent().unwrap()).unwrap();
    let mut text = std::fs::read_to_string(&in_worktree).unwrap();
    text.push_str("\n## Review — 2026-08-10\n\nShipped behind a flag.\n");
    std::fs::write(&landed, &text).unwrap();

    // Stamped explicitly rather than slept on: filesystem timestamp
    // granularity is not something a test should race.
    set_mtime(&landed, 600);
    set_mtime(&in_worktree, -600);
    dossier_index::reindex_issue(&fx.app, &fx.issue(&issue.id));
    assert_eq!(fx.feature_rows(), 3, "the newer landed copy won");
    assert_eq!(fx.feature_hits("flag").len(), 1);

    // Flip the ordering: an agent writing in the live worktree wins again.
    set_mtime(&in_worktree, 1_200);
    dossier_index::reindex_issue(&fx.app, &fx.issue(&issue.id));
    assert_eq!(fx.feature_rows(), 2, "the newer worktree copy won");
    assert!(fx.feature_hits("flag").is_empty());
    assert_eq!(
        core_index::issue_id_of(&fx.feature_hits("PKCE")[0].item_id),
        Some(issue.id.as_str())
    );
}

/// Sets a file's mtime `offset_secs` from now (negative for the past).
fn set_mtime(path: &Path, offset_secs: i64) {
    let now = std::time::SystemTime::now();
    let when = if offset_secs >= 0 {
        now + std::time::Duration::from_secs(offset_secs as u64)
    } else {
        now - std::time::Duration::from_secs(offset_secs.unsigned_abs())
    };
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(when))
        .unwrap();
}

// ---------------------------------------------------------------------------
// 5. The production wiring, not just the handlers
// ---------------------------------------------------------------------------

/// Polls `check` until it holds or the deadline passes. Bounded on purpose:
/// the event bus is a broadcast channel and the boot sweep runs on the
/// blocking pool, so "wait for the real wiring" must never mean "hang".
fn await_until(mut check: impl FnMut() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !check() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// The boot backfill CLEARS the whole `search_index` table, so the feature
/// rows only survive a launch because the sweep rebuilds them from the
/// files. Pin the round trip: same ids in, same ids out.
#[test]
fn the_boot_sweep_restores_exactly_the_rows_the_backfill_wiped() {
    let fx = fixture();
    let (_, task_id) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &task_id, None);
    let before = fx.feature_item_ids();
    assert_eq!(before.len(), 2);

    // The production wiring verbatim, exactly as `lib.rs` calls it.
    fartcode_app_lib::indexer::spawn_search_indexer(fx.app.clone());
    assert_eq!(
        fx.feature_rows(),
        0,
        "the synchronous backfill wipes the table — that is the hazard"
    );

    await_until(
        || fx.feature_item_ids() == before,
        "the boot sweep never restored the feature rows",
    );
}

/// The delete arms only protect anything if the real subscription is wired
/// to them — driving `handle_event` by hand would pass with
/// `spawn_search_indexer` deleted.
#[test]
fn the_spawned_indexer_drops_the_rows_when_a_card_is_deleted() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("Implement OAuth login");
    step_engine::settle_issues_for_task(&fx.app, &task_id, None);

    fartcode_app_lib::indexer::spawn_search_indexer(fx.app.clone());
    // The sweep runs AFTER the subscriber subscribes, so waiting for the
    // rows to come back also proves the subscription exists — no need to
    // resend the deletion into a channel nobody is listening on yet.
    await_until(|| fx.feature_rows() == 2, "boot sweep never ran");

    fx.app.issues.delete(&issue.id).unwrap();
    await_until(
        || fx.feature_rows() == 0,
        "the spawned subscriber never dropped the deleted card's rows",
    );
}

// ---------------------------------------------------------------------------
// 5b. #142: a renamed task must be findable under its NEW title
// ---------------------------------------------------------------------------

/// The rename path is `search::update_title`'s ONLY caller (a plain upsert
/// would wipe the task's project link columns). Pin it end to end: boot
/// backfill indexes the old name, the `TaskRenamed` event retitles the row,
/// and the old title stops matching.
#[test]
fn the_spawned_indexer_retitles_a_renamed_task() {
    let fx = fixture();
    let task = fx
        .app
        .tasks
        .create(fartcode_core::tasks::CreateTaskOptions::new(
            fx.project_id.clone(),
            "old task name",
        ))
        .unwrap();

    // The boot backfill is synchronous: the task is indexed NOW, under its
    // original name.
    fartcode_app_lib::indexer::spawn_search_indexer(fx.app.clone());
    assert!(
        search::query(&fx.app.db, "old task name", 10)
            .unwrap()
            .iter()
            .any(|h| h.item_type == "task"),
        "boot backfill never indexed the task"
    );

    // Renaming may fire in the tiny window before the spawned subscriber
    // has subscribed (a broadcast with no receivers drops the event), so
    // rename until it lands — rename is idempotent and always emits
    // `TaskRenamed`.
    await_until(
        || {
            let _ = fx.app.tasks.rename(&task.id, "new task name");
            search::query(&fx.app.db, "new task name", 10)
                .unwrap()
                .iter()
                .any(|h| h.item_type == "task")
        },
        "the spawned subscriber never retitled the renamed task",
    );

    // The row's title column was replaced — the old title is gone, not
    // merely joined by the new one.
    assert!(
        search::query(&fx.app.db, "old task name", 10)
            .unwrap()
            .is_empty(),
        "the renamed task is still findable under its old title"
    );
}

// ---------------------------------------------------------------------------
// 6. Nothing to index is not an error
// ---------------------------------------------------------------------------

#[test]
fn a_card_without_a_dossier_indexes_nothing_and_never_fails() {
    let fx = fixture();
    let issue = fx.new_issue("No dossier here");
    dossier_index::reindex_issue(&fx.app, &issue);
    dossier_index::reindex_project(&fx.app, &fx.project_id);
    dossier_index::reindex_all(&fx.app);
    assert_eq!(fx.feature_rows(), 0);
}

// ---------------------------------------------------------------------------
// 7. #83: ` · landed` is a base-ref ancestry answer
// ---------------------------------------------------------------------------

/// §8h's tag tracks the dossier's presence in the BASE branch's tree, never
/// the working tree: uncommitted (worktree-only) reads Some(false), a commit
/// on main flips it to Some(true), and a card with no dossier path stays
/// UNKNOWN (None) — the palette renders nothing on anything but `true`.
#[test]
fn feature_rows_marks_landed_only_when_committed_in_base() {
    let fx = fixture();
    let (issue, task_id) = fx.card_mid_step("Invite vetting");
    let rel = issue.dossier_path.clone().expect("dossier born");
    let ids = vec![format!("{}#Plan — 2026-08-07", issue.id)];

    // Worktree-only: the base ref resolves, the path is not in it.
    let rows = fartcode_app_lib::commands::dossiers::feature_rows(&fx.app, &ids);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].landed,
        Some(false),
        "uncommitted is a definitive no"
    );

    // Commit the dossier to main (the fixture's base ref) → landed.
    let root = fx.project_root();
    let landed = root.join(&rel);
    std::fs::create_dir_all(landed.parent().unwrap()).unwrap();
    std::fs::copy(fx.worktree_of(&task_id).join(&rel), &landed).unwrap();
    git_ok(&root, &["add", "."]);
    git_ok(
        &root,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@fartCode.dev",
            "commit",
            "-m",
            "land it",
        ],
    );

    let rows = fartcode_app_lib::commands::dossiers::feature_rows(&fx.app, &ids);
    assert_eq!(rows[0].landed, Some(true), "committed in base is landed");

    // A card with no dossier path: unknown, never a guess.
    let plain = fx.new_issue("No dossier yet");
    let ids = vec![format!("{}#Plan — 2026-08-07", plain.id)];
    let rows = fartcode_app_lib::commands::dossiers::feature_rows(&fx.app, &ids);
    assert_eq!(rows[0].landed, None);
}
