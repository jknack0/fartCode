//! Tauri emission of ACP session events (ticket E2-11-4).
//!
//! The `ade-acp` runtime is transport-agnostic (ADR-0027); this module is
//! the app-layer [`ade_acp::SessionEvents`] implementation that turns its
//! three seams into typed Tauri events, keyed by `conversationId`, the way
//! every other long-lived stream flows to the frontend (PRD §4.1):
//!
//! | seam                    | Tauri event               | payload                              |
//! |-------------------------|---------------------------|--------------------------------------|
//! | `update`                | `acp:update`              | raw `session/update`                 |
//! | `transcript_changed`    | `acp:transcript`          | full live-model snapshot             |
//! | `permission_requested`  | `acp:permission_request`  | pending request + options            |
//!
//! #32 hands [`TauriAcpEvents`] to the `SessionManager` via
//! `StartInput::events`; the frontend subscribes with `listen()` (E2-11-6).
//! `acp:transcript` carries the FULL snapshot on every change — the
//! frontend replaces its store from it (cheap for Phase-2 transcript
//! sizes; #33 may add diffing/throttling if needed).

use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter as _, Manager as _};

use ade_acp::client::SessionUpdateEvent;
use ade_acp::session::{LiveModels, PermissionRequestedEvent, SessionEvents};
use ade_acp::transcript::TranscriptTurnOutcome;

/// E17-03: the id of a turn that just settled Done (last committed turn,
/// nothing in flight). Edge detection per conversation lives in
/// TauriAcpEvents::flipped_turns.
fn newly_done_turn(models: &LiveModels) -> Option<String> {
    if models.active_turn.is_some() || models.session_state.is_generating {
        return None;
    }
    let last = models.committed.last()?;
    matches!(last.outcome, Some(TranscriptTurnOutcome::Done { .. })).then(|| last.id.clone())
}

/// `acp:update` payload — one routed raw `session/update`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpUpdatePayload {
    pub conversation_id: String,
    pub session_id: String,
    /// The raw update, verbatim (debuggability first; the reduced
    /// transcript travels over `acp:transcript`).
    pub update: serde_json::Value,
}

/// `acp:transcript` payload — the full live-model snapshot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpTranscriptPayload {
    pub conversation_id: String,
    pub models: LiveModels,
}

/// `acp:permission_request` payload.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPermissionPayload {
    pub conversation_id: String,
    pub request_id: String,
    pub pending: ade_acp::PendingPermission,
}

/// Builds the `acp:update` payload (`None` when the update is not
/// serializable — malformed frames never reach the reducer anyway).
pub fn update_payload(
    conversation_id: &str,
    event: &SessionUpdateEvent,
) -> Option<AcpUpdatePayload> {
    Some(AcpUpdatePayload {
        conversation_id: conversation_id.to_string(),
        session_id: event.session_id.0.to_string(),
        update: serde_json::to_value(&event.update).ok()?,
    })
}

pub fn transcript_payload(conversation_id: &str, models: &LiveModels) -> AcpTranscriptPayload {
    AcpTranscriptPayload {
        conversation_id: conversation_id.to_string(),
        models: models.clone(),
    }
}

pub fn permission_payload(
    conversation_id: &str,
    event: &PermissionRequestedEvent,
) -> AcpPermissionPayload {
    AcpPermissionPayload {
        conversation_id: conversation_id.to_string(),
        request_id: event.request_id.clone(),
        pending: event.pending.clone(),
    }
}

/// Tauri-backed [`SessionEvents`]. Created once in the setup hook and
/// managed as state; #32 passes it into `StartInput::events`.
pub struct TauriAcpEvents {
    app: tauri::AppHandle,
    /// E17-03: conversation_id → last turn id that flipped its linked
    /// issue, so repeated snapshots of a settled turn flip exactly once.
    flipped_turns: parking_lot::Mutex<std::collections::HashMap<String, String>>,
}

impl TauriAcpEvents {
    pub fn new(app: tauri::AppHandle) -> Arc<Self> {
        Arc::new(Self {
            app,
            flipped_turns: parking_lot::Mutex::new(std::collections::HashMap::new()),
        })
    }
}

impl SessionEvents for TauriAcpEvents {
    fn update(&self, conversation_id: &str, event: &SessionUpdateEvent) {
        if let Some(payload) = update_payload(conversation_id, event) {
            let _ = self.app.emit("acp:update", payload);
        }
    }

    fn transcript_changed(&self, conversation_id: &str, models: &LiveModels) {
        let _ = self.app.emit(
            "acp:transcript",
            transcript_payload(conversation_id, models),
        );
        // E17-03 auto-flip: a turn settling Done flips the task's linked
        // issues In Progress → In Review (once per turn).
        if let Some(turn_id) = newly_done_turn(models) {
            let mut flipped = self.flipped_turns.lock();
            if flipped.get(conversation_id) != Some(&turn_id) {
                flipped.insert(conversation_id.to_string(), turn_id);
                drop(flipped);
                if let Some(app) = self.app.try_state::<std::sync::Arc<crate::app::App>>() {
                    crate::dispatch::flip_issues_for_conversation(&app, conversation_id);
                }
            }
        }
    }

    fn permission_requested(&self, conversation_id: &str, event: &PermissionRequestedEvent) {
        let _ = self.app.emit(
            "acp:permission_request",
            permission_payload(conversation_id, event),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ade_acp::session::{PendingPermission, QueuedPrompt, SessionState};
    use ade_acp::Lifecycle;
    use agent_client_protocol_schema::v1::{SessionId, SessionUpdate};

    fn live_models() -> LiveModels {
        LiveModels {
            session_state: SessionState {
                lifecycle: Lifecycle::Ready,
                active_turn_id: None,
                pending_permissions: Vec::new(),
                last_stop_reason: Some("end_turn".into()),
                queued_prompts: Vec::new(),
                is_generating: false,
                can_submit: true,
                can_cancel: false,
            },
            committed: Vec::new(),
            active_turn: None,
            config: Default::default(),
            usage: None,
            title: Some("My session".into()),
            agents: Vec::new(),
            plan: None,
            draft: None,
            queued_prompts: vec![QueuedPrompt {
                id: "q1".into(),
                text: "queued".into(),
                hidden_context: None,
            }],
            terminals: Vec::new(),
        }
    }

    #[test]
    fn payloads_are_camel_case_and_keyed_by_conversation() {
        let event = SessionUpdateEvent {
            session_id: SessionId::new("sess-1"),
            update: serde_json::from_value::<SessionUpdate>(serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "hi" },
            }))
            .unwrap(),
        };
        let payload = serde_json::to_value(update_payload("conv-1", &event).unwrap()).unwrap();
        assert_eq!(payload["conversationId"], "conv-1");
        assert_eq!(payload["sessionId"], "sess-1");
        assert_eq!(payload["update"]["sessionUpdate"], "agent_message_chunk");

        let models = serde_json::to_value(transcript_payload("conv-1", &live_models())).unwrap();
        assert_eq!(models["conversationId"], "conv-1");
        assert_eq!(models["models"]["title"], "My session");
        assert_eq!(models["models"]["sessionState"]["lifecycle"], "ready");
        assert_eq!(
            models["models"]["sessionState"]["lastStopReason"],
            "end_turn"
        );
        assert_eq!(models["models"]["queuedPrompts"][0]["id"], "q1");
        assert_eq!(models["models"]["terminals"], serde_json::json!([]));
    }

    #[test]
    fn permission_payload_flattens_request_id() {
        let pending = PendingPermission {
            request_id: "req-1".into(),
            session_id: SessionId::new("sess-1"),
            tool_call: serde_json::from_value(serde_json::json!({
                "toolCallId": "call_1",
            }))
            .unwrap(),
            options: Vec::new(),
        };
        let event = PermissionRequestedEvent {
            request_id: "req-1".into(),
            pending,
        };
        let payload = serde_json::to_value(permission_payload("conv-1", &event)).unwrap();
        assert_eq!(payload["conversationId"], "conv-1");
        assert_eq!(payload["requestId"], "req-1");
        assert_eq!(payload["pending"]["toolCall"]["toolCallId"], "call_1");
    }
}
