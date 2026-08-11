//! Remote connection lifecycle and PTY routing (E12-05, E12-06).
//!
//! One pooled [`SshClient`] per connection id, shared by that host's PTY
//! manager, tmux view, and the remote-project commands: a terminal, its
//! agent, its lifecycle scripts and a directory listing all ride the same
//! session instead of paying a handshake each.
//!
//! **Lifecycle (E12-06).** A watchdog polls each live client. An unexpected
//! drop walks the reconnect ladder (1/2/5/10/20 s, one attempt per rung) and
//! rebinds the pool entry on success — remote work resumes without an app
//! restart. Exhausting the ladder parks the connection in `Error`; a manual
//! `disconnect` parks it in `Disconnected` and suppresses every implicit dial
//! until an explicit `connect`.
//!
//! What rehydrate does NOT do: reattach terminals. A terminal's PTY died with
//! the old session; its scrollback and process live in the host's tmux server
//! (ADR-0025, E12-05 AC12), so reopening the terminal attaches back. The pool
//! only guarantees the next open reaches a live host.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use fartcode_core::db::Db;
use fartcode_core::events::{EventBus, InternalEvent};
use fartcode_core::projects::remote::{remote_target_for_task, RemoteTarget};
use fartcode_core::ssh_connections::{ConnectionState, SshConnectionStore};
use fartcode_core::terminals::pty::PtyManager;
use fartcode_core::Error;
use fartcode_ssh::host::connect_profile;
use fartcode_ssh::pty::SshPtyManager;
use fartcode_ssh::tmux::RemoteTmux;
use fartcode_ssh::{is_channel_open_failure, SshClient};
use parking_lot::Mutex;

/// Wait before each reconnect attempt, in ms (E12-06 AC4). The array length
/// is the attempt cap: five rungs, then the connection is `Error`.
const RECONNECT_DELAYS_MS: [u64; 5] = [1_000, 2_000, 5_000, 10_000, 20_000];

/// How often the watchdog asks a pooled client whether it is still alive.
/// russh reports a dead session synchronously, so this costs a flag read.
const WATCHDOG_POLL: Duration = Duration::from_secs(2);

/// A resolved remote route: which host manager to spawn on, that host's
/// tmux (E12-05 AC12), and where.
pub type RemoteRoute = (Arc<dyn PtyManager>, Arc<RemoteTmux>, RemoteTarget);

/// One connected host. Everything here is bound to the same `SshClient`, so
/// a terminal and the durability queries about it never disagree about which
/// machine — or which session — they mean.
#[derive(Clone)]
struct HostEntry {
    client: Arc<SshClient>,
    pty: Arc<SshPtyManager>,
    tmux: Arc<RemoteTmux>,
    /// Bumped on every dial. A watchdog holding a stale generation exits
    /// instead of racing the connection that replaced it.
    generation: u64,
}

/// Pooled remote connections, keyed by connection id.
pub struct RemotePtyRegistry {
    db: Arc<dyn Db>,
    connections: Arc<SshConnectionStore>,
    managers: Mutex<HashMap<String, HostEntry>>,
    /// Connections the user disconnected by hand (E12-05 AC13, E12-06 AC5).
    /// Nothing reconnects them implicitly — not a terminal open, not boot
    /// rehydration, not the watchdog — until an explicit `connect`.
    manual_disconnects: Mutex<HashSet<String>>,
    states: Mutex<HashMap<String, ConnectionState>>,
    /// Connections whose host refused a new channel (MaxSessions, AC7).
    degraded: Mutex<HashSet<String>>,
    generations: Mutex<HashMap<String, u64>>,
    events: Arc<dyn EventBus>,
    runtime: tokio::runtime::Handle,
    /// Handle back to the `Arc` the app holds, for spawned watchdogs. Weak,
    /// so a watchdog cannot keep the registry (and its sessions) alive past
    /// app teardown.
    self_ref: Weak<Self>,
}

impl RemotePtyRegistry {
    pub fn new(
        db: Arc<dyn Db>,
        connections: Arc<SshConnectionStore>,
        events: Arc<dyn EventBus>,
        runtime: tokio::runtime::Handle,
    ) -> Arc<Self> {
        Arc::new_cyclic(|self_ref| Self {
            db,
            connections,
            managers: Mutex::new(HashMap::new()),
            manual_disconnects: Mutex::new(HashSet::new()),
            states: Mutex::new(HashMap::new()),
            degraded: Mutex::new(HashSet::new()),
            generations: Mutex::new(HashMap::new()),
            events,
            runtime,
            self_ref: self_ref.clone(),
        })
    }

    // ── Routing ────────────────────────────────────────────────────────

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

    /// The pooled client for a connection, dialing if needed (E12-06 AC2/AC3).
    /// Async twin of [`Self::host_for_connection`], for the Tauri commands
    /// that used to open a throwaway connection each call.
    pub async fn client_for(&self, connection_id: &str) -> Result<Arc<SshClient>, Error> {
        if let Some(entry) = self.cached(connection_id) {
            return Ok(entry.client);
        }
        self.guard_manual_disconnect(connection_id)?;
        match self.dial(connection_id).await {
            Ok(entry) => Ok(entry.client),
            Err(error) => {
                self.fail(connection_id, &error);
                Err(error)
            }
        }
    }

    fn host_for_connection(&self, connection_id: &str) -> Result<HostEntry, Error> {
        if let Some(entry) = self.cached(connection_id) {
            return Ok(entry);
        }
        self.guard_manual_disconnect(connection_id)?;

        // Connecting is async and this is a blocking call path (the terminal
        // manager runs on ordinary threads), so the handshake — and only the
        // handshake — blocks here.
        let result = tokio::task::block_in_place(|| {
            self.runtime
                .block_on(async { self.dial(connection_id).await })
        });
        result.inspect_err(|error| {
            self.fail(connection_id, error);
        })
    }

    /// A pooled entry, unless its session died since we last looked — a dead
    /// client is dropped here rather than handed to a caller that would fail
    /// on its first channel.
    fn cached(&self, connection_id: &str) -> Option<HostEntry> {
        let mut managers = self.managers.lock();
        match managers.get(connection_id) {
            Some(entry) if !entry.client.is_closed() => Some(entry.clone()),
            Some(_) => {
                managers.remove(connection_id);
                None
            }
            None => None,
        }
    }

    fn guard_manual_disconnect(&self, connection_id: &str) -> Result<(), Error> {
        if self.manual_disconnects.lock().contains(connection_id) {
            return Err(Error::Internal(format!(
                "ssh connection disconnected: {connection_id} — reconnect to resume"
            )));
        }
        Ok(())
    }

    /// One handshake: resolve the profile, connect, install the entry, start
    /// its watchdog. State goes `connecting` → `connected`; failure states are
    /// the caller's call (a mid-ladder failure is not `error` yet).
    async fn dial(&self, connection_id: &str) -> Result<HostEntry, Error> {
        let profile = self
            .connections
            .get(connection_id)?
            .ok_or_else(|| Error::SshConnectionNotFound(connection_id.to_string()))?;

        self.set_state(connection_id, ConnectionState::Connecting, None, None, None);
        let client = Arc::new(connect_profile(&profile).await?);

        let generation = {
            let mut generations = self.generations.lock();
            let next = generations.get(connection_id).copied().unwrap_or(0) + 1;
            generations.insert(connection_id.to_string(), next);
            next
        };
        let entry = HostEntry {
            pty: Arc::new(SshPtyManager::with_runtime(
                client.clone(),
                self.runtime.clone(),
            )),
            tmux: Arc::new(RemoteTmux::new(client.clone(), self.runtime.clone())),
            client,
            generation,
        };
        self.managers
            .lock()
            .insert(connection_id.to_string(), entry.clone());
        // A fresh session can open channels again — whatever MaxSessions
        // pressure the old one hit does not describe this one.
        self.clear_degraded(connection_id);
        self.set_state(connection_id, ConnectionState::Connected, None, None, None);
        self.spawn_watchdog(connection_id.to_string(), generation);
        Ok(entry)
    }

    // ── Lifecycle ───────────────────────────────────────────────────

    /// User-initiated disconnect (E12-05 AC13): drop the connection AND
    /// remember the intent, so no background path silently dials back.
    pub fn disconnect(&self, connection_id: &str) {
        self.manual_disconnects
            .lock()
            .insert(connection_id.to_string());
        self.bump_generation(connection_id);
        self.managers.lock().remove(connection_id);
        self.clear_degraded(connection_id);
        self.set_state(
            connection_id,
            ConnectionState::Disconnected,
            None,
            None,
            None,
        );
    }

    /// User-initiated (re)connect: clears the manual-disconnect intent and
    /// dials, so the caller learns immediately whether the host answers.
    pub fn connect(&self, connection_id: &str) -> Result<(), Error> {
        self.manual_disconnects.lock().remove(connection_id);
        self.host_for_connection(connection_id).map(|_| ())
    }

    /// Drops the cached manager for a connection (profile edited, host gone).
    /// The next open reconnects. Unlike `disconnect`, no intent is recorded.
    pub fn forget(&self, connection_id: &str) {
        self.bump_generation(connection_id);
        self.managers.lock().remove(connection_id);
        self.set_state(
            connection_id,
            ConnectionState::Disconnected,
            None,
            None,
            None,
        );
    }

    /// Whether a connection is currently held open by this process.
    pub fn is_connected(&self, connection_id: &str) -> bool {
        self.cached(connection_id).is_some()
    }

    /// Lifecycle state of one connection (E12-06 AC1). Never-dialed and
    /// forgotten connections read `Disconnected`.
    pub fn state(&self, connection_id: &str) -> ConnectionState {
        self.states
            .lock()
            .get(connection_id)
            .copied()
            .unwrap_or(ConnectionState::Disconnected)
    }

    /// Lifecycle state of every connection this process has touched.
    pub fn states(&self) -> HashMap<String, ConnectionState> {
        self.states.lock().clone()
    }

    /// Whether the host is refusing new channels (MaxSessions, AC7).
    pub fn is_degraded(&self, connection_id: &str) -> bool {
        self.degraded.lock().contains(connection_id)
    }

    /// Report a channel failure seen by a caller (terminal spawn, exec).
    /// Only channel-open refusals count: the session is live, so reconnecting
    /// would fix nothing — the host's `MaxSessions` is the fix.
    pub fn report_channel_error(&self, connection_id: &str, error: &Error) {
        if !is_channel_open_failure(error) {
            return;
        }
        let newly = self.degraded.lock().insert(connection_id.to_string());
        if newly {
            self.events.send(InternalEvent::SshConnectionHealthChanged {
                connection_id: connection_id.to_string(),
                degraded: true,
            });
        }
    }

    /// A channel opened — clears any MaxSessions warning.
    pub fn report_channel_ok(&self, connection_id: &str) {
        self.clear_degraded(connection_id);
    }

    fn clear_degraded(&self, connection_id: &str) {
        let was = self.degraded.lock().remove(connection_id);
        if was {
            self.events.send(InternalEvent::SshConnectionHealthChanged {
                connection_id: connection_id.to_string(),
                degraded: false,
            });
        }
    }

    fn bump_generation(&self, connection_id: &str) {
        let mut generations = self.generations.lock();
        let next = generations.get(connection_id).copied().unwrap_or(0) + 1;
        generations.insert(connection_id.to_string(), next);
    }

    fn fail(&self, connection_id: &str, error: &Error) {
        self.set_state(
            connection_id,
            ConnectionState::Error,
            None,
            None,
            Some(error.to_string()),
        );
    }

    fn set_state(
        &self,
        connection_id: &str,
        state: ConnectionState,
        attempt: Option<u32>,
        delay_ms: Option<u64>,
        error: Option<String>,
    ) {
        self.states.lock().insert(connection_id.to_string(), state);
        self.events.send(InternalEvent::SshConnectionStateChanged {
            connection_id: connection_id.to_string(),
            state: state.as_str().to_string(),
            attempt,
            delay_ms,
            error,
        });
    }

    /// Watches one dialed session. Exits when the entry is gone or replaced
    /// (another dial won), and hands a dead session to the reconnect ladder.
    fn spawn_watchdog(&self, connection_id: String, generation: u64) {
        let weak = self.self_ref.clone();
        self.runtime.spawn(async move {
            loop {
                tokio::time::sleep(WATCHDOG_POLL).await;
                let Some(registry) = weak.upgrade() else {
                    return;
                };
                let entry = registry.managers.lock().get(&connection_id).cloned();
                match entry {
                    Some(entry) if entry.generation != generation => return,
                    Some(entry) if !entry.client.is_closed() => continue,
                    Some(_) => {}
                    None => return,
                }
                registry.managers.lock().remove(&connection_id);
                if registry.manual_disconnects.lock().contains(&connection_id) {
                    return;
                }
                tracing::warn!(connection_id, "ssh session dropped — reconnecting");
                registry.reconnect(connection_id).await;
                return;
            }
        });
    }

    /// The backoff ladder (AC4): wait, dial, repeat; a manual disconnect at
    /// any rung wins, and exhausting the rungs parks the connection in
    /// `Error` rather than dialing forever.
    async fn reconnect(self: Arc<Self>, connection_id: String) {
        let mut last: Option<Error> = None;
        for (index, delay_ms) in RECONNECT_DELAYS_MS.iter().enumerate() {
            self.set_state(
                &connection_id,
                ConnectionState::Reconnecting,
                Some(index as u32 + 1),
                Some(*delay_ms),
                None,
            );
            tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            if self.manual_disconnects.lock().contains(&connection_id) {
                return;
            }
            match self.dial(&connection_id).await {
                Ok(_) => return,
                Err(error) => {
                    tracing::warn!(
                        connection_id,
                        attempt = index + 1,
                        error = %error,
                        "ssh reconnect attempt failed"
                    );
                    last = Some(error);
                }
            }
        }
        let error = last
            .map(|e| e.to_string())
            .unwrap_or_else(|| "reconnect failed".to_string());
        self.set_state(
            &connection_id,
            ConnectionState::Error,
            None,
            None,
            Some(error),
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::db::SqliteDb;
    use fartcode_core::events::BroadcastEventBus;

    fn registry() -> (Arc<RemotePtyRegistry>, Arc<BroadcastEventBus>) {
        let db = SqliteDb::init_in_memory().unwrap();
        let events = Arc::new(BroadcastEventBus::new(64));
        let registry = RemotePtyRegistry::new(
            db.clone(),
            Arc::new(SshConnectionStore::new(db)),
            events.clone(),
            tokio::runtime::Handle::current(),
        );
        (registry, events)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn untouched_connection_is_disconnected() {
        let (registry, _events) = registry();
        assert_eq!(registry.state("nope"), ConnectionState::Disconnected);
        assert!(!registry.is_connected("nope"));
        assert!(registry.states().is_empty());
    }

    /// AC5: the intent survives, and no dial is attempted while it holds —
    /// the error names the connection, not a network failure.
    #[tokio::test(flavor = "multi_thread")]
    async fn manual_disconnect_blocks_implicit_dial() {
        let (registry, _events) = registry();
        registry.disconnect("conn-1");
        assert_eq!(registry.state("conn-1"), ConnectionState::Disconnected);

        let error = registry.client_for("conn-1").await.unwrap_err();
        assert!(
            error.to_string().contains("disconnected"),
            "expected a disconnect-intent error, got: {error}"
        );
        // Still disconnected: a blocked dial is not a failed dial.
        assert_eq!(registry.state("conn-1"), ConnectionState::Disconnected);
    }

    /// A dial for a profile that does not exist ends in `Error`, not a panic
    /// or a silent `Disconnected`.
    #[tokio::test(flavor = "multi_thread")]
    async fn missing_profile_fails_the_connection() {
        let (registry, _events) = registry();
        assert!(registry.client_for("ghost").await.is_err());
        assert_eq!(registry.state("ghost"), ConnectionState::Error);
    }

    /// AC7: only channel-open refusals degrade health, and a later success
    /// clears it. Both edges publish.
    #[tokio::test(flavor = "multi_thread")]
    async fn channel_open_failure_degrades_health() {
        let (registry, events) = registry();
        let mut rx = events.subscribe();

        registry.report_channel_error("conn-1", &Error::Internal("timed out".into()));
        assert!(!registry.is_degraded("conn-1"));

        registry.report_channel_error(
            "conn-1",
            &Error::SshChannel("open session: channel open failure: no more sessions".into()),
        );
        assert!(registry.is_degraded("conn-1"));

        match rx.try_recv().unwrap() {
            InternalEvent::SshConnectionHealthChanged {
                connection_id,
                degraded,
            } => {
                assert_eq!(connection_id, "conn-1");
                assert!(degraded);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        registry.report_channel_ok("conn-1");
        assert!(!registry.is_degraded("conn-1"));
        assert!(matches!(
            rx.try_recv().unwrap(),
            InternalEvent::SshConnectionHealthChanged {
                degraded: false,
                ..
            }
        ));
    }

    /// AC4: five rungs, 1/2/5/10/20 s — the ladder is the contract, so it is
    /// asserted rather than left to a comment.
    #[test]
    fn backoff_ladder_matches_the_spec() {
        assert_eq!(RECONNECT_DELAYS_MS, [1_000, 2_000, 5_000, 10_000, 20_000]);
    }

    /// State changes publish for the UI (AC8).
    #[tokio::test(flavor = "multi_thread")]
    async fn state_changes_publish() {
        let (registry, events) = registry();
        let mut rx = events.subscribe();
        registry.disconnect("conn-1");
        match rx.try_recv().unwrap() {
            InternalEvent::SshConnectionStateChanged {
                connection_id,
                state,
                ..
            } => {
                assert_eq!(connection_id, "conn-1");
                assert_eq!(state, "disconnected");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
