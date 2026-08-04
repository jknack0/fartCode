//! Interactive task terminals (E2-12): PTY-backed shells in the task view.
//!
//! Spawns `$SHELL` (fallback `/bin/sh`) in the task's workspace with the
//! **inherited** env (`EnvPolicy::Inherit` — interactive shells get the
//! user's env, unlike agent launches which are allowlisted-only), pumps
//! output to the frontend as `terminal:output` events (base64 chunks), and
//! reports `terminal:exited` when the shell ends.
//!
//! **Tmux durability (ADR-0025):** when the project's `tmux` setting is on
//! AND the tmux binary resolves, the spawned PTY runs the E2-07
//! create-or-attach shell line around a deterministic per-task slot session
//! (`{project}:{task}:terminal:{slot}`). The tmux SERVER owns the shell, so
//! an app crash/restart leaves it alive: the next `open` for the task
//! reattaches (slot 0 = the default terminal). Closing a tab kills only the
//! attach client (detach — the session survives); deleting the task sweeps
//! every `…:terminal:` session of the task, orphans included.

use std::collections::{HashMap, HashSet};

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ade_core::terminals::pty::{EnvPolicy, PtyHandle, PtyManager, PtySize};
use ade_terminal::PortablePtyManager;
use parking_lot::Mutex;
use serde::Serialize;

const PUMP_INTERVAL: Duration = Duration::from_millis(16);
const PUMP_CHUNK_CAP: usize = 64 * 1024;

/// Terminal output chunk (emitted as `terminal:output`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutput {
    /// Terminal id.
    pub terminal_id: String,
    /// Base64-encoded PTY bytes.
    pub data: String,
}

/// Terminal exit (emitted as `terminal:exited`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExited {
    /// Terminal id.
    pub terminal_id: String,
    /// Exit code when the OS provides one.
    pub exit_code: Option<u32>,
}

struct Entry {
    task_id: String,
    handle: Mutex<Box<dyn PtyHandle>>,
}

/// Owns all live interactive terminals.
pub struct TerminalManager {
    pty: PortablePtyManager,
    app: tauri::AppHandle,
    terminals: Arc<Mutex<HashMap<String, Arc<Entry>>>>,
    /// Tmux slots allocated per task BY THIS PROCESS (ADR-0025). A closed
    /// tab keeps its slot allocated (detach semantics) so the next open gets
    /// a NEW session; a restart starts empty, so the task's first open
    /// reattaches slot 0 — the surviving default terminal.
    task_slots: Mutex<HashMap<String, HashSet<u32>>>,
}

impl TerminalManager {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            pty: PortablePtyManager,
            app,
            terminals: Arc::new(Mutex::new(HashMap::new())),
            task_slots: Mutex::new(HashMap::new()),
        }
    }

    /// Spawns a shell in `cwd` and starts the output pump. Returns the
    /// terminal id.
    ///
    /// With `tmux` on AND a resolvable tmux binary (ADR-0025) the PTY runs
    /// the create-or-attach shell line for slot `{project}:{task}:terminal:
    /// {slot}` instead of a bare shell — the tmux server owns the session,
    /// so the shell survives an app crash; the next `open` reattaches.
    pub fn open(
        &self,
        task_id: &str,
        project_id: &str,
        tmux: bool,
        cwd: &Path,
        rows: u16,
        cols: u16,
    ) -> Result<String, ade_core::Error> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let tmux_binary = tmux
            .then(ade_core::pty::tmux::resolve_tmux_binary)
            .flatten();
        let (spawn_cmd, spawn_args, spawn_env): (String, Vec<String>, Vec<(String, String)>) =
            match tmux_binary {
                Some(binary) => {
                    let slot = self.allocate_slot(task_id);
                    let session_id = format!("{project_id}:{task_id}:terminal:{slot}");
                    let name = ade_core::pty::tmux::make_tmux_session_name(&session_id);
                    let inner = ade_core::pty::tmux::build_terminal_session_command(cwd, &shell);
                    let line = ade_core::pty::tmux::build_tmux_shell_line(&name, &inner);
                    // tmux needs TERM; portable-pty sets none and Dock-launched
                    // apps may inherit no TERM either. PATH overlay covers the
                    // Dock PATH that lacks Homebrew.
                    let mut env = vec![("TERM".into(), "xterm-256color".into())];
                    if let Some(path) = &binary.path_overlay {
                        env.push(("PATH".into(), path.clone()));
                    }
                    ("/bin/sh".into(), vec!["-c".into(), line], env)
                }
                None => {
                    if tmux {
                        tracing::debug!(task_id, "tmux enabled but no binary — plain shell");
                    }
                    (shell, Vec::new(), Vec::new())
                }
            };
        let handle = self.pty.spawn(
            &spawn_cmd,
            &spawn_args,
            cwd,
            &spawn_env,
            PtySize {
                rows: rows.max(1),
                cols: cols.max(2),
            },
            EnvPolicy::Inherit,
        )?;
        let id = uuid::Uuid::new_v4().to_string();
        let entry = Arc::new(Entry {
            task_id: task_id.to_string(),
            handle: Mutex::new(handle),
        });
        self.terminals.lock().insert(id.clone(), Arc::clone(&entry));

        // Output pump: drain the PTY reader ~60x/s; watch for exit via
        // try_wait_exit so the tail of output is delivered before exited.
        let pump_id = id.clone();
        let app = self.app.clone();
        let terminals = self.terminals.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            loop {
                let gained = {
                    let mut handle = entry.handle.lock();
                    match handle.try_read(&mut buf) {
                        Ok(gained) => gained,
                        Err(_) => break,
                    }
                };
                if gained && !buf.is_empty() {
                    let chunk: Vec<u8> = if buf.len() > PUMP_CHUNK_CAP {
                        buf.drain(..PUMP_CHUNK_CAP).collect()
                    } else {
                        std::mem::take(&mut buf)
                    };
                    use base64::Engine as _;
                    let data = base64::engine::general_purpose::STANDARD.encode(&chunk);
                    use tauri::Emitter as _;
                    let _ = app.emit(
                        "terminal:output",
                        TerminalOutput {
                            terminal_id: pump_id.clone(),
                            data,
                        },
                    );
                    continue; // drain before polling exit
                }
                let exited = {
                    let mut handle = entry.handle.lock();
                    handle.try_wait_exit().ok().flatten()
                };
                if let Some(exit) = exited {
                    use tauri::Emitter as _;
                    let _ = app.emit(
                        "terminal:exited",
                        TerminalExited {
                            terminal_id: pump_id.clone(),
                            exit_code: exit.exit_code,
                        },
                    );
                    terminals.lock().remove(&pump_id);
                    break;
                }
                std::thread::sleep(PUMP_INTERVAL);
            }
        });

        Ok(id)
    }

    /// Smallest unused tmux slot for the task IN THIS PROCESS (ADR-0025).
    /// Slots stay allocated after tab close (detach semantics — a new open
    /// gets a fresh session, never accidentally reattaches a detached one);
    /// a restarted process starts empty, so its first open for the task
    /// claims slot 0 and reattaches the surviving default terminal.
    fn allocate_slot(&self, task_id: &str) -> u32 {
        let mut slots = self.task_slots.lock();
        let used = slots.entry(task_id.to_string()).or_default();
        let slot = (0..).find(|n| !used.contains(n)).expect("u32 slot space");
        used.insert(slot);
        slot
    }

    /// Types into the terminal.
    pub fn write(&self, id: &str, data: &str) -> Result<(), ade_core::Error> {
        let entry = self
            .terminals
            .lock()
            .get(id)
            .cloned()
            .ok_or_else(|| ade_core::Error::Internal(format!("terminal not found: {id}")))?;
        let result = entry.handle.lock().write(data);
        result
    }

    /// Resizes the terminal (clamped by the PTY layer).
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), ade_core::Error> {
        let entry = self
            .terminals
            .lock()
            .get(id)
            .cloned()
            .ok_or_else(|| ade_core::Error::Internal(format!("terminal not found: {id}")))?;
        let result = entry.handle.lock().resize(cols, rows);
        result
    }

    /// Kills the shell and drops the entry. Idempotent.
    pub fn close(&self, id: &str) {
        let entry = self.terminals.lock().remove(id);
        if let Some(entry) = entry {
            let _ = entry.handle.lock().kill();
        }
    }

    /// Closes every terminal belonging to a task (task deletion teardown).
    /// Also sweeps the task's tmux sessions — including ones orphaned by a
    /// crashed app instance (ADR-0025). Best-effort: absent tmux → no-op.
    pub fn close_task(&self, project_id: &str, task_id: &str) {
        let ids: Vec<String> = self
            .terminals
            .lock()
            .iter()
            .filter(|(_, e)| e.task_id == task_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.close(&id);
        }
        let killed = ade_core::pty::tmux::kill_tmux_sessions_by_prefix(&format!(
            "{project_id}:{task_id}:terminal:"
        ));
        if killed > 0 {
            tracing::info!(
                task_id,
                killed,
                "tmux terminal sessions swept on task deletion"
            );
        }
        self.task_slots.lock().remove(task_id);
    }
}
