//! PTY session registry (E2-09, reference `pty-session-registry.ts`): tracks
//! running agent sessions by their pty session id (`make_pty_session_id`) so
//! task deletion can reap them. Phase 0 subset: cancel + exit accounting —
//! the attach/resize/ring-buffer surface arrives with the interactive
//! terminal UI.
//!
//! The launcher OWNS each `PtyHandle` on its own thread; the registry never
//! touches handles. Instead it hands the launcher a cancel flag to poll and
//! an exited flag to set once the child is reaped — cooperative teardown
//! that is safe against a delete racing a running agent.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::Error;

/// Flags shared between the registry and one running launch: the launcher
/// polls `cancel` (and kills the child when set) and stores `exited` after
/// the final exit is observed.
#[derive(Debug, Clone)]
pub struct SessionFlags {
    pub cancel: Arc<AtomicBool>,
    pub exited: Arc<AtomicBool>,
}

struct SessionEntry {
    flags: SessionFlags,
    #[allow(dead_code)] // surfaced with the terminal UI's session list
    provider: Option<String>,
}

/// Cooperative teardown registry for running PTY sessions.
#[derive(Default)]
pub struct SessionRegistry {
    inner: Mutex<HashMap<String, SessionEntry>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<String, SessionEntry>>, Error> {
        self.inner
            .lock()
            .map_err(|_| Error::Internal("session registry mutex poisoned".into()))
    }

    /// Registers a running session (a stale entry for the same id is
    /// replaced — re-launches supersede). Returns the flags the launcher
    /// must poll/set. Registration survives a poisoned mutex (the launch
    /// then simply runs untracked).
    pub fn register(&self, session_id: &str, provider: Option<String>) -> SessionFlags {
        let flags = SessionFlags {
            cancel: Arc::new(AtomicBool::new(false)),
            exited: Arc::new(AtomicBool::new(false)),
        };
        if let Ok(mut map) = self.lock() {
            map.insert(
                session_id.to_string(),
                SessionEntry {
                    flags: flags.clone(),
                    provider,
                },
            );
        }
        flags
    }

    /// Removes a session (idempotent — the entry may already be gone).
    pub fn unregister(&self, session_id: &str) {
        if let Ok(mut map) = self.lock() {
            map.remove(session_id);
        }
    }

    pub fn is_active(&self, session_id: &str) -> bool {
        self.lock()
            .map(|m| m.contains_key(session_id))
            .unwrap_or(false)
    }

    /// Sets the cancel flag, waits (bounded) for the launcher to reap the
    /// child, then removes the entry. Returns `true` when a live session was
    /// cancelled (whether or not the exit landed inside the window), `false`
    /// when no session was registered. The launcher's exit path unregisters
    /// too — that is an idempotent no-op after this removal.
    pub fn terminate(&self, session_id: &str, timeout: Duration) -> bool {
        let flags = match self.lock() {
            Ok(map) => map.get(session_id).map(|e| e.flags.clone()),
            Err(_) => return false,
        };
        let Some(flags) = flags else {
            return false;
        };
        flags.cancel.store(true, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + timeout;
        while !flags.exited.load(Ordering::SeqCst) {
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.unregister(session_id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_unregister_round_trip() {
        let reg = SessionRegistry::new();
        let _flags = reg.register("p:t:c", Some("claude".into()));
        assert!(reg.is_active("p:t:c"));
        reg.unregister("p:t:c");
        assert!(!reg.is_active("p:t:c"));
        // Idempotent.
        reg.unregister("p:t:c");
    }

    #[test]
    fn terminate_sets_cancel_and_waits_for_exit() {
        let reg = Arc::new(SessionRegistry::new());
        let flags = reg.register("p:t:c", None);

        let waiter = {
            let reg = reg.clone();
            std::thread::spawn(move || reg.terminate("p:t:c", Duration::from_secs(2)))
        };
        // Simulate the launcher observing the cancel and reaping.
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !flags.cancel.load(Ordering::SeqCst) {
            assert!(std::time::Instant::now() < deadline, "cancel never set");
            std::thread::sleep(Duration::from_millis(5));
        }
        flags.exited.store(true, Ordering::SeqCst);
        assert!(waiter.join().unwrap(), "live session reports cancelled");
        assert!(!reg.is_active("p:t:c"), "terminate removes the entry");
    }

    #[test]
    fn terminate_unknown_session_is_false() {
        let reg = SessionRegistry::new();
        assert!(!reg.terminate("missing", Duration::from_millis(50)));
    }
}
