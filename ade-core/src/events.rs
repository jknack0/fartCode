//! Event bus contract (ARCHITECTURE.md §5, §6.6).
//!
//! Domain services emit `InternalEvent`s onto the bus; the Tauri layer
//! subscribes and forwards them to the frontend as typed events. The bus is
//! a `tokio::sync::broadcast` channel — senders never block and receivers
//! lag/drop when they fall behind (by design).

use serde::Serialize;

/// Events emitted by domain services, consumed by the Tauri command layer.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
#[serde(rename_all = "snake_case")]
pub enum InternalEvent {
    // Lifecycle
    AppStarted,
    AppClosed {
        was_crash: bool,
    },

    // Projects
    ProjectAdded {
        id: String,
        name: String,
        path: String,
    },
    ProjectDeleted {
        id: String,
    },

    // Tasks
    TaskCreated {
        id: String,
        project_id: String,
        name: String,
    },
    TaskProvisioned {
        id: String,
        workspace_id: String,
    },
    TaskStatusChanged {
        id: String,
        old_status: String,
        new_status: String,
    },
    TaskArchived {
        id: String,
    },
    TaskDeleted {
        id: String,
    },

    // Conversations
    ConversationCreated {
        id: String,
        task_id: String,
        provider: String,
        title: String,
    },
    ConversationRenamed {
        id: String,
        title: String,
    },
    ConversationDeleted {
        id: String,
    },

    // Agent lifecycle
    /// E2-04: emitted when a task is created with a pty initial prompt, so
    /// E2-06's agent launcher starts the agent with that prompt.
    AgentStart {
        provider: String,
        project_id: String,
        task_id: String,
        conversation_id: String,
    },
    AgentRunStarted {
        conversation_id: String,
        provider: String,
    },
    AgentRunFinished {
        conversation_id: String,
        provider: String,
        exit_code: i32,
    },
    AgentSessionExited {
        conversation_id: String,
    },

    // PTY
    PtyOutput {
        pty_id: String,
        data: Vec<u8>,
    },
    PtyClosed {
        pty_id: String,
    },

    // Terminals
    TerminalCreated {
        id: String,
        task_id: String,
    },
    TerminalDeleted {
        id: String,
    },
    /// E1-06 lifecycle scripts: status is one of `running|succeeded|failed|stopped`.
    LifecycleScriptStatusChanged {
        session_id: String,
        script_type: String,
        status: String,
    },

    // Git
    GitChanged {
        project_id: String,
        workspace_id: String,
    },

    // Settings
    SettingChanged {
        key: String,
    },

    // UI state
    SidebarToggled {
        visible: bool,
    },

    // Errors
    Error {
        context: String,
        message: String,
    },
}

/// Fan-out bus for `InternalEvent`s (ARCHITECTURE.md §6.6).
pub trait EventBus: Send + Sync {
    /// Subscribe to future events. Returns a lagging receiver; events sent
    /// while a receiver is full are dropped for that receiver.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<InternalEvent>;

    /// Publish an event. Never blocks; drops for lagging receivers.
    fn send(&self, event: InternalEvent);
}

/// Concrete broadcast-channel implementation, shared via `Arc`.
pub struct BroadcastEventBus {
    tx: tokio::sync::broadcast::Sender<InternalEvent>,
}

impl BroadcastEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = tokio::sync::broadcast::channel(capacity);
        Self { tx }
    }
}

impl EventBus for BroadcastEventBus {
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<InternalEvent> {
        self.tx.subscribe()
    }

    fn send(&self, event: InternalEvent) {
        let _ = self.tx.send(event); // ignore if no receivers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_roundtrip_through_the_bus() {
        let bus = BroadcastEventBus::new(16);
        let mut rx = bus.subscribe();

        bus.send(InternalEvent::ProjectAdded {
            id: "p1".into(),
            name: "demo".into(),
            path: "/tmp/demo".into(),
        });

        match rx.try_recv() {
            Ok(InternalEvent::ProjectAdded { id, name, .. }) => {
                assert_eq!(id, "p1");
                assert_eq!(name, "demo");
            }
            other => panic!("expected ProjectAdded, got {other:?}"),
        }
    }

    #[test]
    fn event_serializes_with_type_tag() {
        let event = InternalEvent::ProjectDeleted { id: "p1".into() };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("project_deleted"), "snake_case tag: {json}");
    }
}
