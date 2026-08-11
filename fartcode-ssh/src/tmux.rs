//! Remote tmux durability (E12-05 AC12).
//!
//! The local path resolves a tmux binary on THIS machine and shells out; a
//! remote session lives on the host's tmux server, so every query is an SSH
//! `exec`. Command lines and parsing come from `fartcode_core::pty::tmux` —
//! one session namespace, one format, whichever transport asks.
//!
//! Every call is best-effort by design: no tmux, no server, or a host that
//! went away all read as "no sessions", exactly like the local degradation.
//! Availability is probed once per connection and cached (a host does not
//! grow a tmux mid-session).

use std::sync::Arc;
use std::sync::OnceLock;

use fartcode_core::pty::tmux::{
    make_tmux_session_name, parse_tmux_session_rows, remote_tmux_kill_command,
    remote_tmux_list_command, remote_tmux_probe_command, sessions_from_rows,
    tmux_sessions_matching, TmuxSessionInfo,
};

use crate::SshClient;

/// tmux on one connected host.
pub struct RemoteTmux {
    client: Arc<SshClient>,
    runtime: tokio::runtime::Handle,
    available: OnceLock<bool>,
}

impl RemoteTmux {
    pub fn new(client: Arc<SshClient>, runtime: tokio::runtime::Handle) -> Self {
        Self {
            client,
            runtime,
            available: OnceLock::new(),
        }
    }

    /// Whether the host has tmux (cached `command -v tmux`). A probe that
    /// fails to run at all is "no tmux" — the terminal still opens, plain.
    pub fn available(&self) -> bool {
        *self.available.get_or_init(|| {
            self.run(&remote_tmux_probe_command())
                .map(|out| !out.trim().is_empty())
                .unwrap_or(false)
        })
    }

    /// Live fartCode sessions whose decoded id starts with `id_prefix`.
    pub fn list_sessions_by_prefix(&self, id_prefix: &str) -> Vec<TmuxSessionInfo> {
        if !self.available() {
            return Vec::new();
        }
        let Ok(stdout) = self.run(&remote_tmux_list_command()) else {
            return Vec::new();
        };
        sessions_from_rows(&parse_tmux_session_rows(&stdout), id_prefix)
    }

    /// Kills one session by its encoded name. Idempotent.
    pub fn kill_session(&self, session_name: &str) {
        if !self.available() {
            return;
        }
        if let Err(e) = self.run(&remote_tmux_kill_command(session_name)) {
            tracing::warn!(session = %session_name, error = %e, "remote tmux kill-session failed");
        }
    }

    /// Kills the session for a decoded session id.
    pub fn kill_session_id(&self, session_id: &str) {
        self.kill_session(&make_tmux_session_name(session_id));
    }

    /// Sweeps every fartCode session whose decoded id starts with
    /// `id_prefix` (task teardown, orphans from a crashed app included).
    pub fn kill_sessions_by_prefix(&self, id_prefix: &str) -> usize {
        if !self.available() {
            return 0;
        }
        let Ok(stdout) = self.run(&remote_tmux_list_command()) else {
            return 0;
        };
        let rows = parse_tmux_session_rows(&stdout);
        let names = tmux_sessions_matching(rows.iter().map(|(n, _)| n.as_str()), id_prefix);
        for name in &names {
            self.kill_session(name);
        }
        names.len()
    }

    /// One blocking `exec` on the host. The handshake already happened —
    /// only this round trip blocks the calling thread.
    fn run(&self, command: &str) -> Result<String, fartcode_core::Error> {
        let client = self.client.clone();
        let command = command.to_string();
        tokio::task::block_in_place(|| {
            self.runtime
                .block_on(async move { client.run_command(&command).await })
        })
    }
}
