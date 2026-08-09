//! Issue commands (E17-01, #55; ARCHITECTURE.md §13, ADR-0032) — thin CRUD
//! over [`IssueStore`] for the project board: lanes, blocked-by edges, and
//! the dispatch link the board dispatch engine (E17-03) drives.
//!
//! UI-thread rule (#80): a NON-async `#[tauri::command]` compiles to
//! `ExecutionContext::Blocking` — its body is inlined into the invoke
//! handler and runs on the IPC thread, which on macOS is the main thread.
//! Blocking there stalls the NSRunLoop and the window stops repainting
//! (beachball, not spinner). The two commands here that can reach the
//! network or a git/gh subprocess — [`issue_dispatch`] and
//! [`project_github_issues`] — are therefore `async` AND push their whole
//! body onto the blocking pool via `spawn_blocking`; `async` alone would
//! only move the stall onto a tokio worker. The remaining commands are
//! single indexed SQLite statements against the in-process connection and
//! stay synchronous on purpose.

use std::sync::Arc;

use serde::Deserialize;
use tauri::State;

use fartcode_core::issues::{Issue, IssuePatch, Lane, NewIssue};
use fartcode_core::projects::ProjectStore;

use crate::app::App;

/// Request body for [`issue_create`] (frontend sends one object).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIssueRequest {
    pub project_id: String,
    pub title: String,
    pub body: Option<String>,
    pub acceptance: Option<Vec<String>>,
    pub lane: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub prd_path: Option<String>,
    pub prd_section: Option<String>,
}

/// Request body for [`issue_update`]. Missing = leave alone; explicit
/// `null` on a nullable field clears it (serde: absent → `None`,
/// `null` → `Some(None)`, value → `Some(Some(v))`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateIssueRequest {
    pub title: Option<String>,
    pub body: Option<Option<String>>,
    pub acceptance: Option<Vec<String>>,
    pub provider: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub prd_path: Option<Option<String>>,
    pub prd_section: Option<Option<String>>,
}

/// Manual add (E18-06): the card lands on the project's `is_landing`
/// column, wherever the flag sits — no lane hardcode anywhere on this
/// path. `request.lane` is still accepted for wire compatibility (the
/// lane board sends `"backlog"`) but is validated-and-ignored; the UI
/// wave drops the field.
#[tauri::command]
pub fn issue_create(
    app: State<'_, Arc<App>>,
    request: CreateIssueRequest,
) -> Result<Issue, String> {
    // Validate-only: a malformed lane string still errors, so the wire
    // contract is unchanged for callers that send one.
    request
        .lane
        .as_deref()
        .map(Lane::parse)
        .transpose()
        .map_err(String::from)?;
    app.issues
        .create(NewIssue {
            project_id: request.project_id,
            title: request.title,
            body: request.body,
            acceptance: request.acceptance.unwrap_or_default(),
            lane: None,
            provider: request.provider,
            model: request.model,
            prd_path: request.prd_path,
            prd_section: request.prd_section,
            external_ref: None,
        })
        .map_err(String::from)
}

/// Issues for a project in board render order (lane rank, position), with
/// derived blocked status and blocker hover lists attached.
#[tauri::command]
pub fn issue_list(app: State<'_, Arc<App>>, project_id: String) -> Result<Vec<Issue>, String> {
    app.issues
        .list_for_project(&project_id)
        .map_err(String::from)
}

#[tauri::command]
pub fn issue_update(
    app: State<'_, Arc<App>>,
    issue_id: String,
    patch: UpdateIssueRequest,
) -> Result<Issue, String> {
    app.issues
        .update(
            &issue_id,
            IssuePatch {
                title: patch.title,
                body: patch.body,
                acceptance: patch.acceptance,
                provider: patch.provider,
                model: patch.model,
                prd_path: patch.prd_path,
                prd_section: patch.prd_section,
            },
        )
        .map_err(String::from)
}

/// Lane move (board drag). `position: None` appends to the lane end.
/// Blocked-dispatch confirmation is a frontend concern (ADR-0032); any
/// transition is permitted here.
///
/// E18-04 item 5: a manual drag overrides a parked (queue-mode) step —
/// unless the drag stays in the parked column. The move itself routes
/// through the enter primitive inside the store.
#[tauri::command]
pub fn issue_move(
    app: State<'_, Arc<App>>,
    issue_id: String,
    lane: String,
    position: Option<i64>,
) -> Result<Issue, String> {
    let lane = Lane::parse(&lane).map_err(String::from)?;
    // Fail-closed ordering (#66 fix round): since the authority flip,
    // `move_to` can REFUSE (target lane's seeded column deleted). The
    // engine side effects (epoch reset, park drop) are destructive and
    // must not run for a move that never happened — so capture the
    // pre-move lane, run the fallible move FIRST, and only on success
    // apply the drag-overrides-park semantics keyed on the PRE-move lane
    // (a same-lane reorder touches nothing).
    let pre_lane = app
        .issues
        .get(&issue_id)
        .map_err(String::from)?
        .map(|i| i.lane);
    let moved = app
        .issues
        .move_to(&issue_id, lane, position)
        .map_err(String::from)?;
    if pre_lane != Some(lane) {
        crate::step_engine::on_lane_move_committed(&app, &issue_id, lane);
    }
    Ok(moved)
}

#[tauri::command]
pub fn issue_delete(app: State<'_, Arc<App>>, issue_id: String) -> Result<(), String> {
    app.issues.delete(&issue_id).map_err(String::from)?;
    // E18-04 fix round (finding 4): sweep the dead card's park (with the
    // cleared event) and its launch-registry traces.
    crate::step_engine::on_issue_deleted(&app, &issue_id);
    Ok(())
}

/// `issue_id` becomes blocked by `blocked_by_id`. Cycle/cross-project
/// rejections surface as errors for the card-detail UI.
#[tauri::command]
pub fn issue_link(
    app: State<'_, Arc<App>>,
    issue_id: String,
    blocked_by_id: String,
) -> Result<Issue, String> {
    app.issues
        .add_dependency(&issue_id, &blocked_by_id)
        .map_err(String::from)
}

#[tauri::command]
pub fn issue_unlink(
    app: State<'_, Arc<App>>,
    issue_id: String,
    blocked_by_id: String,
) -> Result<Issue, String> {
    app.issues
        .remove_dependency(&issue_id, &blocked_by_id)
        .map_err(String::from)
}

/// Drag-into-In-Progress (E17-03, #57): creates the linked task (worktree
/// + issue-derived name + prompt packet) or reattaches to the live linked
/// one. The frontend launches the agent terminal with the returned prompt.
///
/// Off the UI thread (#80): the first dispatch of a card provisions a
/// worktree (`create_with_provision` → `ensure_worktree`: git fetch, branch
/// create, `worktree add`, best-effort push) — unbounded network + git
/// subprocess time. The reattach branch is two DB reads, but the command
/// cannot know which branch it will take before it runs, so the whole body
/// goes to the blocking pool.
#[tauri::command]
pub async fn issue_dispatch(
    app: State<'_, Arc<App>>,
    issue_id: String,
) -> Result<crate::dispatch::DispatchOutcome, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || issue_dispatch_blocking(&app, &issue_id))
        .await
        .map_err(|e| e.to_string())?
}

/// [`issue_dispatch`]'s body, verbatim — a plain function so the blocking
/// pool runs it and the tests exercise the same code the command does.
fn issue_dispatch_blocking(
    app: &App,
    issue_id: &str,
) -> Result<crate::dispatch::DispatchOutcome, String> {
    // Fail-closed ordering (#66 fix round): resolve every failable
    // precondition BEFORE the destructive park drop — a refused dispatch
    // (issue gone, seeded In Progress column deleted) must leave the
    // pending confirm gate intact.
    crate::dispatch::issue_dispatch_precheck(app, issue_id)?;
    // E18-04 item 5: a dispatch entry supersedes any parked step (the
    // dispatch launches an agent now; the pending confirm is moot).
    crate::step_engine::drop_parked_step(app, issue_id);
    let outcome = crate::dispatch::issue_dispatch_core(app, issue_id)?;
    // Final round item 3: a real dispatch entry (not a reattach-focus)
    // is a user gesture — new settle epoch.
    if !outcome.reattached {
        crate::step_engine::begin_entry_epoch(app, issue_id);
    }
    Ok(outcome)
}

/// Open GitHub issues of the project's checkout (E17 dogfood), fetched via
/// the gh CLI. Errors name the remedy (gh missing, not authed).
///
/// Off the UI thread (#80): `find_on_path("gh")` plus an un-timed
/// `gh issue list` — a process spawn and a network round trip.
#[tauri::command]
pub async fn project_github_issues(
    app: State<'_, Arc<App>>,
    project_id: String,
) -> Result<Vec<fartcode_git::issues::GitHubIssue>, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || project_github_issues_blocking(&app, &project_id))
        .await
        .map_err(|e| e.to_string())?
}

/// [`project_github_issues`]'s body, verbatim (see [`issue_dispatch_blocking`]).
fn project_github_issues_blocking(
    app: &App,
    project_id: &str,
) -> Result<Vec<fartcode_git::issues::GitHubIssue>, String> {
    let project = app
        .projects
        .get(project_id)
        .map_err(String::from)?
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    fartcode_git::issues::list_github_issues(&project.path).map_err(String::from)
}

/// Import payload (one object over the wire, mirrors the GitHub fields).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGithubIssueRequest {
    pub project_id: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub body: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
}

/// Imports a GitHub issue as a native board card with EVERYTHING mapped:
/// title (`#N` prefix), checkbox lines → acceptance criteria, body kept,
/// and labels/assignees/milestone folded into the body so nothing is lost.
/// The GitHub URL survives only as the internal dedupe key — once migrated,
/// the card is a native issue with no link back. Idempotent.
#[tauri::command]
pub fn issue_import_github(
    app: State<'_, Arc<App>>,
    request: ImportGithubIssueRequest,
) -> Result<Issue, String> {
    let gh = fartcode_git::issues::GitHubIssue {
        number: request.number,
        title: request.title,
        url: request.url.clone(),
        body: request.body,
        labels: request.labels,
        assignees: request.assignees,
        milestone: request.milestone,
        created_at: None,
    };
    let mapped = fartcode_git::issues::map_issue_fields(&gh);
    app.issues
        .create(NewIssue {
            project_id: request.project_id,
            title: mapped.title,
            body: mapped.body,
            acceptance: mapped.acceptance,
            // E18-06: no lane hardcode — the store lands the card on the
            // project's is_landing column.
            lane: None,
            provider: None,
            model: None,
            prd_path: None,
            prd_section: None,
            external_ref: Some(request.url), // dedupe key only — never surfaced
        })
        .map_err(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use fartcode_core::events::{EventBus, InternalEvent};
    use tauri::Manager as _;

    /// In-memory App with one project and the seeded default board — the
    /// same shape `step_engine`'s tests use. No git, no filesystem.
    fn fixture() -> Arc<App> {
        let app = App::init(Some(":memory:")).unwrap();
        {
            let conn = app.db.conn().lock().unwrap();
            conn.execute_batch(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'proj', '/tmp/proj');",
            )
            .unwrap();
            fartcode_core::issues::columns::seed_default_columns(&conn, "p1").unwrap();
        }
        app
    }

    /// A Tauri app managing the `Arc<App>` so the commands can be called
    /// with a real `State` — the caller keeps it alive while borrowing.
    fn managed(app: &Arc<App>) -> tauri::App<tauri::test::MockRuntime> {
        let tapp = tauri::test::mock_app();
        tapp.handle().manage(app.clone());
        tapp
    }

    fn new_issue(app: &App, title: &str) -> Issue {
        app.issues
            .create(NewIssue {
                project_id: "p1".into(),
                title: title.into(),
                body: None,
                acceptance: Vec::new(),
                lane: None,
                provider: None,
                model: None,
                prd_path: None,
                prd_section: None,
                external_ref: None,
            })
            .unwrap()
    }

    fn with_task(app: &App, task_id: &str) {
        app.db
            .conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO tasks (id, project_id, name, status)
                 VALUES (?1, 'p1', 't', 'in_progress')",
                [task_id],
            )
            .unwrap();
    }

    fn task_count(app: &App) -> i64 {
        app.db
            .conn()
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap()
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) -> Vec<InternalEvent> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            out.push(event);
        }
        out
    }

    /// Fail-closed ordering (#66 fix round, defect 2): a dispatch the
    /// core would REFUSE (seeded In Progress column deleted — legal
    /// since the flip) must not destroy the pending confirm gate. The
    /// park drop runs only after the precheck passes.
    #[tokio::test]
    async fn refused_dispatch_keeps_the_parked_confirm_gate() {
        use fartcode_core::issues::columns::{ColumnPatch, ColumnStore, OnEnter};

        let app = fixture();
        let issue = new_issue(&app, "parked card");
        let col_store = ColumnStore::new(app.db.clone());
        let columns = col_store.list_for_project("p1").unwrap();
        let quick = columns.iter().find(|c| c.name == "Quick").unwrap();
        let in_progress = columns
            .iter()
            .find(|c| c.seed_lane.as_deref() == Some("in_progress"))
            .unwrap();
        // Park the card: Quick becomes queue-mode, then enter it.
        col_store
            .update(
                &quick.id,
                ColumnPatch {
                    on_enter: Some(OnEnter::Queue),
                    ..Default::default()
                },
            )
            .unwrap();
        crate::step_engine::enter_column_from_command(&app, &issue.id, &quick.id, None).unwrap();
        assert!(app.steps.peek_park(&issue.id).is_some());
        // Delete the seeded In Progress column (empty → legal since #66).
        col_store.delete(&in_progress.id).unwrap();

        let mut rx = app.event_bus.subscribe();
        let tapp = managed(&app);
        let err = issue_dispatch(tapp.state::<Arc<App>>(), issue.id.clone())
            .await
            .unwrap_err();
        assert!(
            err.contains("no seeded In Progress column"),
            "typed refusal expected, got: {err}"
        );
        // No task, no worktree, and — the defect — the park SURVIVES:
        // step_confirm still fires the gated step.
        assert_eq!(task_count(&app), 0);
        assert!(app.steps.peek_park(&issue.id).is_some());
        assert!(
            !drain(&mut rx)
                .iter()
                .any(|e| matches!(e, InternalEvent::StepQueueCleared { .. })),
            "a refused dispatch must not clear the queue gate"
        );
        // The confirm gate still works: confirm takes the park and
        // proceeds to launch — in this repo-less fixture it then fails
        // at git provisioning, but it must NOT be the "no parked step"
        // refusal the defect produced.
        let confirm_err = crate::step_engine::confirm_step(&app, &issue.id).unwrap_err();
        assert!(
            !confirm_err.contains("no parked step"),
            "the confirm gate was destroyed: {confirm_err}"
        );
    }

    /// Error text is part of the wire contract (the board surfaces it) —
    /// the async command must produce it byte-for-byte, and the join-error
    /// mapping must not swallow it.
    #[tokio::test]
    async fn dispatch_keeps_the_missing_issue_error_verbatim() {
        let app = fixture();
        let tapp = managed(&app);
        let err = issue_dispatch(tapp.state::<Arc<App>>(), "nope".into())
            .await
            .unwrap_err();
        assert_eq!(err, "issue not found: nope");
    }

    /// The reattach branch: same outcome fields as before the conversion
    /// (empty prompt + provider, `reattached: true`), no lane change, no
    /// second task, and — crucially — not one event on the bus.
    #[tokio::test]
    async fn dispatch_reattaches_to_the_live_linked_task_with_no_side_effects() {
        let app = fixture();
        let issue = new_issue(&app, "reattach me");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let lane_before = app.issues.get(&issue.id).unwrap().unwrap().lane;

        let mut rx = app.event_bus.subscribe();
        let tapp = managed(&app);
        let outcome = issue_dispatch(tapp.state::<Arc<App>>(), issue.id.clone())
            .await
            .unwrap();

        assert!(outcome.reattached);
        assert_eq!(outcome.task.id, "t1");
        assert_eq!(outcome.prompt, "");
        assert_eq!(outcome.provider, "");
        assert_eq!(outcome.issue.lane, lane_before);
        // No provisioning: no worktree, no second task row.
        assert_eq!(task_count(&app), 1);
        assert!(drain(&mut rx).is_empty(), "reattach must emit nothing");
    }

    /// The serialized shape the frontend reads is unchanged by the
    /// conversion (camelCase keys, same fields).
    #[tokio::test]
    async fn dispatch_outcome_serializes_with_the_same_wire_shape() {
        let app = fixture();
        let issue = new_issue(&app, "wire shape");
        with_task(&app, "t1");
        app.issues.set_linked_task(&issue.id, Some("t1")).unwrap();
        let tapp = managed(&app);
        let outcome = issue_dispatch(tapp.state::<Arc<App>>(), issue.id.clone())
            .await
            .unwrap();
        let value = serde_json::to_value(&outcome).unwrap();
        assert!(value.get("task").is_some());
        assert!(value.get("issue").is_some());
        assert_eq!(value["prompt"], "");
        assert_eq!(value["provider"], "");
        assert_eq!(value["reattached"], true);
    }

    #[tokio::test]
    async fn github_issues_keeps_the_missing_project_error_verbatim() {
        let app = fixture();
        let tapp = managed(&app);
        let err = project_github_issues(tapp.state::<Arc<App>>(), "nope".into())
            .await
            .unwrap_err();
        assert_eq!(err, "project not found: nope");
    }

    /// The point of #80: the body must LEAVE the calling thread. Proven
    /// without a sleep — the DB connection mutex is the barrier. While a
    /// helper thread holds it, the command's first statement cannot run; if
    /// the body were inlined (the old non-async command) the caller would
    /// be stuck inside it. Instead the future is merely pending, so a
    /// bounded `timeout` elapses; releasing the guard lets it finish with
    /// the exact same error. Every wait here is bounded.
    ///
    /// `project_github_issues` is the safe probe: its only work before the
    /// error return is one DB read — no `gh`, no network.
    #[tokio::test]
    async fn github_issues_leaves_the_calling_thread_before_touching_the_db() {
        let app = fixture();
        let tapp = managed(&app);
        let hold = DbHold::take(&app);

        let fut = project_github_issues(tapp.state::<Arc<App>>(), "nope".into());
        tokio::pin!(fut);
        // Bounded: 200ms is plenty for the blocking pool to pick the
        // closure up and park on the mutex.
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut fut)
                .await
                .is_err(),
            "command completed inline — its body never left the caller's thread"
        );

        hold.release();
        let err = tokio::time::timeout(Duration::from_secs(5), &mut fut)
            .await
            .expect("command did not finish after the DB was released")
            .unwrap_err();
        assert_eq!(err, "project not found: nope");
    }

    /// Holds the DB connection mutex on a helper thread until told to let
    /// go — so no guard is ever held across an await on the async side.
    /// Self-releasing after 10s so a panicking test can never wedge the run.
    struct DbHold {
        release: std::sync::mpsc::Sender<()>,
        thread: std::thread::JoinHandle<()>,
    }

    impl DbHold {
        fn take(app: &Arc<App>) -> Self {
            let (release, wait) = std::sync::mpsc::channel::<()>();
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
            let db = app.db.clone();
            let thread = std::thread::spawn(move || {
                let _guard = db.conn().lock().unwrap();
                let _ = ready_tx.send(());
                let _ = wait.recv_timeout(Duration::from_secs(10));
            });
            ready_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("holder thread never took the DB lock");
            Self { release, thread }
        }

        fn release(self) {
            let _ = self.release.send(());
            self.thread.join().unwrap();
        }
    }
}
