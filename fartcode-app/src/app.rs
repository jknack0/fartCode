//! fartcode-app — application bootstrap (ARCHITECTURE §7).
//!
//! The single place concrete domain services are wired together and shared
//! via `Arc`. `App::init` builds everything from a DB path; the Tauri setup
//! hook manages it as state and forwards internal events to the frontend.

use std::sync::Arc;

use fartcode_core::conversations::DbConversationStore;
use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::dependencies::HostDependencyStore;
use fartcode_core::events::EventBus;
use fartcode_core::events::{BroadcastEventBus, InternalEvent};
use fartcode_core::fs_watch::FsWatchService;
use fartcode_core::projects::remote::RemoteProjectStore;
use fartcode_core::projects::worktrees::WorktreeManager;
use fartcode_core::projects::DbProjectStore;
use fartcode_core::provider_accounts::ProviderAccountStore;
use fartcode_core::pty::launcher::Rehydrator;
use fartcode_core::pty::sessions::SessionRegistry;
use fartcode_core::settings::{DbSettingsStore, TaskGroup, TASKS};
use fartcode_core::ssh_connections::SshConnectionStore;
use fartcode_core::tasks::deletion::TaskDeletionService;
use fartcode_core::tasks::operations::TaskCreationService;
use fartcode_core::tasks::DbTaskStore;

pub struct App {
    pub projects: Arc<DbProjectStore>,
    pub tasks: Arc<DbTaskStore>,
    pub event_bus: Arc<BroadcastEventBus>,
    // Wired for future tickets (E2-06 conversations/pty, settings UI);
    // kept alive by App so the stores' Arcs stay valid.
    #[allow(dead_code)]
    pub db: Arc<dyn Db>,
    #[allow(dead_code)]
    pub settings: Arc<DbSettingsStore>,
    #[allow(dead_code)]
    pub conversations: Arc<DbConversationStore>,
    /// E2-07 boot rehydration orchestration (call `rehydrate_all` on a
    /// background thread after init).
    pub rehydrator: Rehydrator,
    /// E2-09 task deletion/teardown.
    pub deletion: TaskDeletionService,
    /// E2-04 create+provision (worktree materialization). `create_task`
    /// routes through this so every task gets its worktree at creation.
    pub task_creation: TaskCreationService,
    /// E3-07 provider credentials (keyring-backed).
    pub provider_accounts: Arc<ProviderAccountStore>,
    /// E12-03 SSH connection profiles (secrets in the keyring).
    pub ssh_connections: Arc<SshConnectionStore>,
    /// E12-04 SSH-backed projects (create/clone + remote worktrees).
    pub remote_projects: Arc<RemoteProjectStore>,
    /// E12-05 remote PTY routing (one SSH manager per connection), shared by
    /// boot rehydration and the terminal manager.
    pub remote_pty: Arc<crate::remote_pty::RemotePtyRegistry>,
    /// E12-09 SSH port-forward tunnels (shared with E6-04 previews).
    pub port_forwards: Arc<crate::port_forwards::PortForwardService>,
    /// E4-01 workspace file+git watcher (registered via `watchers.rs`).
    pub fs_watch: Arc<FsWatchService>,
    /// E4-10 diff line comments (§14).
    pub line_comments: Arc<fartcode_core::line_comments::LineCommentStore>,
    /// E4-09 PR sync cache (§11 pull_requests; engine in fartcode-git).
    pub pr_sync: Arc<fartcode_core::pr_sync::PrSyncStore>,
    /// E17-01 project board issues (§13).
    pub issues: Arc<fartcode_core::issues::IssueStore>,
    /// E18-01 configurable pipeline columns (ADR-0037) — authoritative
    /// for board placement since the E18-07 flip (#66).
    pub columns: Arc<fartcode_core::issues::columns::ColumnStore>,
    /// #82 step spend ledger: durable launch/hold history + the budget
    /// guard's token totals.
    pub ledger: Arc<fartcode_core::issues::ledger::StepLedgerStore>,
    /// E18-04 step engine state: in-memory parked (queue-mode) steps.
    pub steps: crate::step_engine::StepEngine,
    /// E3-02 host dependencies (agent CLIs): detection cache +
    /// install/update (7d agents-on-this-machine). Shared with the
    /// rehydrator, which already consumed the store.
    pub host_dependencies: Arc<HostDependencyStore>,
}

impl App {
    /// `db_path`: explicit path (tests / `FARTCODE_DB_FILE`), else the platform
    /// app-data directory. `:memory:` is allowed for tests.
    pub fn init(db_path: Option<&str>) -> Result<Arc<Self>, String> {
        // SqliteDb::init already returns Arc<Self>.
        let db: Arc<dyn Db> =
            SqliteDb::init(db_path.map(|p| p.to_string()).as_deref()).map_err(|e| e.to_string())?;
        let event_bus = Arc::new(BroadcastEventBus::new(256));
        let settings = Arc::new(DbSettingsStore::new(db.clone()));
        let projects = Arc::new(DbProjectStore::new(
            db.clone(),
            settings.clone(),
            Arc::new(fartcode_git::CliGit),
            event_bus.clone(),
        ));
        let tasks = Arc::new(DbTaskStore::new(db.clone(), event_bus.clone()));
        let conversations = Arc::new(DbConversationStore::new(db.clone(), event_bus.clone()));
        let provider_accounts = Arc::new(ProviderAccountStore::new(db.clone()));
        let ssh_connections = Arc::new(SshConnectionStore::new(db.clone()));
        let remote_projects = Arc::new(RemoteProjectStore::new(db.clone(), event_bus.clone()));
        // E12-05: remote-workspace tasks spawn their PTYs on the SSH host.
        // russh is async and the PTY trait is blocking, so the registry needs
        // a runtime handle; Tauri's async runtime is the app's only one.
        let remote_pty = crate::remote_pty::RemotePtyRegistry::new(
            db.clone(),
            ssh_connections.clone(),
            event_bus.clone(),
            tauri::async_runtime::handle().inner().clone(),
        );

        // E2-09: one registry shared by boot rehydration (launches register)
        // and task deletion (cancel + reap).
        let sessions = Arc::new(SessionRegistry::new());

        // E3-02/7d: one host-dependency store shared by the rehydrator and
        // the `host_dependency_*` commands (same kv detection cache).
        let host_dependencies = Arc::new(HostDependencyStore::new(
            db.clone(),
            Arc::new(fartcode_terminal::PtyInstallRunner),
        ));

        // E2-07 boot rehydration: previously-spawned PTY conversations are
        // resumed after DB init (reference boot order). The app shell calls
        // `rehydrate_all` on a background thread (each launch blocks).
        let rehydrator = Rehydrator::new(
            Arc::new(fartcode_terminal::PortablePtyManager),
            host_dependencies.clone(),
            event_bus.clone(),
            conversations.clone(),
            tasks.clone(),
            projects.clone(),
            db.clone(),
            // #139: boot resume honors the app-level auto-approve default
            // (the conversation's own toggle still wins inside `rehydrate`).
            settings
                .get::<TaskGroup>(&TASKS)
                .map(|t| t.auto_approve_by_default)
                .unwrap_or(false),
            Some(sessions.clone()),
            Some(remote_pty.clone()),
        );

        // E2-09 task deletion/teardown.
        let worktrees =
            WorktreeManager::new(db.clone(), settings.clone(), Arc::new(fartcode_git::CliGit));
        let deletion = TaskDeletionService::new(
            db.clone(),
            tasks.clone(),
            conversations.clone(),
            projects.clone(),
            worktrees,
            sessions,
        );

        // E2-04 create+provision: the `create_task` command's real flow —
        // store-only create never materializes a worktree (regression the
        // E4-03 Changes panel exposed as "workspace has no local path").
        let task_creation = TaskCreationService::new(
            db.clone(),
            settings.clone(),
            Arc::new(fartcode_git::CliGit),
            WorktreeManager::new(db.clone(), settings.clone(), Arc::new(fartcode_git::CliGit)),
            event_bus.clone(),
        );

        // E4-01: file+git event watcher → live refresh pipeline. Lifecycle
        // (boot backfill, provision/delete hooks) is wired in watchers.rs.
        let fs_watch = Arc::new(
            FsWatchService::new(event_bus.clone() as Arc<dyn EventBus>)
                .map_err(|e| e.to_string())?,
        );

        // E4-10: diff line comments (§14) — CRUD + bidirectional task link.
        let line_comments = Arc::new(fartcode_core::line_comments::LineCommentStore::new(
            db.clone(),
            event_bus.clone() as Arc<dyn EventBus>,
        ));
        // E4-09: PR sync cache — the PR tab renders from it instantly; the
        // scheduler (spawned in lib.rs) and on-demand syncs feed it.
        let pr_sync = Arc::new(fartcode_core::pr_sync::PrSyncStore::new(
            db.clone(),
            event_bus.clone() as Arc<dyn EventBus>,
        ));
        // E17-01: project board issues (§13) — local-first store, derived
        // blocked state, cycle-rejected edges.
        let issues = Arc::new(fartcode_core::issues::IssueStore::new(
            db.clone(),
            event_bus.clone() as Arc<dyn EventBus>,
        ));
        // E18-01: pipeline column store (ADR-0037 spike) — seeded defaults
        // come from migration 0006 (existing projects) and the project
        // create hook (new projects).
        let columns = Arc::new(fartcode_core::issues::columns::ColumnStore::new(db.clone()));
        let ledger = Arc::new(fartcode_core::issues::ledger::StepLedgerStore::new(
            db.clone(),
        ));

        Ok(Arc::new(Self {
            db,
            settings,
            projects,
            tasks,
            conversations,
            event_bus,
            rehydrator,
            deletion,
            task_creation,
            provider_accounts,
            ssh_connections,
            remote_projects,
            port_forwards: Arc::new(crate::port_forwards::PortForwardService::new(
                remote_pty.clone(),
            )),
            remote_pty,
            fs_watch,
            line_comments,
            pr_sync,
            issues,
            columns,
            ledger,
            steps: crate::step_engine::StepEngine::new(),
            host_dependencies,
        }))
    }
}

/// The variant → `"domain:name"` wire-name table; `None` = backend-internal,
/// skipped by the forwarder. Only the events the Phase-0 UI consumes are
/// named. Adding a UI-visible event is one row here — the payload keys come
/// from the enum's serde derive (camelCase fields, events.rs) — plus a
/// snapshot entry in tests/snapshots/events.json.
fn event_name(event: &InternalEvent) -> Option<&'static str> {
    use InternalEvent as E;
    Some(match event {
        // E12-06: connection lifecycle. `attempt`/`delayMs` ride only the
        // reconnecting frames, so the UI can render "retrying in 5s (3/5)"
        // without keeping its own ladder.
        E::SshConnectionStateChanged { .. } => "ssh:state_changed",
        E::SshConnectionHealthChanged { .. } => "ssh:health_changed",
        // E12-10 / ADR-0044: a possibly-leaked (billed) machine must reach
        // the user, not just the log.
        E::ByoiTerminateWarning { .. } => "task:terminate_warning",
        E::ProjectAdded { .. } => "project:added",
        E::ProjectDeleted { .. } => "project:deleted",
        E::TaskCreated { .. } => "task:created",
        E::TaskRenamed { .. } => "task:renamed",
        E::TaskDeleted { .. } => "task:deleted",
        E::TaskStatusChanged { .. } => "task:status_changed",
        // 7a archive (⌘⌫ "a archive instead") / ⌘K restore — consumers
        // refetch the project task list (archivedAt filters the board).
        E::TaskArchived { .. } => "task:archived",
        E::TaskRestored { .. } => "task:restored",
        E::ConversationCreated { .. } => "conversation:created",
        E::ConversationRenamed { .. } => "conversation:renamed",
        E::ConversationDeleted { .. } => "conversation:deleted",
        E::GitChanged { .. } => "git:changed",
        E::PrUpdated { .. } => "pr:updated",
        E::FilesChanged { .. } => "files:changed",
        E::CommentCreated { .. } => "comment:created",
        E::CommentResolved { .. } => "comment:resolved",
        // E17: project board — any issue change refetches the project's list
        // (blocked status is derived, so one move can flip other badges).
        E::IssueCreated { .. } => "issue:created",
        E::IssueUpdated { .. } => "issue:updated",
        E::IssueDeleted { .. } => "issue:deleted",
        // E18-04/05 step engine: launch is a directive (open/focus the
        // task's agent terminal); the rest are state notifications for the
        // queue-confirm overlay and derived step-done styling.
        E::StepLaunch { .. } => "step:launch",
        E::StepQueued { .. } => "step:queued",
        E::StepQueueCleared { .. } => "step:queue_cleared",
        E::StepSettled { .. } => "step:settled",
        E::StepChainHeld { .. } => "step:chain_held",
        // App settings (set_default_agent): consumers refetch the changed
        // key (the ProjectSettings "Default agent · model" row).
        E::SettingChanged { .. } => "setting:changed",
        _ => return None,
    })
}

/// Serializes an internal event for the frontend channel (`fartcode:event`).
/// Only the events the Phase-0 UI consumes are mapped; the rest are skipped.
///
/// The envelope is `{"type": event_name(..), ..payload}`, payload keys taken
/// from the enum's own serde derive — except the hand-written arms below,
/// which exist ONLY to stay byte-identical with the shipped wire format.
/// Every envelope is pinned byte-for-byte by tests/event_wire_contract.rs.
pub fn event_to_value(event: &InternalEvent) -> Option<serde_json::Value> {
    use serde_json::json;

    let name = event_name(event)?;

    let payload = match event {
        // HAND-WRITTEN EXCEPTIONS — shapes the serde derive would not
        // produce. Shipped drift, not design: task:created emits the task id
        // as "id" (derive path) while the arms below emit "taskId", and the
        // frontend's hand-typed FartcodeEvent encodes exactly this split.
        // TODO(audits/2026-08-12-architecture-deep-dive.md T1): emit "taskId"
        // everywhere in ONE coordinated backend+frontend change, regenerate
        // tests/snapshots/events.json, and delete these arms.
        InternalEvent::TaskDeleted { id }
        | InternalEvent::TaskArchived { id }
        | InternalEvent::TaskRestored { id } => json!({ "taskId": id }),
        // task:renamed shipped on the "taskId" side of the split (50d1f6e),
        // unlike its sibling task:created — the derive would say "id".
        InternalEvent::TaskRenamed {
            id,
            project_id,
            name,
        } => json!({ "taskId": id, "projectId": project_id, "name": name }),
        // Also renames `new_status` → "status" and drops `old_status`.
        InternalEvent::TaskStatusChanged { id, new_status, .. } => {
            json!({ "taskId": id, "status": new_status })
        }
        // Drops `provider` from the wire (the UI never consumed it).
        InternalEvent::ConversationCreated {
            id, task_id, title, ..
        } => json!({ "id": id, "taskId": task_id, "title": title }),

        // Derive path: the enum serializes as {"type": tag, "payload": {..}}
        // (events.rs) — the envelope takes the payload fields verbatim.
        _ => match serde_json::to_value(event).ok()? {
            serde_json::Value::Object(mut tagged) => {
                tagged.remove("payload").unwrap_or_else(|| json!({}))
            }
            _ => json!({}),
        },
    };

    let fields = match payload {
        serde_json::Value::Object(fields) => fields,
        _ => serde_json::Map::new(),
    };
    // "type" first, then payload fields in declaration order — serde_json's
    // preserve_order feature is active (via tauri), so insertion order is
    // the wire order the snapshot pins.
    let mut envelope = serde_json::Map::with_capacity(fields.len() + 1);
    envelope.insert("type".into(), name.into());
    envelope.extend(fields);
    Some(serde_json::Value::Object(envelope))
}

/// Spawns the event-forwarding task: internal events → `fartcode:event` on the
/// Tauri emitter (runs for the app's lifetime).
pub fn spawn_event_forwarder(app_handle: tauri::AppHandle, event_bus: Arc<BroadcastEventBus>) {
    use tauri::Emitter;
    let rx = event_bus.subscribe();
    tauri::async_runtime::spawn(forward_events(rx, move |value| {
        let _ = app_handle.emit("fartcode:event", value);
    }));
}

/// Forwarder loop, generic over the emitter so tests can observe the
/// envelope stream without a Tauri app. Only a closed bus ends it.
///
/// Lag here is not skippable the way the other subscribers' is: the
/// frontend's liveness model is "event → refetch", so a silently dropped
/// envelope strands the UI stale. On `Lagged` the frontend gets a
/// `sys:resync` envelope instead — unknown types fall through its event
/// switches safely today; refetch-on-resync wiring is a follow-up.
async fn forward_events(
    mut rx: tokio::sync::broadcast::Receiver<InternalEvent>,
    emit: impl Fn(serde_json::Value),
) {
    loop {
        match rx.recv().await {
            Ok(event) => {
                if let Some(value) = event_to_value(&event) {
                    emit(value);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(dropped = n, "event forwarder lagged; emitting sys:resync");
                emit(serde_json::json!({ "type": "sys:resync", "droppedCount": n }));
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::events::InternalEvent;

    /// Envelope bytes are pinned by tests/event_wire_contract.rs; this test
    /// documents only the INTENTIONAL quirks the hand-written arms preserve.
    #[test]
    fn maps_ui_events_to_frontend_payloads() {
        // Derive path: name from the table, keys from the enum's serde derive.
        let ev = InternalEvent::TaskCreated {
            id: "t1".into(),
            project_id: "p1".into(),
            name: "fix".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "task:created");
        assert_eq!(v["id"], "t1");
        assert_eq!(v["projectId"], "p1");

        // Hand exceptions: the shipped id/taskId drift (see the TODO in
        // event_to_value) — task:created says "id", these say "taskId".
        for (ev, ty) in [
            (
                InternalEvent::TaskDeleted { id: "t1".into() },
                "task:deleted",
            ),
            (
                InternalEvent::TaskArchived { id: "t1".into() },
                "task:archived",
            ),
            (
                InternalEvent::TaskRestored { id: "t1".into() },
                "task:restored",
            ),
        ] {
            let v = event_to_value(&ev).unwrap();
            assert_eq!(v["type"], ty);
            assert_eq!(v["taskId"], "t1");
            assert!(v.get("id").is_none(), "{ty} must not emit \"id\"");
        }

        // task:renamed shipped on the "taskId" side too (50d1f6e), while
        // keeping projectId/name — its arm differs from the derive only
        // in the id key.
        let ev = InternalEvent::TaskRenamed {
            id: "t1".into(),
            project_id: "p1".into(),
            name: "renamed".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "task:renamed");
        assert_eq!(v["taskId"], "t1");
        assert_eq!(v["projectId"], "p1");
        assert_eq!(v["name"], "renamed");
        assert!(v.get("id").is_none(), "task:renamed must not emit \"id\"");

        // task:status_changed renames new_status → "status", drops old_status.
        let ev = InternalEvent::TaskStatusChanged {
            id: "t1".into(),
            old_status: "open".into(),
            new_status: "running".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["taskId"], "t1");
        assert_eq!(v["status"], "running");
        assert!(v.get("oldStatus").is_none());

        // conversation:created drops `provider`.
        let ev = InternalEvent::ConversationCreated {
            id: "cv1".into(),
            task_id: "t1".into(),
            provider: "claude".into(),
            title: "chat".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["taskId"], "t1");
        assert!(v.get("provider").is_none());

        // Events the UI doesn't consume are skipped, not panicked on.
        assert!(event_to_value(&InternalEvent::AppStarted).is_none());
        assert!(event_to_value(&InternalEvent::AgentStart {
            provider: "claude".into(),
            project_id: "p".into(),
            task_id: "t".into(),
            conversation_id: "c".into(),
        })
        .is_none());
    }

    #[tokio::test]
    async fn lagged_forwarder_emits_sys_resync() {
        use fartcode_core::events::{BroadcastEventBus, EventBus};
        use std::sync::Mutex;

        // Capacity 1: an un-drained receiver lags as soon as the ring wraps.
        let bus = BroadcastEventBus::new(1);
        let rx = bus.subscribe();
        for i in 0..3 {
            bus.send(InternalEvent::ProjectDeleted {
                id: format!("p{i}"),
            });
        }
        drop(bus); // close the bus so the loop ends after draining

        let emitted = Mutex::new(Vec::new());
        forward_events(rx, |v| emitted.lock().unwrap().push(v)).await;

        let emitted = emitted.into_inner().unwrap();
        assert_eq!(emitted[0]["type"], "sys:resync");
        assert_eq!(emitted[0]["droppedCount"], 2);
        // The surviving newest event still follows the resync marker.
        assert_eq!(emitted[1]["type"], "project:deleted");
        assert_eq!(emitted[1]["id"], "p2");
        assert_eq!(emitted.len(), 2);
    }
}
