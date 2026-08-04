//! ade-app ↔ ACP worker wire schema (E2-11-2).
//!
//! **Env security invariant:** [`SessionInput::env`] exists in the
//! renderer-facing schema only so the renderer's mistake of sending env is
//! visible and testable — [`prepare_session`] ALWAYS discards it. The env
//! that reaches the adapter comes exclusively from server-side provider
//! settings (E3-07 keyring-backed `resolve_env`).

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// One queued prompt (drained by the SessionManager, E2-11-3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedPrompt {
    /// Prompt text.
    pub text: String,
}

/// The renderer-supplied session request. `env` is UNTRUSTED input and is
/// dropped by [`prepare_session`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInput {
    /// Conversation this session belongs to.
    pub conversation_id: String,
    /// Owning project.
    pub project_id: String,
    /// Owning task.
    pub task_id: String,
    /// Provider registry id (e.g. "claude").
    pub provider_id: String,
    /// Workspace row id.
    pub workspace_id: String,
    /// Working directory for the adapter session (worktree path).
    pub cwd: String,
    /// Provider session id to resume, if any (persisted per E2-11-3).
    #[serde(default)]
    pub session_id: Option<String>,
    /// Model override, if any.
    #[serde(default)]
    pub model: Option<String>,
    /// Prompts queued ahead of the first turn.
    #[serde(default)]
    pub initial_queue: Vec<QueuedPrompt>,
    /// Adapter binary path (resolved by the launcher, E3-02 host deps).
    pub adapter_path: String,
    /// Adapter argv.
    #[serde(default)]
    pub adapter_args: Vec<String>,
    /// UNTRUSTED — always discarded server-side. Env reaching the adapter
    /// comes only from provider settings.
    #[serde(default)]
    pub env: Vec<EnvPair>,
}

/// An env var pair on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvPair {
    /// Variable name.
    pub key: String,
    /// Variable value.
    pub value: String,
}

/// The worker-side session description: renderer input minus `env`, plus
/// the server-resolved provider env. This is what crosses into the worker
/// process.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSession {
    /// Conversation this session belongs to.
    pub conversation_id: String,
    /// Owning project.
    pub project_id: String,
    /// Owning task.
    pub task_id: String,
    /// Provider registry id.
    pub provider_id: String,
    /// Workspace row id.
    pub workspace_id: String,
    /// Working directory for the adapter session.
    pub cwd: String,
    /// Provider session id to resume, if any.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Model override, if any.
    #[serde(default)]
    pub model: Option<String>,
    /// Prompts queued ahead of the first turn.
    #[serde(default)]
    pub initial_queue: Vec<QueuedPrompt>,
    /// Adapter binary path.
    pub adapter_path: String,
    /// Adapter argv.
    #[serde(default)]
    pub adapter_args: Vec<String>,
    /// Server-resolved env (provider credentials). The ONLY env the adapter
    /// receives.
    #[serde(default)]
    pub env: Vec<EnvPair>,
}

/// Validates the renderer input and produces the worker session.
///
/// `provider_env` is the server-side resolution (E3-07). `input.env` is
/// discarded unconditionally — the renderer cannot inject env.
pub fn prepare_session(
    input: SessionInput,
    provider_env: &[(String, String)],
) -> Result<WorkerSession, Error> {
    if input.conversation_id.trim().is_empty() {
        return Err(Error::Protocol("conversationId is required".into()));
    }
    if input.cwd.trim().is_empty() {
        return Err(Error::Protocol("cwd is required".into()));
    }
    if input.adapter_path.trim().is_empty() {
        return Err(Error::Protocol("adapterPath is required".into()));
    }
    // input.env is deliberately ignored — see module docs.
    Ok(WorkerSession {
        conversation_id: input.conversation_id,
        project_id: input.project_id,
        task_id: input.task_id,
        provider_id: input.provider_id,
        workspace_id: input.workspace_id,
        cwd: input.cwd,
        session_id: input.session_id,
        model: input.model,
        initial_queue: input.initial_queue,
        adapter_path: input.adapter_path,
        adapter_args: input.adapter_args,
        env: provider_env
            .iter()
            .map(|(key, value)| EnvPair {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
    })
}

/// Worker ↔ host method names.
pub mod worker_methods {
    /// Host → worker.
    pub const SESSION_START: &str = "session/start";
    pub const SESSION_PROMPT: &str = "session/prompt";
    pub const SESSION_CANCEL: &str = "session/cancel";
    pub const SESSION_STOP: &str = "session/stop";

    /// Worker → host.
    pub const WORKER_UPDATE: &str = "worker/update";
    pub const WORKER_PERMISSION_REQUEST: &str = "worker/permission_request";
    pub const WORKER_EXITED: &str = "worker/exited";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(env: Vec<EnvPair>) -> SessionInput {
        SessionInput {
            conversation_id: "conv-1".into(),
            project_id: "proj-1".into(),
            task_id: "task-1".into(),
            provider_id: "claude".into(),
            workspace_id: "ws-1".into(),
            cwd: "/tmp/work".into(),
            session_id: None,
            model: None,
            initial_queue: vec![QueuedPrompt { text: "hi".into() }],
            adapter_path: "/usr/local/bin/fake-adapter".into(),
            adapter_args: vec![],
            env,
        }
    }

    #[test]
    fn prepare_session_discards_renderer_env() {
        let hostile = input(vec![
            EnvPair {
                key: "ANTHROPIC_API_KEY".into(),
                value: "renderer-injected".into(),
            },
            EnvPair {
                key: "LD_PRELOAD".into(),
                value: "/tmp/evil.so".into(),
            },
        ]);
        let prepared = prepare_session(
            hostile,
            &[("ANTHROPIC_API_KEY".into(), "keyring-secret".into())],
        )
        .unwrap();
        assert_eq!(prepared.env.len(), 1, "renderer env must be discarded");
        assert_eq!(prepared.env[0].key, "ANTHROPIC_API_KEY");
        assert_eq!(prepared.env[0].value, "keyring-secret");
    }

    #[test]
    fn prepare_session_validates_required_fields() {
        let mut bad = input(vec![]);
        bad.conversation_id = " ".into();
        assert!(prepare_session(bad, &[]).is_err());
        let mut bad = input(vec![]);
        bad.cwd = "".into();
        assert!(prepare_session(bad, &[]).is_err());
        let mut bad = input(vec![]);
        bad.adapter_path = "".into();
        assert!(prepare_session(bad, &[]).is_err());
    }

    #[test]
    fn worker_session_round_trips_wire_json() {
        let prepared = prepare_session(input(vec![]), &[("K".into(), "V".into())]).unwrap();
        let json = serde_json::to_string(&prepared).unwrap();
        assert!(json.contains("\"conversationId\":\"conv-1\""));
        let parsed: WorkerSession = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.cwd, prepared.cwd);
        assert_eq!(parsed.initial_queue.len(), 1);
    }
}
