//! Tasks integration tests (ticket E2-01 acceptance criteria).
//!
//! All tests use `tempfile::tempdir()` + `:memory:`-style fixtures — never
//! real app data paths.

use std::sync::Arc;

use ade_core::db::{Db, SqliteDb};
use ade_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use ade_core::tasks::{
    CreateTaskOptions, DbTaskStore, LinkedIssue, TaskStatus, TaskStore, TaskType,
};

struct Fixture {
    _tmp: tempfile::TempDir,
    db: Arc<SqliteDb>,
    store: DbTaskStore,
    bus: Arc<BroadcastEventBus>,
    project_id: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = SqliteDb::init(Some(db_path.to_str().unwrap())).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        let store = DbTaskStore::new(db.clone(), bus.clone());
        db.conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'demo', '/tmp/demo')",
                [],
            )
            .unwrap();
        Self {
            _tmp: tmp,
            db,
            store,
            bus,
            project_id: "p1".into(),
        }
    }

    fn create(&self, name: &str) -> ade_core::tasks::Task {
        self.store
            .create(CreateTaskOptions::new(&self.project_id, name))
            .unwrap()
    }
}

fn count(db: &Arc<SqliteDb>, table: &str) -> i64 {
    db.conn()
        .lock()
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Acceptance 1: create writes task + workspace + initial conversation
// atomically; failure rolls back
// ---------------------------------------------------------------------------

#[test]
fn test_create_writes_task_workspace_conversation_atomically() {
    let fx = Fixture::new();
    let task = fx
        .store
        .create(
            CreateTaskOptions::new(&fx.project_id, "fix auth")
                .with_initial_conversation("Fix auth", Some("claude")),
        )
        .unwrap();

    assert_eq!(task.name, "fix auth");
    assert_eq!(
        task.status,
        TaskStatus::InProgress,
        "default initial status"
    );
    assert_eq!(task.created_by, "user");
    assert_eq!(task.r#type, TaskType::Task);
    assert!(task.status_changed_at.is_some());
    assert!(task.last_interacted_at.is_some());
    assert!(
        task.workspace_id.is_some(),
        "workspace row must be attached"
    );

    assert_eq!(count(&fx.db, "tasks"), 1);
    assert_eq!(
        count(&fx.db, "workspaces"),
        1,
        "workspace row created in same tx"
    );
    assert_eq!(
        count(&fx.db, "conversations"),
        1,
        "initial conversation in same tx"
    );

    // Conversation is linked and marked initial.
    let (task_id, is_initial): (String, i64) = fx
        .db
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT task_id, is_initial_conversation FROM conversations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(task_id, task.id);
    assert_eq!(is_initial, 1);
}

#[test]
fn test_create_rolls_back_on_failure() {
    let fx = Fixture::new();
    let err = fx
        .store
        .create(CreateTaskOptions::new("no-such-project", "doomed"))
        .unwrap_err();
    assert!(err.to_string().contains("project not found"));

    assert_eq!(count(&fx.db, "tasks"), 0, "no task row on failure");
    assert_eq!(
        count(&fx.db, "workspaces"),
        0,
        "no workspace row on failure"
    );
    assert_eq!(
        count(&fx.db, "conversations"),
        0,
        "no conversation row on failure"
    );
}

#[test]
fn test_create_rolls_back_mid_transaction() {
    let fx = Fixture::new();
    // Force the conversation insert (the LAST statement in the create tx) to
    // fail, so the task + workspace inserts must roll back with it.
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_conversation AFTER INSERT ON conversations
             BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();

    let err = fx
        .store
        .create(
            CreateTaskOptions::new(&fx.project_id, "doomed").with_initial_conversation("x", None),
        )
        .unwrap_err();
    assert!(err.to_string().contains("boom"));

    assert_eq!(count(&fx.db, "tasks"), 0, "task insert must roll back");
    assert_eq!(
        count(&fx.db, "workspaces"),
        0,
        "workspace insert must roll back"
    );
    assert_eq!(
        count(&fx.db, "conversations"),
        0,
        "conversation insert must roll back"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 2: status transitions persist and update status_changed_at
// ---------------------------------------------------------------------------

#[test]
fn test_status_transition_persists_and_updates_status_changed_at() {
    let fx = Fixture::new();
    let task = fx.create("fix auth");
    // Pin status_changed_at to a known old value so the update is observable
    // (datetime('now') resolution is 1s).
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET status_changed_at = '2020-01-01 00:00:00' WHERE id = ?1",
            [&task.id],
        )
        .unwrap();

    let updated = fx.store.update_status(&task.id, TaskStatus::Done).unwrap();
    assert_eq!(updated.status, TaskStatus::Done);
    assert_ne!(
        updated.status_changed_at.as_deref(),
        Some("2020-01-01 00:00:00"),
        "status_changed_at must be bumped"
    );

    // Same-status update is a no-op (status_changed_at untouched).
    let reupdated = fx.store.update_status(&task.id, TaskStatus::Done).unwrap();
    assert_eq!(reupdated.status_changed_at, updated.status_changed_at);
}

#[test]
fn test_unknown_task_status_ops_error() {
    let fx = Fixture::new();
    let err = fx
        .store
        .update_status("nope", TaskStatus::Done)
        .unwrap_err();
    assert!(err.to_string().contains("task not found"));
}

// ---------------------------------------------------------------------------
// Acceptance 3: automation-run tasks
// ---------------------------------------------------------------------------

#[test]
fn test_automation_run_task_fields() {
    let fx = Fixture::new();
    let task = fx
        .store
        .create(CreateTaskOptions::new(&fx.project_id, "auto task").with_automation_run("run-42"))
        .unwrap();
    assert_eq!(task.r#type, TaskType::AutomationRun);
    assert_eq!(task.automation_run_id.as_deref(), Some("run-42"));

    let fetched = fx.store.get(&task.id).unwrap().unwrap();
    assert_eq!(fetched.r#type, TaskType::AutomationRun);
}

// ---------------------------------------------------------------------------
// Archive / restore / delete
// ---------------------------------------------------------------------------

#[test]
fn test_archive_and_restore() {
    let fx = Fixture::new();
    let task = fx.create("archivable");
    assert!(task.archived_at.is_none());

    let archived = fx.store.archive(&task.id).unwrap();
    assert!(archived.archived_at.is_some());
    assert_eq!(
        fx.store.get(&task.id).unwrap().unwrap().archived_at,
        archived.archived_at
    );

    let restored = fx.store.restore(&task.id).unwrap();
    assert!(restored.archived_at.is_none());
}

#[test]
fn test_delete_cascades_to_conversations_and_terminals() {
    let fx = Fixture::new();
    let task = fx
        .store
        .create(
            CreateTaskOptions::new(&fx.project_id, "cascade")
                .with_initial_conversation("cascade", None),
        )
        .unwrap();
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO terminals (id, project_id, task_id, name) VALUES ('t1', 'p1', ?1, 'shell')",
            [&task.id],
        )
        .unwrap();

    fx.store.delete(&task.id).unwrap();
    assert!(fx.store.get(&task.id).unwrap().is_none());
    assert_eq!(count(&fx.db, "conversations"), 0, "conversations cascade");
    assert_eq!(count(&fx.db, "terminals"), 0, "terminals cascade");
}

#[test]
fn test_pinned_and_linked_issue() {
    let fx = Fixture::new();
    let task = fx
        .store
        .create(
            CreateTaskOptions::new(&fx.project_id, "with issue").with_linked_issue(LinkedIssue {
                provider: "github".into(),
                url: "https://github.com/x/y/issues/1".into(),
                title: "Big bug".into(),
                identifier: "x/y#1".into(),
                display_identifier: None,
                description: None,
                branch_name: None,
                status: None,
            }),
        )
        .unwrap();
    assert_eq!(task.linked_issue.as_ref().unwrap().identifier, "x/y#1");

    let pinned = fx.store.set_pinned(&task.id, true).unwrap();
    assert!(pinned.is_pinned);
    assert!(!fx.store.set_pinned(&task.id, false).unwrap().is_pinned);
}

#[test]
fn test_list_by_project_orders_by_recency() {
    let fx = Fixture::new();
    fx.create("first");
    let second = fx.create("second");
    // datetime('now') has 1s resolution — bump recency deterministically.
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE tasks SET updated_at = '2026-08-04 00:00:00' WHERE id = ?1",
            [&second.id],
        )
        .unwrap();
    let tasks = fx.store.list_by_project(&fx.project_id).unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id, second.id, "most recent first");
}

// ---------------------------------------------------------------------------
// Provision fast-path + events
// ---------------------------------------------------------------------------

#[test]
fn test_provision_is_idempotent_and_emits() {
    let fx = Fixture::new();
    let task = fx.create("provision me");
    let mut rx = fx.bus.subscribe();

    fx.store.provision(&task.id).unwrap();
    fx.store.provision(&task.id).unwrap(); // re-provision is a no-op error-wise

    match rx.try_recv().unwrap() {
        InternalEvent::TaskProvisioned { id, .. } => assert_eq!(id, task.id),
        other => panic!("expected TaskProvisioned, got {other:?}"),
    }
}

#[test]
fn test_lifecycle_events_are_emitted() {
    let fx = Fixture::new();
    let mut rx = fx.bus.subscribe();
    let task = fx
        .store
        .create(
            CreateTaskOptions::new(&fx.project_id, "eventful")
                .with_initial_conversation("Eventful", Some("claude")),
        )
        .unwrap();

    match rx.try_recv().unwrap() {
        InternalEvent::TaskCreated { id, .. } => assert_eq!(id, task.id),
        other => panic!("expected TaskCreated, got {other:?}"),
    }
    match rx.try_recv().unwrap() {
        InternalEvent::ConversationCreated { task_id, .. } => assert_eq!(task_id, task.id),
        other => panic!("expected ConversationCreated, got {other:?}"),
    }

    fx.store
        .update_status(&task.id, TaskStatus::Review)
        .unwrap();
    match rx.try_recv().unwrap() {
        InternalEvent::TaskStatusChanged { id, new_status, .. } => {
            assert_eq!(id, task.id);
            assert_eq!(new_status, "review");
        }
        other => panic!("expected TaskStatusChanged, got {other:?}"),
    }

    fx.store.archive(&task.id).unwrap();
    match rx.try_recv().unwrap() {
        InternalEvent::TaskArchived { id } => assert_eq!(id, task.id),
        other => panic!("expected TaskArchived, got {other:?}"),
    }

    fx.store.delete(&task.id).unwrap();
    match rx.try_recv().unwrap() {
        InternalEvent::TaskDeleted { id } => assert_eq!(id, task.id),
        other => panic!("expected TaskDeleted, got {other:?}"),
    }
}

#[test]
fn test_trait_object_usable() {
    let fx = Fixture::new();
    let store: Arc<dyn TaskStore> = Arc::new(fx.store);
    let task = store
        .create(CreateTaskOptions::new("p1", "via trait"))
        .unwrap();
    assert_eq!(store.get(&task.id).unwrap().unwrap().id, task.id);
}
