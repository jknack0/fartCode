//! E2-11-5 acceptance: end-to-end ACP conversation through the app-layer
//! `AcpRuntime` (fake adapter binary), the provider-decision gate keeping
//! non-ACP conversations byte-identical on the TUI path, and task-deletion
//! teardown stopping running sessions.
//!
//! NOTE: the fake adapter binary is built by `cargo test --workspace`
//! (the merge gate); it lives in the shared target directory.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use fartcode_acp::client::SessionUpdateEvent;
use fartcode_acp::session::{LiveModels, PermissionRequestedEvent, SessionEvents};
use fartcode_app_lib::acp_runtime::AcpRuntime;
use fartcode_core::conversations::model::ConversationType;
use fartcode_core::conversations::{
    ConversationStore, CreateConversationParams, DbConversationStore,
};
use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::events::BroadcastEventBus;
use fartcode_core::provider_accounts::ProviderAccountStore;
use fartcode_core::tasks::DbTaskStore;
use parking_lot::Mutex;

fn fake_adapter_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let bin = manifest
        .parent()
        .unwrap()
        .join("target")
        .join("debug")
        .join("fake_acp_adapter");
    assert!(
        bin.exists(),
        "fake_acp_adapter not built — run `cargo test --workspace` (the merge gate)"
    );
    bin
}

#[derive(Default)]
struct RecordingEvents {
    updates: Mutex<Vec<SessionUpdateEvent>>,
    snapshots: Mutex<Vec<LiveModels>>,
    permissions: Mutex<Vec<PermissionRequestedEvent>>,
}

impl SessionEvents for RecordingEvents {
    fn update(&self, _conversation_id: &str, event: &SessionUpdateEvent) {
        self.updates.lock().push(event.clone());
    }
    fn transcript_changed(&self, _conversation_id: &str, models: &LiveModels) {
        self.snapshots.lock().push(models.clone());
    }
    fn permission_requested(&self, _conversation_id: &str, event: &PermissionRequestedEvent) {
        self.permissions.lock().push(event.clone());
    }
}

struct Fixture {
    // Lifetime keeps: the in-memory DB backs the runtime, and the worktree
    // directory must exist on disk for the adapter's cwd while the session
    // runs. Both are held, not read.
    #[allow(dead_code)]
    db: Arc<SqliteDb>,
    conversations: Arc<DbConversationStore>,
    events: Arc<RecordingEvents>,
    runtime: Arc<AcpRuntime>,
    /// Adapter-resolver invocations — one per spawn attempt. The start
    /// lock's whole job is to keep this at 1 for overlapping starts.
    spawns: Arc<AtomicUsize>,
    #[allow(dead_code)]
    worktree: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let db = SqliteDb::init_in_memory().unwrap();
        let bus = Arc::new(BroadcastEventBus::new(64));
        let worktree = tempfile::tempdir().unwrap();
        let worktree_path = worktree.path().to_str().unwrap().replace('\'', "''");

        // Project + task with a worktree workspace (the adapter's cwd).
        db.conn()
            .lock()
            .unwrap()
            .execute_batch(&format!(
                "INSERT INTO projects (id, name, path) VALUES ('proj-1', 'demo', '/tmp/demo');
                 INSERT INTO workspaces (id, type, kind, path)
                     VALUES ('ws-1', 'local', 'worktree', '{worktree_path}');
                 INSERT INTO tasks (id, project_id, name, status, workspace_id)
                     VALUES ('task-1', 'proj-1', 'fix', 'in_progress', 'ws-1');"
            ))
            .unwrap();

        let conversations = Arc::new(DbConversationStore::new(db.clone(), bus.clone()));
        let tasks = Arc::new(DbTaskStore::new(db.clone(), bus.clone()));
        let provider_accounts = Arc::new(ProviderAccountStore::new(db.clone()));
        let events = Arc::new(RecordingEvents::default());
        let adapter = fake_adapter_path();
        let spawns = Arc::new(AtomicUsize::new(0));
        let spawn_counter = spawns.clone();
        let runtime = AcpRuntime::new(
            conversations.clone(),
            tasks,
            db.clone(),
            provider_accounts,
            events.clone(),
            Arc::new(move |_provider_id: &str| {
                spawn_counter.fetch_add(1, Ordering::SeqCst);
                Ok(adapter.clone())
            }),
        );
        Self {
            db,
            conversations,
            events,
            runtime,
            spawns,
            worktree,
        }
    }

    fn create_conversation(&self, id: &str, r#type: ConversationType, provider: &str) {
        self.conversations
            .create(CreateConversationParams {
                id: Some(id.into()),
                project_id: "proj-1".into(),
                task_id: Some("task-1".into()),
                scope: None,
                provider: Some(provider.into()),
                title: "E2E".into(),
                auto_approve: None,
                model: None,
                initial_prompt: None,
                initial_queue: None,
                is_initial_conversation: false,
                r#type: Some(r#type),
            })
            .unwrap();
    }
}

async fn eventually(f: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition never became true"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// -- acceptance 1: end-to-end with the fake adapter -----------------------------

#[tokio::test]
async fn end_to_end_acp_conversation_with_fake_adapter() {
    let fx = Fixture::new();
    fx.create_conversation("conv-e2e", ConversationType::Acp, "claude");

    // Start → adapter spawned, session id returned AND persisted.
    let outcome = fx.runtime.start("conv-e2e").await.unwrap();
    assert_eq!(outcome.session_id, "fake-session-1");
    assert!(!outcome.resumed);
    let conv = fx.conversations.get("conv-e2e").unwrap().unwrap();
    assert_eq!(
        conv.session_id.as_deref(),
        Some("fake-session-1"),
        "session id must persist through the DbSessionIdStore seam"
    );

    // Start is idempotent while running.
    let again = fx.runtime.start("conv-e2e").await.unwrap();
    assert_eq!(again.session_id, "fake-session-1");
    assert!(!again.resumed);

    // Send a prompt → turn lands in the live models (reduced transcript).
    let response = fx
        .runtime
        .send_prompt("conv-e2e", "rich", None)
        .await
        .unwrap();
    assert!(!response.queued, "idle prompt runs immediately");
    eventually(|| {
        fx.runtime
            .live_models("conv-e2e")
            .is_some_and(|m| m.committed.len() == 1 && m.active_turn.is_none())
    })
    .await;
    let models = fx.runtime.live_models("conv-e2e").unwrap();
    assert_eq!(models.title.as_deref(), Some("Rich turn"));
    assert_eq!(
        models
            .config
            .model_options
            .as_ref()
            .unwrap()
            .selected
            .as_deref(),
        Some("fake-1")
    );
    assert!(
        !fx.events.snapshots.lock().is_empty(),
        "acp:transcript fired"
    );
    assert!(!fx.events.updates.lock().is_empty(), "acp:update fired");

    // Permission round-trip through the runtime.
    let prompt = {
        let runtime = Arc::clone(&fx.runtime);
        tokio::spawn(async move { runtime.send_prompt("conv-e2e", "tool time", None).await })
    };
    eventually(|| !fx.events.permissions.lock().is_empty()).await;
    let request = fx.events.permissions.lock()[0].clone();
    fx.runtime
        .resolve_permission("conv-e2e", &request.request_id, "allow-once")
        .unwrap();
    prompt.await.unwrap().unwrap();
    eventually(|| {
        fx.runtime
            .live_models("conv-e2e")
            .is_some_and(|m| m.committed.len() == 2)
    })
    .await;

    // Cancel an in-flight turn.
    let hanging = {
        let runtime = Arc::clone(&fx.runtime);
        tokio::spawn(async move { runtime.send_prompt("conv-e2e", "hang please", None).await })
    };
    eventually(|| {
        fx.runtime
            .live_models("conv-e2e")
            .is_some_and(|m| m.active_turn.is_some())
    })
    .await;
    fx.runtime.cancel("conv-e2e").await.unwrap();
    hanging.await.unwrap().unwrap();
    eventually(|| {
        fx.runtime
            .live_models("conv-e2e")
            .is_some_and(|m| m.committed.len() == 3 && m.active_turn.is_none())
    })
    .await;

    // Stop → session gone; restart resumes the persisted session.
    fx.runtime.stop("conv-e2e").unwrap();
    assert!(!fx.runtime.is_running("conv-e2e"));
    let resumed = fx.runtime.start("conv-e2e").await.unwrap();
    assert!(resumed.resumed, "restart must take the session/load path");
    // The fake adapter replays two history updates → one committed turn.
    eventually(|| {
        fx.runtime
            .live_models("conv-e2e")
            .is_some_and(|m| m.committed.len() == 1)
    })
    .await;
    fx.runtime.stop("conv-e2e").unwrap();
}

// -- acceptance 2: the provider decision keeps the TUI path untouched -----------

#[tokio::test]
async fn non_acp_conversations_are_rejected_by_the_gate() {
    let fx = Fixture::new();

    // ACP-capable provider but PTY conversation type → TUI path.
    fx.create_conversation("conv-tui", ConversationType::Pty, "claude");
    let err = fx.runtime.start("conv-tui").await.unwrap_err();
    assert!(
        matches!(&err, fartcode_acp::Error::InvalidState(msg) if msg.contains("not on the ACP path")),
        "PTY conversations must never start an ACP session: {err}"
    );

    // Non-ACP provider (pi) with an ACP type → TUI path (capability gate).
    fx.create_conversation("conv-pi", ConversationType::Acp, "pi");
    let err = fx.runtime.start("conv-pi").await.unwrap_err();
    assert!(matches!(err, fartcode_acp::Error::InvalidState(_)));

    // No adapter process was spawned for either.
    assert!(!fx.runtime.is_running("conv-tui"));
    assert!(!fx.runtime.is_running("conv-pi"));
    // And no ACP events fired.
    assert!(fx.events.snapshots.lock().is_empty());
}

// -- acceptance 3: overlapping starts collapse into one adapter spawn -----------

#[tokio::test]
async fn concurrent_starts_spawn_exactly_one_adapter() {
    let fx = Fixture::new();
    fx.create_conversation("conv-race", ConversationType::Acp, "claude");

    // The double-mount race: board→task navigation, a project switch, or
    // StrictMode's double effect fire start() twice before the first has
    // registered with the manager. Without the start lock each call passed
    // the is_running fast path and spawned its own adapter; the second
    // overwrote the first in `clients`, orphaning a live process.
    let (a, b, c) = tokio::join!(
        fx.runtime.start("conv-race"),
        fx.runtime.start("conv-race"),
        fx.runtime.start("conv-race"),
    );
    let a = a.unwrap();
    let b = b.unwrap();
    let c = c.unwrap();

    // Every caller gets the SAME session (reference startSession
    // idempotency), and only one adapter process ever existed.
    assert_eq!(a.session_id, "fake-session-1");
    assert_eq!(b.session_id, a.session_id);
    assert_eq!(c.session_id, a.session_id);
    assert_eq!(
        fx.spawns.load(Ordering::SeqCst),
        1,
        "the start lock must collapse overlapping starts into one spawn"
    );

    // Still one after the dust settles: a later start is the plain
    // running-session fast path, not another spawn.
    let again = fx.runtime.start("conv-race").await.unwrap();
    assert_eq!(again.session_id, a.session_id);
    assert!(!again.resumed);
    assert_eq!(fx.spawns.load(Ordering::SeqCst), 1);

    fx.runtime.stop("conv-race").unwrap();
}

// -- acceptance 4: task deletion teardown stops ACP sessions --------------------

#[tokio::test]
async fn stop_task_stops_running_acp_sessions() {
    let fx = Fixture::new();
    fx.create_conversation("conv-tear", ConversationType::Acp, "claude");
    fx.runtime.start("conv-tear").await.unwrap();
    assert!(fx.runtime.is_running("conv-tear"));

    // The delete_task command's ACP step: stop every session of the task.
    fx.runtime.stop_task("task-1");
    assert!(!fx.runtime.is_running("conv-tear"));

    // Unknown tasks are a silent no-op.
    fx.runtime.stop_task("ghost-task");
}
