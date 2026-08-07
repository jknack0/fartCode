//! Conversation session supervisor integration tests (ticket E2-05
//! acceptance criteria).
//!
//! All tests use `:memory:` DBs — never the real app data path.

use std::sync::Arc;

use fartcode_core::conversations::model::{
    ConversationScope, ConversationType, InitialQueuePrompt,
};
use fartcode_core::conversations::{
    resolve_agent_session_command_args, ConversationAgentStatus, ConversationStore,
    CreateConversationParams, DbConversationStore, PROVIDER_SESSION_ID_REQUIRED_FOR_RESUME,
};
use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use fartcode_core::Error;

struct Fixture {
    db: Arc<SqliteDb>,
    bus: Arc<BroadcastEventBus>,
    store: DbConversationStore,
    project_id: String,
    task_id: String,
}

impl Fixture {
    fn new() -> Self {
        let db = SqliteDb::init_in_memory().unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        // Minimal project + task rows (FK targets for conversations).
        db.conn()
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO projects (id, name, path) VALUES ('proj-1', 'demo', '/tmp/demo');
                 INSERT INTO tasks (id, project_id, name, status)
                     VALUES ('task-1', 'proj-1', 'fix', 'in_progress');
                 INSERT INTO tasks (id, project_id, name, status)
                     VALUES ('task-2', 'proj-1', 'other', 'in_progress');",
            )
            .unwrap();
        let store = DbConversationStore::new(db.clone(), bus.clone());
        Self {
            db,
            bus,
            store,
            project_id: "proj-1".into(),
            task_id: "task-1".into(),
        }
    }

    fn conv(&self, id: &str, r#type: ConversationType) -> CreateConversationParams {
        CreateConversationParams {
            id: Some(id.into()),
            project_id: self.project_id.clone(),
            task_id: Some(self.task_id.clone()),
            scope: None,
            provider: Some("claude".into()),
            title: "Fix the bug".into(),
            auto_approve: None,
            model: Some("sonnet".into()),
            initial_prompt: Some("Fix it".into()),
            initial_queue: None,
            is_initial_conversation: false,
            r#type: Some(r#type),
        }
    }
}

// -- acceptance 1: stable session id + resume round-trip via session_id ------

#[test]
fn pty_conversation_gets_session_id_at_creation() {
    let fx = Fixture::new();
    let conv = fx
        .store
        .create(fx.conv("conv-1", ConversationType::Pty))
        .unwrap();
    assert_eq!(conv.session_id.as_deref(), Some("conv-1"));
    assert_eq!(conv.r#type, ConversationType::Pty);
    // The versioned config carries the model + prompt.
    assert_eq!(conv.config.model.as_deref(), Some("sonnet"));
    assert_eq!(conv.config.initial_prompt.as_deref(), Some("Fix it"));
    // Task-scoped by default.
    assert_eq!(conv.scope, ConversationScope::Task);
    assert_eq!(conv.task_id.as_deref(), Some("task-1"));
}

#[test]
fn acp_conversation_starts_without_session_id_and_derives_initial_queue() {
    let fx = Fixture::new();
    let mut params = fx.conv("conv-acp", ConversationType::Acp);
    params.initial_queue = Some(vec![InitialQueuePrompt {
        text: "Review this".into(),
        hidden_context: None,
    }]);
    let conv = fx.store.create(params).unwrap();
    assert_eq!(conv.session_id, None, "ACP sessions establish ids lazily");
    assert_eq!(
        conv.initial_queue(),
        Some(vec![InitialQueuePrompt {
            text: "Review this".into(),
            hidden_context: None,
        }])
    );

    // Legacy fallback: acp with only initialPrompt → single queued prompt.
    let mut legacy = fx.conv("conv-acp2", ConversationType::Acp);
    legacy.initial_queue = None;
    legacy.initial_prompt = Some("  Do the thing  ".into());
    let conv = fx.store.create(legacy).unwrap();
    assert_eq!(
        conv.initial_queue(),
        Some(vec![InitialQueuePrompt {
            text: "Do the thing".into(),
            hidden_context: None
        }])
    );

    // Once a session id is set, the queue is exhausted.
    fx.store
        .set_session_id("conv-acp", "provider-native")
        .unwrap();
    let conv = fx.store.get("conv-acp").unwrap().unwrap();
    assert_eq!(conv.initial_queue(), None);
}

// -- E2-11-3 acceptance 3: provider decision hook (ACP vs TUI path) ----------

#[test]
fn session_path_decision_acp_requires_capability_and_type() {
    use fartcode_core::conversations::{resolve_session_path, SessionPath};
    let fx = Fixture::new();

    // ACP config + ACP-capable provider (claude) → structured ACP path.
    let mut params = fx.conv("conv-acp", ConversationType::Acp);
    params.provider = Some("claude".into());
    let conv = fx.store.create(params).unwrap();
    assert_eq!(resolve_session_path(&conv), SessionPath::Acp);

    // ACP config but NON-ACP provider (pi) → TUI/PTY path untouched.
    let mut params = fx.conv("conv-pi", ConversationType::Acp);
    params.provider = Some("pi".into());
    let conv = fx.store.create(params).unwrap();
    assert_eq!(resolve_session_path(&conv), SessionPath::Pty);

    // ACP-capable provider but PTY config → TUI path (type wins too).
    let mut params = fx.conv("conv-pty", ConversationType::Pty);
    params.provider = Some("claude".into());
    let conv = fx.store.create(params).unwrap();
    assert_eq!(resolve_session_path(&conv), SessionPath::Pty);

    // No provider at all → TUI path.
    let mut params = fx.conv("conv-none", ConversationType::Acp);
    params.provider = None;
    let conv = fx.store.create(params).unwrap();
    assert_eq!(resolve_session_path(&conv), SessionPath::Pty);
}

#[test]
fn set_session_id_round_trips() {
    let fx = Fixture::new();
    fx.store
        .create(fx.conv("conv-1", ConversationType::Pty))
        .unwrap();

    // Native id overwrites the initial conversation id.
    fx.store
        .set_session_id("conv-1", "  provider-native-1  ")
        .unwrap();
    let conv = fx.store.get("conv-1").unwrap().unwrap();
    assert_eq!(
        conv.session_id.as_deref(),
        Some("provider-native-1"),
        "trimmed"
    );

    // Empty id → typed error.
    let err = fx.store.set_session_id("conv-1", "   ").unwrap_err();
    assert!(matches!(err, Error::EmptySessionId));

    // Missing conversation → typed error.
    let err = fx.store.set_session_id("nope", "x").unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));
}

// -- acceptance 2: resume-flag logic matches the 7-provider list + fallback --

#[test]
fn resume_provider_set_and_fallback_match_reference() {
    assert_eq!(
        PROVIDER_SESSION_ID_REQUIRED_FOR_RESUME,
        [
            "amp",
            "codex",
            "commandcode",
            "droid",
            "goose",
            "oh-my-pi",
            "pi"
        ]
    );

    // 7-set provider + native id != conversation id → native + resuming.
    for provider in PROVIDER_SESSION_ID_REQUIRED_FOR_RESUME {
        let (sid, resuming) =
            resolve_agent_session_command_args(provider, "conv-1", Some("native"), true, None);
        assert_eq!(
            (sid.as_str(), resuming),
            ("native", true),
            "provider {provider}"
        );
    }
    // ... but with no native id → fresh (isResuming cleared).
    for provider in PROVIDER_SESSION_ID_REQUIRED_FOR_RESUME {
        let (sid, resuming) =
            resolve_agent_session_command_args(provider, "conv-1", None, true, None);
        assert_eq!(
            (sid.as_str(), resuming),
            ("conv-1", false),
            "provider {provider}"
        );
    }

    // Non-7-set provider passes through untouched (native id ignored).
    let (sid, resuming) =
        resolve_agent_session_command_args("claude", "conv-1", Some("native"), true, None);
    assert_eq!((sid.as_str(), resuming), ("conv-1", true));
}

// -- acceptance 3: hydrate after restart restores per-task resume state ------

#[test]
fn hydrate_first_spawn_then_resume_after_restart() {
    let fx = Fixture::new();
    fx.store
        .create(fx.conv("conv-1", ConversationType::Pty))
        .unwrap();

    // Store-created PTY conversations carry session_id = conversation.id
    // (reference createConversation) — a restart hydrate reports resume.
    let first = fx
        .store
        .hydrate(&fx.project_id, &fx.task_id, "conv-1")
        .unwrap();
    assert!(!first.is_first_spawn);
    assert!(first.is_resuming);

    // "Restart": a fresh store over the same DB still reports resume.
    let store2 = DbConversationStore::new(fx.db.clone(), fx.bus.clone());
    let second = store2
        .hydrate(&fx.project_id, &fx.task_id, "conv-1")
        .unwrap();
    assert!(second.is_resuming);

    // Per-task restore: the conversation shows up with its session id.
    let convs = store2.list_by_task(&fx.task_id).unwrap();
    assert_eq!(convs.len(), 1);
    assert_eq!(convs[0].session_id.as_deref(), Some("conv-1"));
}

#[test]
fn hydrate_null_session_id_fires_fresh_spawn_guard() {
    let fx = Fixture::new();
    // A PTY conversation row WITHOUT a session id (e.g. created through the
    // E2-01 task flow, which never set session_id) — hydrate must treat it
    // as the first spawn, write the guard, and report fresh.
    fx.db
        .conn()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO conversations (id, project_id, task_id, scope, title, provider,
                 config, type, is_initial_conversation, last_interacted_at)
             VALUES ('legacy-1', 'proj-1', 'task-1', 'task', 'Legacy', 'claude',
                     '{\"version\":\"1\",\"type\":\"pty\",\"initialPrompt\":\"Go\"}',
                     'pty', 1, datetime('now'))",
            [],
        )
        .unwrap();

    let first = fx
        .store
        .hydrate(&fx.project_id, &fx.task_id, "legacy-1")
        .unwrap();
    assert!(first.is_first_spawn, "null session id = first spawn");
    assert!(!first.is_resuming);
    assert_eq!(
        fx.store
            .get("legacy-1")
            .unwrap()
            .unwrap()
            .session_id
            .as_deref(),
        Some("legacy-1"),
        "idempotency guard written"
    );

    // Second hydrate: guard is set → resume.
    let second = fx
        .store
        .hydrate(&fx.project_id, &fx.task_id, "legacy-1")
        .unwrap();
    assert!(!second.is_first_spawn);
    assert!(second.is_resuming);
}

#[test]
fn hydrate_ownership_and_errors() {
    let fx = Fixture::new();
    fx.store
        .create(fx.conv("conv-1", ConversationType::Pty))
        .unwrap();

    // Wrong task / project → conversation-not-found.
    let err = fx
        .store
        .hydrate(&fx.project_id, "task-2", "conv-1")
        .unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));
    let err = fx
        .store
        .hydrate("other-proj", &fx.task_id, "conv-1")
        .unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));
    let err = fx
        .store
        .hydrate(&fx.project_id, &fx.task_id, "nope")
        .unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));

    // dehydrate: ACP no-ops; PTY validates ownership.
    fx.store
        .dehydrate(&fx.project_id, &fx.task_id, "conv-1")
        .unwrap();
    let err = fx
        .store
        .dehydrate(&fx.project_id, "task-2", "conv-1")
        .unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));
}

#[test]
fn hydrate_acp_never_writes_the_guard_and_tracks_provider_session() {
    let fx = Fixture::new();
    // ACP without a provider session id: fresh, and NO session_id write.
    fx.store
        .create(fx.conv("conv-acp", ConversationType::Acp))
        .unwrap();
    let first = fx
        .store
        .hydrate(&fx.project_id, &fx.task_id, "conv-acp")
        .unwrap();
    assert!(first.is_first_spawn);
    assert!(!first.is_resuming);
    assert_eq!(
        fx.store.get("conv-acp").unwrap().unwrap().session_id,
        None,
        "ACP hydrate must not write the PTY guard"
    );

    // Once the ACP client records a provider session id → resume.
    fx.store.set_session_id("conv-acp", "acp-native").unwrap();
    let second = fx
        .store
        .hydrate(&fx.project_id, &fx.task_id, "conv-acp")
        .unwrap();
    assert!(!second.is_first_spawn);
    assert!(second.is_resuming);
}

// -- CRUD + tracking ---------------------------------------------------------

#[test]
fn rename_touch_mark_seen_and_agent_status() {
    let fx = Fixture::new();
    fx.store
        .create(fx.conv("conv-1", ConversationType::Pty))
        .unwrap();

    // Rename trims + clamps to 100 chars.
    let renamed = fx.store.rename("conv-1", "  a very long title...").unwrap();
    assert_eq!(renamed.title, "a very long title...");
    let long = "x".repeat(150);
    let renamed = fx.store.rename("conv-1", &long).unwrap();
    assert_eq!(renamed.title.len(), 100);

    // Multi-byte UTF-8 titles must not panic (char-clamped, not byte-sliced).
    let emoji = "🚀".repeat(120);
    let renamed = fx.store.rename("conv-1", &emoji).unwrap();
    assert_eq!(renamed.title.chars().count(), 100);
    assert_eq!(renamed.title, "🚀".repeat(100));

    // Rename on a missing conversation → typed error.
    let err = fx.store.rename("nope", "x").unwrap_err();
    assert!(matches!(err, Error::ConversationNotFound(_)));

    // Touch updates last_interacted_at.
    fx.store.touch("conv-1", "2026-08-03 12:00:00").unwrap();
    assert_eq!(
        fx.store
            .get("conv-1")
            .unwrap()
            .unwrap()
            .last_interacted_at
            .as_deref(),
        Some("2026-08-03 12:00:00")
    );

    // Agent status + seen tracking.
    fx.store
        .update_agent_status("conv-1", ConversationAgentStatus::Working)
        .unwrap();
    let conv = fx.store.get("conv-1").unwrap().unwrap();
    assert_eq!(conv.agent_status, Some(ConversationAgentStatus::Working));
    assert!(!conv.agent_status_seen);
    fx.store.mark_seen("conv-1").unwrap();
    assert!(fx.store.get("conv-1").unwrap().unwrap().agent_status_seen);
}

#[test]
fn delete_scoped_and_emits_event() {
    let fx = Fixture::new();
    fx.store
        .create(fx.conv("conv-1", ConversationType::Pty))
        .unwrap();
    let mut events = fx.bus.subscribe();

    // Wrong task → reference fire-and-forget: Ok, nothing deleted, no event.
    fx.store.delete(&fx.project_id, "task-2", "conv-1").unwrap();
    assert!(fx.store.get("conv-1").unwrap().is_some());

    // Correct scope → deleted + event emitted.
    fx.store
        .delete(&fx.project_id, &fx.task_id, "conv-1")
        .unwrap();
    assert!(fx.store.get("conv-1").unwrap().is_none());
    let mut seen = false;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev, InternalEvent::ConversationDeleted { id } if id == "conv-1") {
            seen = true;
        }
    }
    assert!(seen, "conversation:deleted emitted");
}

#[test]
fn project_scope_rejected_in_phase_0() {
    let fx = Fixture::new();
    let mut params = fx.conv("conv-1", ConversationType::Pty);
    params.scope = Some(ConversationScope::Project);
    let err = fx.store.create(params).unwrap_err();
    assert!(err.to_string().contains("invalid task input"));

    // task_id missing → typed error too.
    let mut params = fx.conv("conv-1", ConversationType::Pty);
    params.task_id = None;
    let err = fx.store.create(params).unwrap_err();
    assert!(err.to_string().contains("invalid task input"));
}

#[test]
fn list_by_project_and_event_on_create() {
    let fx = Fixture::new();
    let mut events = fx.bus.subscribe();
    fx.store
        .create(fx.conv("conv-1", ConversationType::Pty))
        .unwrap();
    let mut params = fx.conv("conv-2", ConversationType::Pty);
    params.task_id = Some("task-2".into());
    fx.store.create(params).unwrap();

    assert_eq!(fx.store.list_by_project(&fx.project_id).unwrap().len(), 2);
    assert_eq!(fx.store.list_by_task(&fx.task_id).unwrap().len(), 1);

    let mut created = 0;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev, InternalEvent::ConversationCreated { .. }) {
            created += 1;
        }
    }
    assert_eq!(created, 2, "conversation:created emitted per create");
}
