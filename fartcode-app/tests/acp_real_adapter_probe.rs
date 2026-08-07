//! LIVE diagnostic probe: the real `claude-agent-acp` adapter through the
//! app-layer AcpRuntime. Ignored by default (needs the adapter on PATH +
//! claude auth + network) — run explicitly:
//!   cargo test -p fartcode-app --test acp_real_adapter_probe -- --ignored --nocapture

use std::sync::Arc;
use std::time::Duration;

use fartcode_acp::client::SessionUpdateEvent;
use fartcode_acp::session::{LiveModels, PermissionRequestedEvent, SessionEvents};
use fartcode_app_lib::acp_runtime::{default_adapter_resolver, AcpRuntime};
use fartcode_core::conversations::model::ConversationType;
use fartcode_core::conversations::{
    ConversationStore, CreateConversationParams, DbConversationStore,
};
use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::events::BroadcastEventBus;
use fartcode_core::provider_accounts::ProviderAccountStore;
use fartcode_core::tasks::DbTaskStore;
use parking_lot::Mutex;

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

#[ignore = "live probe: needs claude-agent-acp on PATH and claude auth"]
#[tokio::test]
async fn real_adapter_prompt_round_trip() {
    let db = SqliteDb::init_in_memory().unwrap();
    let bus = Arc::new(BroadcastEventBus::new(64));
    let worktree = tempfile::tempdir().unwrap();
    std::fs::write(worktree.path().join("probe.txt"), "hello\n").unwrap();
    let worktree_path = worktree.path().to_str().unwrap().replace('\'', "''");
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
    let runtime = AcpRuntime::new(
        conversations.clone(),
        tasks,
        db.clone(),
        provider_accounts,
        events.clone(),
        Arc::new(|provider_id: &str| default_adapter_resolver(provider_id)),
    );

    conversations
        .create(CreateConversationParams {
            id: Some("conv-live".into()),
            project_id: "proj-1".into(),
            task_id: Some("task-1".into()),
            scope: None,
            provider: Some("claude".into()),
            title: "live probe".into(),
            auto_approve: None,
            model: None,
            initial_prompt: None,
            initial_queue: None,
            is_initial_conversation: false,
            r#type: Some(ConversationType::Acp),
        })
        .unwrap();

    let outcome = runtime.start("conv-live").await.unwrap();
    eprintln!("session started: {}", outcome.session_id);

    runtime
        .send_prompt(
            "conv-live",
            "In probe.txt, change 'hello' to 'goodbye'. Just do the edit.",
            None,
        )
        .await
        .unwrap();

    // Watchdog-style progress dump every 5s so a hang shows its last state.
    for i in 0..24 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let settled = runtime
            .live_models("conv-live")
            .is_some_and(|m| m.committed.len() == 1 && m.active_turn.is_none());
        eprintln!(
            "t={:>2}s updates={} snapshots={} permissions={} settled={settled}",
            (i + 1) * 5,
            events.updates.lock().len(),
            events.snapshots.lock().len(),
            events.permissions.lock().len(),
        );
        if settled {
            break;
        }
    }

    let updates = events.updates.lock().len();
    let snapshots = events.snapshots.lock().len();
    eprintln!("updates={updates} snapshots={snapshots}");
    assert!(updates > 0, "acp:update fired for the real adapter");
    assert!(snapshots > 0, "acp:transcript fired for the real adapter");
    let models = runtime.live_models("conv-live").unwrap();
    assert!(
        models.committed.len() == 1 && models.active_turn.is_none(),
        "turn must settle, models: {models:?}"
    );
    let text = format!("{:?}", models.committed);
    assert!(text.contains("goodbye"), "edit landed: {text}");
    assert_eq!(
        std::fs::read_to_string(worktree.path().join("probe.txt")).unwrap(),
        "goodbye\n"
    );
}
