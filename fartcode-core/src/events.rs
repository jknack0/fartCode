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
    /// `archived_at` cleared — the task is back on the board.
    TaskRestored {
        id: String,
    },
    TaskDeleted {
        id: String,
    },

    // Line comments (E4-10, ARCHITECTURE.md §14)
    CommentCreated {
        id: String,
        task_id: String,
        file_path: String,
        line_number: i64,
    },
    CommentResolved {
        id: String,
    },

    // Issues (E17, ARCHITECTURE.md §13)
    IssueCreated {
        id: String,
        project_id: String,
        title: String,
    },
    /// Fields, lane, edges, or the linked task changed — consumers refetch
    /// the project's issue list (blocked status is derived, so a lane move
    /// can change OTHER cards' badges too).
    IssueUpdated {
        id: String,
        project_id: String,
    },
    IssueDeleted {
        id: String,
        project_id: String,
    },
    /// A card CHANGED COLUMN — emitted by the enter primitive alongside
    /// the coarse `IssueUpdated`, which says only "something about this
    /// card changed".
    ///
    /// Both endpoints ride the event because only the emitter knows them:
    /// the writer holds `previous_column` and the target in the same
    /// transaction, while any later reader sees just the current column
    /// and has to guess what it moved from (E19-01 fix round — the
    /// dossier appender used to diff against an in-memory map, which was
    /// wrong under rapid moves and unbounded in memory). `from` is `None`
    /// only for a card that had no column, which the E18-07 backfill
    /// makes an out-of-contract row.
    ///
    /// Not forwarded to the frontend: `IssueUpdated` already fires for the
    /// same move and is what the UI refetches on.
    IssueColumnChanged {
        id: String,
        project_id: String,
        from: Option<String>,
        to: String,
    },

    // Step engine (E18-04/05, #63/#64; ADR-0037 items 2–4)
    /// Directive event: an agent step wants an agent running — the
    /// frontend opens (or, when `reattached`, focuses) the task's agent
    /// terminal with `prompt` and the resolved provider/model/effort.
    /// (Named as a directive, unlike the state-change events around it:
    /// the launch has not happened yet — the terminal is a UI resource.)
    ///
    /// Until the UI wave subscribes to this event, settle-chained
    /// launches are DEFERRED directives, not lost state: the engine's
    /// launch registry marks them undelivered, and re-entering the column
    /// re-emits the directive (and returns it through the command
    /// outcome, which the current UI does consume).
    StepLaunch {
        issue_id: String,
        project_id: String,
        column_id: String,
        task_id: String,
        prompt: String,
        provider: String,
        model: Option<String>,
        effort: Option<String>,
        reattached: bool,
    },
    /// A queue-mode agent step parked awaiting `step_confirm` (the 8c
    /// confirm UI). Carries the resolved agent config so the overlay can
    /// name provider/model before fire (ADR-0037 cost doctrine).
    StepQueued {
        issue_id: String,
        project_id: String,
        column_id: String,
        provider: String,
        model: Option<String>,
        effort: Option<String>,
    },
    /// A parked step was dropped without launching (manual drag override,
    /// or superseded by another entry).
    StepQueueCleared {
        issue_id: String,
        project_id: String,
        column_id: String,
    },
    /// A hold-mode agent step settled. Step-done is DERIVED state — this
    /// event exists for the UI; nothing is stored.
    StepSettled {
        issue_id: String,
        project_id: String,
        column_id: String,
        task_id: String,
    },
    /// The chain guard held a card instead of chaining the next automatic
    /// launch (#82): depth cap reached, `advance_to` cycle detected, or
    /// the project's token budget exhausted. The card stays on
    /// `column_id`; `target_column_id` is the launch that was refused.
    /// The refusal is also a durable `step_ledger` hold row — this event
    /// is the live announcement, the ledger the record.
    StepChainHeld {
        issue_id: String,
        project_id: String,
        column_id: String,
        target_column_id: Option<String>,
        /// `"depth"` | `"cycle"` | `"budget"`.
        reason: String,
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
    /// E4-09: a PR row changed in the sync cache (or appeared). Consumers
    /// refetch the workspace's cached PR section.
    PrUpdated {
        workspace_id: String,
        pr_url: String,
    },
    /// E4-01: working files changed in a workspace's worktree. `paths` are
    /// worktree-relative, deduped, and capped (a hint for editors; consumers
    /// needing exact state refetch).
    FilesChanged {
        workspace_id: String,
        paths: Vec<String>,
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
