//! ade-app — application bootstrap (ARCHITECTURE §7).
//!
//! The single place concrete domain services are wired together and shared
//! via `Arc`. `App::init` builds everything from a DB path; the Tauri setup
//! hook manages it as state and forwards internal events to the frontend.

use std::sync::Arc;

use ade_core::conversations::DbConversationStore;
use ade_core::db::{Db, SqliteDb};
use ade_core::events::EventBus;
use ade_core::events::{BroadcastEventBus, InternalEvent};
use ade_core::projects::DbProjectStore;
use ade_core::settings::DbSettingsStore;
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

        Ok(Arc::new(Self {
            db,
            settings,
            projects,
            tasks,
            conversations,
            event_bus,
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
            id,
            task_id,
            provider,
        } => Some(json!({
            "type": "conversation:created", "id": id, "taskId": task_id, "provider": provider,
        })),
        InternalEvent::ConversationDeleted { id } => {
            Some(json!({ "type": "conversation:deleted", "id": id }))
        }
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
