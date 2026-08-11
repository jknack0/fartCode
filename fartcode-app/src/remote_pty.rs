//! Remote PTY routing (E12-05).
//!
//! One [`SshPtyManager`] per connection, created on first use and reused: a
//! terminal, its agent, and its lifecycle scripts share the host's SSH
//! session instead of paying a handshake each. Connection *states* (backoff,
//! reconnect, MaxSessions) are E12-06 — this cache only avoids redundant
//! connects and can forget a manager whose connection is gone.

use std::collections::HashMap;
use std::sync::Arc;

use fartcode_core::db::Db;
use fartcode_core::projects::remote::{remote_target_for_task, RemoteTarget};
use fartcode_core::ssh_connections::SshConnectionStore;
use fartcode_core::terminals::pty::PtyManager;
use fartcode_core::Error;
use fartcode_ssh::host::connect_profile;
use fartcode_ssh::pty::SshPtyManager;
use fartcode_ssh::tmux::RemoteTmux;
use parking_lot::Mutex;

/// A resolved remote route: which host manager to spawn on, that host's
/// tmux (E12-05 AC12), and where.
pub type RemoteRoute = (Arc<dyn PtyManager>, Arc<RemoteTmux>, RemoteTarget);

/// One connected host: its PTY manager and its tmux server view. Both are
/// bound to the same `SshClient`, so a terminal and the durability queries
/// about it never disagree about which machine they mean.
#[derive(Clone)]
struct HostEntry {
    pty: Arc<SshPtyManager>,
    tmux: Arc<RemoteTmux>,
}

/// Lazily-connected remote PTY managers, keyed by connection id.
pub struct RemotePtyRegistry {
    db: Arc<dyn Db>,
    connections: Arc<SshConnectionStore>,
    managers: Mutex<HashMap<String, HostEntry>>,
    /// Connections the user disconnected by hand (E12-05 AC13). Nothing
    /// reconnects them implicitly — not a terminal open, not boot
    /// rehydration — until an explicit `connect`.
    manual_disconnects: Mutex<std::collections::HashSet<String>>,
    runtime: tokio::runtime::Handle,
}

impl RemotePtyRegistry {
    pub fn new(
        db: Arc<dyn Db>,
        connections: Arc<SshConnectionStore>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            db,
            connections,
            managers: Mutex::new(HashMap::new()),
            manual_disconnects: Mutex::new(std::collections::HashSet::new()),
            runtime,
        }
    }

    /// The PTY manager and remote workspace for `task_id`, or `None` when the
    /// task is local.
    ///
    /// `Err` means "this task IS remote and the host is unreachable" — never
    /// a silent fall back to the local machine.
    pub fn route_for_task(&self, task_id: &str) -> Result<Option<RemoteRoute>, Error> {
        let Some(target) = remote_target_for_task(self.db.as_ref(), task_id)? else {
            return Ok(None);
        };
        let host = self.host_for_connection(&target.connection_id)?;
        Ok(Some((host.pty, host.tmux, target)))
    }

    /// tmux on the host a connection points at (E12-05 AC12) — used by
    /// teardown paths that no longer have a live terminal to ask.
    pub fn tmux_for_connection(&self, connection_id: &str) -> Result<Arc<RemoteTmux>, Error> {
        Ok(self.host_for_connection(connection_id)?.tmux)
    }

    fn host_for_connection(&self, connection_id: &str) -> Result<HostEntry, Error> {
        if let Some(existing) = self.managers.lock().get(connection_id).cloned() {
            return Ok(existing);
        }
        if self.manual_disconnects.lock().contains(connection_id) {
            return Err(Error::Internal(format!(
                "ssh connection disconnected: {connection_id} — reconnect to resume"
            )));
        }
        let profile = self
            .connections
            .get(connection_id)?
            .ok_or_else(|| Error::SshConnectionNotFound(connection_id.to_string()))?;

        // Connecting is async and this is a blocking call path (the terminal
        // manager runs on ordinary threads), so the handshake — and only the
        // handshake — blocks here.
        let client = tokio::task::block_in_place(|| {
            self.runtime
                .block_on(async { connect_profile(&profile).await })
        })?;
        let client = Arc::new(client);
        let entry = HostEntry {
            pty: Arc::new(SshPtyManager::with_runtime(
                client.clone(),
                self.runtime.clone(),
            )),
            tmux: Arc::new(RemoteTmux::new(client, self.runtime.clone())),
        };
        self.managers
            .lock()
            .insert(connection_id.to_string(), entry.clone());
        Ok(entry)
    }

    /// Drops the cached manager for a connection (profile edited, host gone).
    /// The next open reconnects.
    pub fn forget(&self, connection_id: &str) {
        self.managers.lock().remove(connection_id);
    }

    /// User-initiated disconnect (E12-05 AC13): drop the connection AND
    /// remember the intent, so no background path silently dials back.
    pub fn disconnect(&self, connection_id: &str) {
        self.managers.lock().remove(connection_id);
        self.manual_disconnects
            .lock()
            .insert(connection_id.to_string());
    }

    /// User-initiated (re)connect: clears the manual-disconnect intent and
    /// dials, so the caller learns immediately whether the host answers.
    pub fn connect(&self, connection_id: &str) -> Result<(), Error> {
        self.manual_disconnects.lock().remove(connection_id);
        self.host_for_connection(connection_id).map(|_| ())
    }

    /// Whether a connection is currently held open by this process.
    pub fn is_connected(&self, connection_id: &str) -> bool {
        self.managers.lock().contains_key(connection_id)
    }
}

impl fartcode_core::terminals::pty::RemotePtyLookup for RemotePtyRegistry {
    fn resolve(
        &self,
        task_id: &str,
    ) -> Result<Option<fartcode_core::terminals::pty::RemotePtyRoute>, Error> {
        Ok(self
            .route_for_task(task_id)?
            .map(|(manager, _tmux, target)| (manager, target.workspace_id)))
    }
}
