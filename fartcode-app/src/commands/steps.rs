//! Step-engine commands (E18-04, #63) — thin wrappers over
//! [`crate::step_engine`]: the generalized column entry and the queue
//! confirm. The legacy `issue_move`/`issue_dispatch` commands stay for the
//! current UI; the UI wave migrates drags here.
//!
//! UI-thread rule (#80): a NON-async `#[tauri::command]` compiles to
//! `ExecutionContext::Blocking` and runs inline on the IPC thread — the
//! macOS main thread — so the window stops repainting for the duration.
//! Both commands here can reach `launch_step` → `provision_issue_task` →
//! `create_with_provision` → `ensure_worktree` (git fetch, branch create,
//! `worktree add`, best-effort push): unbounded network + subprocess time.
//! Shelves, human gates, queue parks and reattaches cost ~3ms, but the
//! branch is only known once the body runs, so the whole body goes to the
//! blocking pool. `async` alone would not be enough — it would move the
//! stall onto a tokio worker instead of leaving the thread.

use std::sync::Arc;

use tauri::State;

use crate::app::App;
use crate::step_engine::{EnterOutcome, ParkedStepDto};

/// The engine's move: enter a column, then run/queue/nothing per the
/// column's kind and `on_enter`. `position: None` appends.
#[tauri::command]
pub async fn issue_enter_column(
    app: State<'_, Arc<App>>,
    issue_id: String,
    column_id: String,
    position: Option<i64>,
) -> Result<EnterOutcome, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Command path: the returned launch is DELIVERED — the frontend
        // opens the session from this outcome.
        crate::step_engine::enter_column_from_command(&app, &issue_id, &column_id, position)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Fires the parked (queue-mode) step for an issue. Single-shot; typed
/// error when nothing is parked (never parked, already confirmed, or
/// cleared by a drag).
#[tauri::command]
pub async fn step_confirm(
    app: State<'_, Arc<App>>,
    issue_id: String,
) -> Result<EnterOutcome, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || crate::step_engine::confirm_step(&app, &issue_id))
        .await
        .map_err(|e| e.to_string())?
}

/// The project's current parks (E18-09 rehydration): parks live only in
/// the in-memory engine registry and `step:*` events cover live sessions
/// only, so a webview reload loses queued-dot / confirm-overlay state —
/// this query is how the frontend re-seeds it. Read-only: no state
/// mutation, no events. The registry read is cheap, but resolving each
/// park's agent config touches SQLite (issue + column + settings), so the
/// body leaves the IPC thread like the other step commands.
#[tauri::command]
pub async fn step_parked_list(
    app: State<'_, Arc<App>>,
    project_id: String,
) -> Result<Vec<ParkedStepDto>, String> {
    let app = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || crate::step_engine::parked_list(&app, &project_id))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use fartcode_core::events::{EventBus, InternalEvent};
    use fartcode_core::issues::columns::{ColumnKind, ColumnStore, NewColumn, OnEnter, OnSettle};
    use fartcode_core::issues::{Issue, NewIssue};
    use tauri::Manager as _;

    /// In-memory App with one project and the seeded default board (Backlog
    /// / Ready shelves, Quick + In Progress agent steps, In Review gate).
    /// No git, no filesystem — every path exercised here is DB-only.
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

    fn column_id(app: &App, name: &str) -> String {
        ColumnStore::new(app.db.clone())
            .list_for_project("p1")
            .unwrap()
            .into_iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no column named {name}"))
            .id
    }

    fn queue_step(app: &App, name: &str) -> String {
        ColumnStore::new(app.db.clone())
            .create(NewColumn {
                project_id: "p1".into(),
                name: name.into(),
                kind: ColumnKind::AgentStep,
                counts_as_done: false,
                is_landing: false,
                on_enter: Some(OnEnter::Queue),
                on_settle: Some(OnSettle::Hold),
                advance_to: None,
                step_prompt: None,
                step_provider: None,
                step_model: None,
                step_effort: None,
                step_tools: None,
            })
            .unwrap()
            .id
    }

    fn step_events(rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) -> Vec<InternalEvent> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if matches!(
                event,
                InternalEvent::StepLaunch { .. }
                    | InternalEvent::StepQueued { .. }
                    | InternalEvent::StepQueueCleared { .. }
                    | InternalEvent::StepSettled { .. }
            ) {
                out.push(event);
            }
        }
        out
    }

    /// Shelf entry: inert, no launch, and the mirror pointer moved — the
    /// pre-conversion behavior, now observed through the async command.
    #[tokio::test]
    async fn enter_a_shelf_column_is_inert_and_still_moves_the_card() {
        let app = fixture();
        let issue = new_issue(&app, "shelve me");
        let ready = column_id(&app, "Ready");
        let mut rx = app.event_bus.subscribe();
        let tapp = managed(&app);

        let outcome = issue_enter_column(
            tapp.state::<Arc<App>>(),
            issue.id.clone(),
            ready.clone(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(outcome.step, "inert");
        assert!(outcome.launch.is_none());
        assert_eq!(outcome.issue.column_id.as_deref(), Some(ready.as_str()));
        assert!(step_events(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn enter_column_keeps_the_unknown_column_error_verbatim() {
        let app = fixture();
        let issue = new_issue(&app, "nowhere to go");
        let tapp = managed(&app);
        let err = issue_enter_column(tapp.state::<Arc<App>>(), issue.id, "nope".into(), None)
            .await
            .unwrap_err();
        assert_eq!(err, "board column not found: nope");
    }

    #[tokio::test]
    async fn enter_column_keeps_the_unknown_issue_error_verbatim() {
        let app = fixture();
        let ready = column_id(&app, "Ready");
        let tapp = managed(&app);
        let err = issue_enter_column(tapp.state::<Arc<App>>(), "nope".into(), ready, None)
            .await
            .unwrap_err();
        assert_eq!(err, "issue not found: nope");
    }

    #[tokio::test]
    async fn step_confirm_without_a_park_keeps_the_typed_error() {
        let app = fixture();
        let issue = new_issue(&app, "nothing parked");
        let tapp = managed(&app);
        let err = step_confirm(tapp.state::<Arc<App>>(), issue.id.clone())
            .await
            .unwrap_err();
        assert_eq!(err, format!("no parked step for issue {}", issue.id));
    }

    /// Event ordering across three command calls is unchanged: queue-mode
    /// entry parks (`StepQueued`), a later entry elsewhere supersedes it
    /// (`StepQueueCleared`), and the now-empty confirm gives the typed
    /// error instead of a stray launch.
    #[tokio::test]
    async fn queue_park_is_cleared_by_a_later_entry_and_confirm_then_errors() {
        let app = fixture();
        let issue = new_issue(&app, "park then drag");
        let gate = queue_step(&app, "Gate");
        let ready = column_id(&app, "Ready");
        let mut rx = app.event_bus.subscribe();
        let tapp = managed(&app);

        let queued = issue_enter_column(
            tapp.state::<Arc<App>>(),
            issue.id.clone(),
            gate.clone(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(queued.step, "queued");
        assert!(queued.launch.is_none());

        let moved = issue_enter_column(tapp.state::<Arc<App>>(), issue.id.clone(), ready, None)
            .await
            .unwrap();
        assert_eq!(moved.step, "inert");

        let err = step_confirm(tapp.state::<Arc<App>>(), issue.id.clone())
            .await
            .unwrap_err();
        assert_eq!(err, format!("no parked step for issue {}", issue.id));

        let events = step_events(&mut rx);
        assert_eq!(events.len(), 2, "unexpected step events: {events:?}");
        assert!(matches!(
            &events[0],
            InternalEvent::StepQueued { issue_id, column_id, .. }
                if issue_id == &issue.id && column_id == &gate
        ));
        assert!(matches!(
            &events[1],
            InternalEvent::StepQueueCleared { issue_id, column_id, .. }
                if issue_id == &issue.id && column_id == &gate
        ));
    }

    /// Rehydration query (E18-09): park a step, query, and get the same
    /// shape `step:queued` carried — issue/project/column plus the
    /// RESOLVED agent config (column NULL provider → project default).
    /// The query is a pure read: a second call answers identically and
    /// no step events are emitted by either.
    #[tokio::test]
    async fn step_parked_list_returns_the_projects_parks_with_resolved_agent() {
        let app = fixture();
        let issue = new_issue(&app, "park me");
        let gate = queue_step(&app, "Gate");
        let tapp = managed(&app);

        let queued = issue_enter_column(
            tapp.state::<Arc<App>>(),
            issue.id.clone(),
            gate.clone(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(queued.step, "queued");

        let mut rx = app.event_bus.subscribe();
        let parks = step_parked_list(tapp.state::<Arc<App>>(), "p1".into())
            .await
            .unwrap();
        assert_eq!(parks.len(), 1);
        assert_eq!(parks[0].issue_id, issue.id);
        assert_eq!(parks[0].project_id, "p1");
        assert_eq!(parks[0].column_id, gate);
        assert_eq!(parks[0].provider, "claude"); // project default
        assert_eq!(parks[0].model, None);
        assert_eq!(parks[0].effort, None);

        // Pure read: same answer twice, and no events emitted.
        let again = step_parked_list(tapp.state::<Arc<App>>(), "p1".into())
            .await
            .unwrap();
        assert_eq!(again.len(), 1);
        assert!(step_events(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn step_parked_list_is_empty_for_a_project_with_no_parks() {
        let app = fixture();
        let tapp = managed(&app);
        let parks = step_parked_list(tapp.state::<Arc<App>>(), "p1".into())
            .await
            .unwrap();
        assert!(parks.is_empty());
        // Unknown project is an empty list too, never an error.
        let parks = step_parked_list(tapp.state::<Arc<App>>(), "ghost".into())
            .await
            .unwrap();
        assert!(parks.is_empty());
    }

    /// The #80 property: the body leaves the calling thread. The DB
    /// connection mutex is the barrier — no sleeps, every wait bounded.
    /// `step_confirm` on an unparked issue is the safe probe: one DB read,
    /// then the typed error. Nothing here can launch or touch git.
    ///
    /// Falsified: reverting the body to run inline (the pre-#80 shape)
    /// deadlocks this test instead of passing it.
    #[tokio::test]
    async fn step_confirm_leaves_the_calling_thread_before_touching_the_db() {
        let app = fixture();
        let issue = new_issue(&app, "off thread");
        let tapp = managed(&app);
        // The DB guard is held by a helper thread, never across an await
        // here (the connection mutex is non-reentrant, and holding it on an
        // async task is exactly the thing we forbid in command code).
        let hold = DbHold::take(&app);

        let fut = step_confirm(tapp.state::<Arc<App>>(), issue.id.clone());
        tokio::pin!(fut);
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
        assert_eq!(err, format!("no parked step for issue {}", issue.id));
    }

    /// Holds the DB connection mutex on a helper thread until told to let
    /// go. Self-releasing after 10s so a panicking test can never wedge
    /// the run.
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
