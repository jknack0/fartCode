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
    /// Provider id when this terminal runs an agent CLI (`terminal_open_agent`);
    /// `None` for plain shells — the diff selection prompt routes to agents.
    agent: Option<String>,
    /// Decoded tmux session id (`{project}:{task}:terminal:{slot}`) when
    /// the PTY runs a durable session (ADR-0025); `None` for plain shells.
    tmux_session_id: Option<String>,
    handle: Mutex<Box<dyn PtyHandle>>,
}

/// Everything needed to open one interactive terminal (ADR-0025 merged with
/// the agent-terminal spawn: program + args for plain PTYs, project id +
/// tmux flag for slot durability).
pub struct TerminalSpec<'a> {
    pub task_id: &'a str,
    pub project_id: &'a str,
    /// Provider id for agent terminals (`terminal_open_agent`); shells pass
    /// `None`.
    pub agent: Option<&'a str>,
    /// Run under tmux slot durability when the binary resolves.
    pub tmux: bool,
    pub program: &'a str,
    pub args: &'a [String],
    pub cwd: &'a Path,
    pub rows: u16,
    pub cols: u16,
}

/// Owns all live interactive terminals.
pub struct TerminalManager {
    pty: PortablePtyManager,
    app: tauri::AppHandle,
    terminals: Arc<Mutex<HashMap<String, Arc<Entry>>>>,
    /// Tmux slots allocated per task BY THIS PROCESS (ADR-0025). A restart
    /// starts empty; the first open then prefers REUSING a live detached
    /// session over creating a fresh slot (ADR-0028).
    task_slots: Mutex<HashMap<String, HashSet<u32>>>,
}

/// One live terminal for `list_for_task`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    /// Provider id for agent terminals; `None` for shells.
    pub agent: Option<String>,
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

    /// Spawns `spec.program` (with `spec.args`) in `spec.cwd` and starts
    /// the output pump. Returns the terminal id.
    ///
    /// With `spec.tmux` on AND a resolvable tmux binary (ADR-0025) the PTY
    /// runs the create-or-attach shell line for slot `{project}:{task}:
    /// terminal:{slot}` with `spec.program` as the session's foreground
    /// command (`spec.args` are not passed into a tmux session) — the tmux
    /// server owns it, so it survives an app crash; the next `open`
    /// reattaches.
    pub fn open(&self, spec: TerminalSpec<'_>) -> Result<String, ade_core::Error> {
        let TerminalSpec {
            task_id,
            project_id,
            agent,
            tmux,
            program,
            args,
            cwd,
            rows,
            cols,
        } = spec;
        let tmux_binary = tmux
            .then(ade_core::pty::tmux::resolve_tmux_binary)
            .flatten();
        let mut tmux_session_id: Option<String> = None;
        let (spawn_cmd, spawn_args, spawn_env): (String, Vec<String>, Vec<(String, String)>) =
            match tmux_binary {
                Some(binary) => {
                    let slot = self.pick_slot(project_id, task_id);
                    let session_id = format!("{project_id}:{task_id}:terminal:{slot}");
                    let name = ade_core::pty::tmux::make_tmux_session_name(&session_id);
                    let inner = ade_core::pty::tmux::build_terminal_session_command(cwd, program);
                    let line = ade_core::pty::tmux::build_tmux_shell_line(&name, &inner);
                    // tmux needs TERM; portable-pty sets none and Dock-launched
                    // apps may inherit no TERM either. PATH overlay covers the
                    // Dock PATH that lacks Homebrew.
                    let mut env = vec![("TERM".into(), "xterm-256color".into())];
                    if let Some(path) = &binary.path_overlay {
                        env.push(("PATH".into(), path.clone()));
                    }
                    tmux_session_id = Some(session_id);
                    ("/bin/sh".into(), vec!["-c".into(), line], env)
                }
                None => {
                    if tmux {
                        tracing::debug!(task_id, "tmux enabled but no binary — plain spawn");
                    }
                    (program.to_string(), args.to_vec(), Vec::new())
                }
            };
        let spawn_result = self.pty.spawn(
            &spawn_cmd,
            &spawn_args,
            cwd,
            &spawn_env,
            PtySize {
                rows: rows.max(1),
                cols: cols.max(2),
            },
            EnvPolicy::Inherit,
        );
        let handle = match spawn_result {
            Ok(handle) => handle,
            Err(e) => {
                // A failed spawn must not keep the slot reserved (ADR-0028):
                // the session was neither created nor attached.
                if let Some(session_id) = &tmux_session_id {
                    if let Some(rest) = session_id.rsplit_once(':') {
                        if let Ok(slot) = rest.1.parse::<u32>() {
                            self.task_slots
                                .lock()
                                .get_mut(task_id)
                                .map(|used| used.remove(&slot));
                        }
                    }
                }
                return Err(e);
            }
        };
        let id = uuid::Uuid::new_v4().to_string();
        let entry = Arc::new(Entry {
            task_id: task_id.to_string(),
            agent: agent.map(str::to_string),
            tmux_session_id,
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

    /// Slot for the task's next open (ADR-0028): reuse the smallest live
    /// DETACHED session this process does not already own; else the first
    /// free slot. See `choose_terminal_slot` for the policy.
    fn pick_slot(&self, project_id: &str, task_id: &str) -> u32 {
        let prefix = format!("{project_id}:{task_id}:terminal:");
        let live = ade_core::pty::tmux::list_tmux_sessions_by_prefix(&prefix);
        let owned = self
            .task_slots
            .lock()
            .get(task_id)
            .cloned()
            .unwrap_or_default();
        let slot = ade_core::pty::tmux::choose_terminal_slot(&prefix, &owned, &live);
        self.task_slots
            .lock()
            .entry(task_id.to_string())
            .or_default()
            .insert(slot);
        slot
    }

    /// Live tmux sessions of the task this process does NOT currently show
    /// (what a fresh restore would surface — ADR-0028). 0 when tmux is
    /// absent or no server runs.
    pub fn surviving_session_count(&self, project_id: &str, task_id: &str) -> usize {
        let prefix = format!("{project_id}:{task_id}:terminal:");
        let owned: HashSet<String> = {
            let terminals = self.terminals.lock();
            terminals
                .values()
                .filter(|e| e.task_id == task_id)
                .filter_map(|e| e.tmux_session_id.clone())
                .collect()
        };
        ade_core::pty::tmux::list_tmux_sessions_by_prefix(&prefix)
            .iter()
            .filter(|s| !owned.contains(&s.session_id))
            .count()
    }

    /// Live terminals of a task with their agent tags (the diff selection
    /// prompt routes to the agent terminal when one exists).
    pub fn list_for_task(&self, task_id: &str) -> Vec<TerminalInfo> {
        self.terminals
            .lock()
            .iter()
            .filter(|(_, e)| e.task_id == task_id)
            .map(|(id, e)| TerminalInfo {
                id: id.clone(),
                agent: e.agent.clone(),
            })
            .collect()
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

    /// Kills the shell and drops the entry. Idempotent. With tmux
    /// durability, closing ALSO kills the session (ADR-0028): a closed tab
    /// is an intentional teardown, not a detach — the session must not
    /// linger detached for a later open to trip over.
    pub fn close(&self, id: &str) {
        let entry = self.terminals.lock().remove(id);
        if let Some(entry) = entry {
            let _ = entry.handle.lock().kill();
            if let Some(session_id) = &entry.tmux_session_id {
                let name = ade_core::pty::tmux::make_tmux_session_name(session_id);
                if let Err(e) = ade_core::pty::tmux::kill_tmux_session(&name) {
                    tracing::warn!(session = %name, error = %e, "tmux kill-session on close failed");
                }
                // The session is gone — free its slot so the next open can
                // recreate a low-numbered session instead of climbing forever.
                if let Some((_, slot_str)) = session_id.rsplit_once(':') {
                    if let Ok(slot) = slot_str.parse::<u32>() {
                        self.task_slots
                            .lock()
                            .get_mut(&entry.task_id)
                            .map(|used| used.remove(&slot));
                    }
                }
            }
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

    /// Detach every live terminal (ADR-0028 window-close semantics): kill
    /// the ATTACH clients (PTYs) but keep tmux sessions alive — reopening
    /// the UI reattaches the same shells. Plain-PTY shells die with their
    /// PTY (they have nothing to survive into; reference parity). Entries
    /// and slots are dropped so a reload starts clean.
    pub fn detach_all(&self) {
        let entries: Vec<Arc<Entry>> = self.terminals.lock().drain().map(|(_, e)| e).collect();
        for entry in entries {
            let _ = entry.handle.lock().kill();
        }
        self.task_slots.lock().clear();
    }
}
