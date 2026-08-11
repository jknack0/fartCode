//! Port-forward service (E12-09): id-keyed tunnels over the pooled SSH
//! connections. The tunnel machinery lives in `fartcode_ssh::forward`; this
//! layer owns the records, the registry-backed dialer, and teardown when a
//! connection is dropped.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fartcode_core::Error;
use fartcode_ssh::forward::{
    open_tunnel, OpenTunnelOptions, PortForwardTunnel, TunnelDialer, TunnelStream,
};

use crate::remote_pty::RemotePtyRegistry;

/// What the frontend sees for one live forward.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardRecord {
    pub id: String,
    pub connection_id: String,
    pub remote_port: u16,
    pub local_port: u16,
}

struct Entry {
    record: PortForwardRecord,
    tunnel: PortForwardTunnel,
}

/// Dials direct-tcpip channels through the POOLED client (E12-06): every
/// dial asks the registry, so a reconnected session is picked up without
/// re-opening the tunnel, and channel refusals feed MaxSessions degradation
/// tracking exactly like terminal spawns do.
struct RegistryDialer {
    registry: Arc<RemotePtyRegistry>,
    connection_id: String,
}

#[async_trait]
impl TunnelDialer for RegistryDialer {
    async fn dial(&self, remote_host: &str, remote_port: u16) -> Result<TunnelStream, Error> {
        let client = self.registry.client_for(&self.connection_id).await?;
        match client.dial(remote_host, remote_port).await {
            Ok(stream) => {
                self.registry.report_channel_ok(&self.connection_id);
                Ok(stream)
            }
            Err(e) => {
                self.registry.report_channel_error(&self.connection_id, &e);
                Err(e)
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.registry.is_connected(&self.connection_id)
    }
}

/// Id-keyed live tunnels. `open` is idempotent per id (the reference
/// service's contract): re-opening an existing forward returns its record.
pub struct PortForwardService {
    registry: Arc<RemotePtyRegistry>,
    tunnels: Mutex<HashMap<String, Entry>>,
}

impl PortForwardService {
    pub fn new(registry: Arc<RemotePtyRegistry>) -> Self {
        Self {
            registry,
            tunnels: Mutex::new(HashMap::new()),
        }
    }

    pub async fn open(
        &self,
        id: &str,
        connection_id: &str,
        remote_port: u16,
        preferred_local_port: Option<u16>,
    ) -> Result<PortForwardRecord, Error> {
        if let Some(entry) = self.tunnels.lock().expect("port forward lock").get(id) {
            return Ok(entry.record.clone());
        }
        let dialer: Arc<dyn TunnelDialer> = Arc::new(RegistryDialer {
            registry: self.registry.clone(),
            connection_id: connection_id.to_string(),
        });
        let tunnel = open_tunnel(
            dialer,
            OpenTunnelOptions {
                remote_port,
                preferred_local_port,
            },
        )
        .await?;
        let record = PortForwardRecord {
            id: id.to_string(),
            connection_id: connection_id.to_string(),
            remote_port,
            local_port: tunnel.local_port(),
        };
        // A concurrent open for the same id may have won the race while the
        // listener bound; keep the FIRST tunnel (its port is already handed
        // out) and let ours drop-close.
        let mut tunnels = self.tunnels.lock().expect("port forward lock");
        if let Some(existing) = tunnels.get(id) {
            return Ok(existing.record.clone());
        }
        tunnels.insert(
            id.to_string(),
            Entry {
                record: record.clone(),
                tunnel,
            },
        );
        Ok(record)
    }

    /// Closes one forward. Unknown id is a no-op (already stopped).
    pub fn stop(&self, id: &str) {
        if let Some(entry) = self.tunnels.lock().expect("port forward lock").remove(id) {
            entry.tunnel.close();
        }
    }

    /// Tears down every forward on a connection — called when the user
    /// disconnects it (the tunnels would only spray doomed dials).
    pub fn stop_for_connection(&self, connection_id: &str) {
        let removed: Vec<Entry> = {
            let mut tunnels = self.tunnels.lock().expect("port forward lock");
            let ids: Vec<String> = tunnels
                .iter()
                .filter(|(_, e)| e.record.connection_id == connection_id)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| tunnels.remove(id)).collect()
        };
        for entry in removed {
            entry.tunnel.close();
        }
    }

    pub fn list(&self) -> Vec<PortForwardRecord> {
        let mut records: Vec<PortForwardRecord> = self
            .tunnels
            .lock()
            .expect("port forward lock")
            .values()
            .map(|e| e.record.clone())
            .collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));
        records
    }
}
