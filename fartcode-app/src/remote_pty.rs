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
use fartcode_core::projects::remote::remote_target_for_task;
use fartcode_core::ssh_connections::SshConnectionStore;
use fartcode_core::terminals::pty::PtyManager;
use fartcode_core::Error;
use fartcode_ssh::host::connect_profile;
use fartcode_ssh::pty::SshPtyManager;
use parking_lot::Mutex;

/// Lazily-connected remote PTY managers, keyed by connection id.
pub struct RemotePtyRegistry {
    db: Arc<dyn Db>,
    connections: Arc<SshConnectionStore>,
    managers: Mutex<HashMap<String, Arc<SshPtyManager>>>,
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
            runtime,
        }
    }

    /// The PTY manager for `task_id`, or `None` when the task is local.
    ///
    /// `Err` means "this task IS remote and the host is unreachable" — never
    /// a silent fall back to the local machine.
    pub fn manager_for_task(&self, task_id: &str) -> Result<Option<Arc<dyn PtyManager>>, Error> {
        let Some(target) = remote_target_for_task(self.db.as_ref(), task_id)? else {
            return Ok(None);
        };
        Ok(Some(self.manager_for_connection(&target.connection_id)?))
    }

    fn manager_for_connection(&self, connection_id: &str) -> Result<Arc<dyn PtyManager>, Error> {
        if let Some(existing) = self.managers.lock().get(connection_id).cloned() {
            return Ok(existing);
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
        let manager = Arc::new(SshPtyManager::with_runtime(
            Arc::new(client),
            self.runtime.clone(),
        ));
        self.managers
            .lock()
            .insert(connection_id.to_string(), manager.clone());
        Ok(manager)
    }

    /// Drops the cached manager for a connection (profile edited, host gone).
    /// The next open reconnects.
    pub fn forget(&self, connection_id: &str) {
        self.managers.lock().remove(connection_id);
    }
}
