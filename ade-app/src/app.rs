//! ade-app — application bootstrap (ARCHITECTURE §7).
//!
//! The single place concrete domain services are wired together and shared
//! via `Arc`. `App::init` builds everything from a DB path; the Tauri setup
//! hook manages it as state and forwards internal events to the frontend.

use std::sync::Arc;

use ade_core::conversations::DbConversationStore;
use ade_core::db::{Db, SqliteDb};
use ade_core::dependencies::{HostDependencyStore, ProcessInstallRunner};
use ade_core::events::EventBus;
use ade_core::events::{BroadcastEventBus, InternalEvent};
use ade_core::fs_watch::FsWatchService;
use ade_core::projects::worktrees::WorktreeManager;
use ade_core::projects::DbProjectStore;
use ade_core::provider_accounts::ProviderAccountStore;
use ade_core::pty::launcher::{NoopRemoteRehydrate, Rehydrator};
use ade_core::pty::sessions::SessionRegistry;
use ade_core::settings::DbSettingsStore;
use ade_core::tasks::deletion::TaskDeletionService;
use ade_core::tasks::operations::TaskCreationService;
use ade_core::tasks::DbTaskStore;

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
    /// E4-01 workspace file+git watcher (registered via `watchers.rs`).
    pub fs_watch: Arc<FsWatchService>,
    /// E4-10 diff line comments (§14).
    pub line_comments: Arc<ade_core::line_comments::LineCommentStore>,
    /// E17-01 project board issues (§13).
    pub issues: Arc<ade_core::issues::IssueStore>,
}

impl App {
    /// `db_path`: explicit path (tests / `ADE_DB_FILE`), else the platform
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
            Arc::new(ade_git::CliGit),
            event_bus.clone(),
        ));
        let tasks = Arc::new(DbTaskStore::new(db.clone(), event_bus.clone()));
        let conversations = Arc::new(DbConversationStore::new(db.clone(), event_bus.clone()));
        let provider_accounts = Arc::new(ProviderAccountStore::new(db.clone()));

        // E2-09: one registry shared by boot rehydration (launches register)
        // and task deletion (cancel + reap).
        let sessions = Arc::new(SessionRegistry::new());

        // E2-07 boot rehydration: previously-spawned PTY conversations are
        // resumed after DB init (reference boot order). The app shell calls
        // `rehydrate_all` on a background thread (each launch blocks).
        let rehydrator = Rehydrator::new(
            Arc::new(ade_terminal::PortablePtyManager),
            Arc::new(HostDependencyStore::new(
                db.clone(),
                Arc::new(ProcessInstallRunner),
            )),
            event_bus.clone(),
            conversations.clone(),
            tasks.clone(),
            projects.clone(),
            db.clone(),
            false, // auto-approve defaults off on boot
            Arc::new(NoopRemoteRehydrate),
            Some(sessions.clone()),
        );

        // E2-09 task deletion/teardown.
        let worktrees =
            WorktreeManager::new(db.clone(), settings.clone(), Arc::new(ade_git::CliGit));
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
            Arc::new(ade_git::CliGit),
            WorktreeManager::new(db.clone(), settings.clone(), Arc::new(ade_git::CliGit)),
            event_bus.clone(),
        );

        // E4-01: file+git event watcher → live refresh pipeline. Lifecycle
        // (boot backfill, provision/delete hooks) is wired in watchers.rs.
        let fs_watch = Arc::new(
            FsWatchService::new(event_bus.clone() as Arc<dyn EventBus>)
                .map_err(|e| e.to_string())?,
        );

        // E4-10: diff line comments (§14) — CRUD + bidirectional task link.
        let line_comments = Arc::new(ade_core::line_comments::LineCommentStore::new(
            db.clone(),
            event_bus.clone() as Arc<dyn EventBus>,
        ));
        // E17-01: project board issues (§13) — local-first store, derived
        // blocked state, cycle-rejected edges.
        let issues = Arc::new(ade_core::issues::IssueStore::new(
            db.clone(),
            event_bus.clone() as Arc<dyn EventBus>,
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
            fs_watch,
            line_comments,
            issues,
        }))
    }
}

/// Serializes an internal event for the frontend channel (`ade:event`).
/// Only the events the Phase-0 UI consumes are mapped; the rest are skipped.
pub fn event_to_value(event: &InternalEvent) -> Option<serde_json::Value> {
    use serde_json::json;
    match event {
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
        _ => None,
    }
}

/// Spawns the event-forwarding task: internal events → `ade:event` on the
/// Tauri emitter (runs for the app's lifetime).
pub fn spawn_event_forwarder(app_handle: tauri::AppHandle, event_bus: Arc<BroadcastEventBus>) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        let mut rx = event_bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Some(value) = event_to_value(&event) {
                        let _ = app_handle.emit("ade:event", value);
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
    use ade_core::events::InternalEvent;

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
