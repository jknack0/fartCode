//! fartcode-app — application bootstrap (ARCHITECTURE §7).
//!
//! The single place concrete domain services are wired together and shared
//! via `Arc`. `App::init` builds everything from a DB path; the Tauri setup
//! hook manages it as state and forwards internal events to the frontend.

use std::sync::Arc;

use fartcode_core::conversations::DbConversationStore;
use fartcode_core::db::{Db, SqliteDb};
use fartcode_core::dependencies::{HostDependencyStore, ProcessInstallRunner};
use fartcode_core::events::EventBus;
use fartcode_core::events::{BroadcastEventBus, InternalEvent};
use fartcode_core::fs_watch::FsWatchService;
use fartcode_core::projects::remote::RemoteProjectStore;
use fartcode_core::projects::worktrees::WorktreeManager;
use fartcode_core::projects::DbProjectStore;
use fartcode_core::provider_accounts::ProviderAccountStore;
use fartcode_core::pty::launcher::Rehydrator;
use fartcode_core::pty::sessions::SessionRegistry;
use fartcode_core::settings::DbSettingsStore;
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
            Arc::new(ProcessInstallRunner),
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
            false, // auto-approve defaults off on boot
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

/// Serializes an internal event for the frontend channel (`fartcode:event`).
/// Only the events the Phase-0 UI consumes are mapped; the rest are skipped.
pub fn event_to_value(event: &InternalEvent) -> Option<serde_json::Value> {
    use serde_json::json;
    match event {
        // E12-06: connection lifecycle. `attempt`/`delayMs` ride only the
        // reconnecting frames, so the UI can render "retrying in 5s (3/5)"
        // without keeping its own ladder.
        InternalEvent::SshConnectionStateChanged {
            connection_id,
            state,
            attempt,
            delay_ms,
            error,
        } => Some(json!({
            "type": "ssh:state_changed",
            "connectionId": connection_id,
            "state": state,
            "attempt": attempt,
            "delayMs": delay_ms,
            "error": error,
        })),
        InternalEvent::SshConnectionHealthChanged {
            connection_id,
            degraded,
        } => Some(json!({
            "type": "ssh:health_changed",
            "connectionId": connection_id,
            "degraded": degraded,
        })),
        InternalEvent::ProjectAdded { id, name, path } => Some(json!({
            "type": "project:added", "id": id, "name": name, "path": path,
        })),
        InternalEvent::ProjectDeleted { id } => {
            Some(json!({ "type": "project:deleted", "id": id }))
        }
        InternalEvent::TaskCreated {
            id,
            project_id,
            name,
        } => Some(json!({
            "type": "task:created", "id": id, "projectId": project_id, "name": name,
        })),
        InternalEvent::TaskDeleted { id } => Some(json!({ "type": "task:deleted", "taskId": id })),
        InternalEvent::TaskStatusChanged { id, new_status, .. } => {
            Some(json!({ "type": "task:status_changed", "taskId": id, "status": new_status }))
        }
        // 7a archive (⌘⌫ "a archive instead") / ⌘K restore — consumers
        // refetch the project task list (archivedAt filters the board).
        InternalEvent::TaskArchived { id } => {
            Some(json!({ "type": "task:archived", "taskId": id }))
        }
        InternalEvent::TaskRestored { id } => {
            Some(json!({ "type": "task:restored", "taskId": id }))
        }
        InternalEvent::ConversationCreated {
            id, task_id, title, ..
        } => Some(json!({
            "type": "conversation:created", "id": id, "taskId": task_id, "title": title,
        })),
        InternalEvent::ConversationRenamed { id, title } => Some(json!({
            "type": "conversation:renamed", "id": id, "title": title,
        })),
        InternalEvent::ConversationDeleted { id } => {
            Some(json!({ "type": "conversation:deleted", "id": id }))
        }
        InternalEvent::GitChanged {
            project_id,
            workspace_id,
        } => Some(json!({
            "type": "git:changed", "projectId": project_id, "workspaceId": workspace_id,
        })),
        InternalEvent::PrUpdated {
            workspace_id,
            pr_url,
        } => Some(json!({
            "type": "pr:updated", "workspaceId": workspace_id, "prUrl": pr_url,
        })),
        InternalEvent::FilesChanged {
            workspace_id,
            paths,
        } => Some(json!({
            "type": "files:changed", "workspaceId": workspace_id, "paths": paths,
        })),
        InternalEvent::CommentCreated {
            id,
            task_id,
            file_path,
            line_number,
        } => Some(json!({
            "type": "comment:created", "id": id, "taskId": task_id,
            "filePath": file_path, "lineNumber": line_number,
        })),
        InternalEvent::CommentResolved { id } => {
            Some(json!({ "type": "comment:resolved", "id": id }))
        }
        // E17: project board — any issue change refetches the project's list
        // (blocked status is derived, so one move can flip other badges).
        InternalEvent::IssueCreated {
            id,
            project_id,
            title,
        } => Some(json!({
            "type": "issue:created", "id": id, "projectId": project_id, "title": title,
        })),
        InternalEvent::IssueUpdated { id, project_id } => Some(json!({
            "type": "issue:updated", "id": id, "projectId": project_id,
        })),
        InternalEvent::IssueDeleted { id, project_id } => Some(json!({
            "type": "issue:deleted", "id": id, "projectId": project_id,
        })),
        // E18-04/05 step engine: launch is a directive (open/focus the
        // task's agent terminal); the rest are state notifications for the
        // queue-confirm overlay and derived step-done styling.
        InternalEvent::StepLaunch {
            issue_id,
            project_id,
            column_id,
            task_id,
            prompt,
            provider,
            model,
            effort,
            reattached,
        } => Some(json!({
            "type": "step:launch", "issueId": issue_id, "projectId": project_id,
            "columnId": column_id, "taskId": task_id, "prompt": prompt,
            "provider": provider, "model": model, "effort": effort,
            "reattached": reattached,
        })),
        InternalEvent::StepQueued {
            issue_id,
            project_id,
            column_id,
            provider,
            model,
            effort,
        } => Some(json!({
            "type": "step:queued", "issueId": issue_id, "projectId": project_id,
            "columnId": column_id, "provider": provider, "model": model,
            "effort": effort,
        })),
        InternalEvent::StepQueueCleared {
            issue_id,
            project_id,
            column_id,
        } => Some(json!({
            "type": "step:queue_cleared", "issueId": issue_id,
            "projectId": project_id, "columnId": column_id,
        })),
        InternalEvent::StepSettled {
            issue_id,
            project_id,
            column_id,
            task_id,
        } => Some(json!({
            "type": "step:settled", "issueId": issue_id, "projectId": project_id,
            "columnId": column_id, "taskId": task_id,
        })),
        InternalEvent::StepChainHeld {
            issue_id,
            project_id,
            column_id,
            target_column_id,
            reason,
        } => Some(json!({
            "type": "step:chain_held", "issueId": issue_id, "projectId": project_id,
            "columnId": column_id, "targetColumnId": target_column_id, "reason": reason,
        })),
        // App settings (set_default_agent): consumers refetch the changed
        // key (the ProjectSettings "Default agent · model" row).
        InternalEvent::SettingChanged { key } => Some(json!({
            "type": "setting:changed", "key": key,
        })),
        _ => None,
    }
}

/// Spawns the event-forwarding task: internal events → `fartcode:event` on the
/// Tauri emitter (runs for the app's lifetime).
pub fn spawn_event_forwarder(app_handle: tauri::AppHandle, event_bus: Arc<BroadcastEventBus>) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        let mut rx = event_bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Some(value) = event_to_value(&event) {
                        let _ = app_handle.emit("fartcode:event", value);
                    }
                }
                // A lagging frontend drops the oldest events but the bridge
                // must survive — only a closed bus ends the forwarder.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::events::InternalEvent;

    #[test]
    fn maps_ui_events_to_frontend_payloads() {
        let ev = InternalEvent::ProjectAdded {
            id: "p1".into(),
            name: "demo".into(),
            path: "/repo/demo".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "project:added");
        assert_eq!(v["id"], "p1");

        let ev = InternalEvent::TaskCreated {
            id: "t1".into(),
            project_id: "p1".into(),
            name: "fix".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "task:created");
        assert_eq!(v["projectId"], "p1");
        assert_eq!(v["name"], "fix");

        // E4-01 watcher events reach the frontend envelope.
        let ev = InternalEvent::GitChanged {
            project_id: "p1".into(),
            workspace_id: "w1".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "git:changed");
        assert_eq!(v["projectId"], "p1");
        assert_eq!(v["workspaceId"], "w1");

        let ev = InternalEvent::FilesChanged {
            workspace_id: "w1".into(),
            paths: vec!["src/main.rs".into()],
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "files:changed");
        assert_eq!(v["paths"][0], "src/main.rs");

        // E4-09 PR sync events reach the frontend envelope.
        let ev = InternalEvent::PrUpdated {
            workspace_id: "w1".into(),
            pr_url: "https://github.com/o/r/pull/42".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "pr:updated");
        assert_eq!(v["workspaceId"], "w1");
        assert_eq!(v["prUrl"], "https://github.com/o/r/pull/42");

        // E4-10 line-comment events reach the frontend envelope.
        let ev = InternalEvent::CommentCreated {
            id: "lc_1".into(),
            task_id: "t1".into(),
            file_path: "src/main.rs".into(),
            line_number: 42,
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "comment:created");
        assert_eq!(v["taskId"], "t1");
        assert_eq!(v["filePath"], "src/main.rs");
        assert_eq!(v["lineNumber"], 42);

        let ev = InternalEvent::CommentResolved { id: "lc_1".into() };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "comment:resolved");
        assert_eq!(v["id"], "lc_1");

        // E18-04/05 step-engine events reach the frontend envelope.
        let ev = InternalEvent::StepLaunch {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
            task_id: "t1".into(),
            prompt: "go".into(),
            provider: "claude".into(),
            model: Some("haiku".into()),
            effort: None,
            reattached: false,
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "step:launch");
        assert_eq!(v["issueId"], "i1");
        assert_eq!(v["taskId"], "t1");
        assert_eq!(v["model"], "haiku");
        assert_eq!(v["reattached"], false);

        let ev = InternalEvent::StepQueued {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
            provider: "claude".into(),
            model: None,
            effort: Some("high".into()),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "step:queued");
        assert_eq!(v["columnId"], "c1");
        assert_eq!(v["effort"], "high");

        let ev = InternalEvent::StepQueueCleared {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
        };
        assert_eq!(event_to_value(&ev).unwrap()["type"], "step:queue_cleared");

        let ev = InternalEvent::StepSettled {
            issue_id: "i1".into(),
            project_id: "p1".into(),
            column_id: "c1".into(),
            task_id: "t1".into(),
        };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "step:settled");
        assert_eq!(v["taskId"], "t1");

        // 7a archive/restore events reach the frontend envelope.
        let ev = InternalEvent::TaskArchived { id: "t1".into() };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "task:archived");
        assert_eq!(v["taskId"], "t1");

        let ev = InternalEvent::TaskRestored { id: "t1".into() };
        let v = event_to_value(&ev).unwrap();
        assert_eq!(v["type"], "task:restored");
        assert_eq!(v["taskId"], "t1");

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
}
