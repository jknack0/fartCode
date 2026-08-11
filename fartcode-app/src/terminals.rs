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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fartcode_core::terminals::lifecycle::LifecycleScriptType;
use fartcode_core::terminals::pty::{EnvPolicy, PtyHandle, PtyManager, PtySize};
use fartcode_terminal::PortablePtyManager;
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
    /// Script type when this terminal runs a lifecycle script (E1-06);
    /// `None` for plain shells and agent terminals. Lifecycle entries are
    /// RETAINED after the script exits (the tab still shows the output tail
    /// and a rerun mints a fresh terminal), whereas plain terminals drop
    /// their entry when the shell exits.
    lifecycle_type: Option<String>,
    /// Set when the PTY process has exited (pump). Plain entries are then
    /// removed from the map; retained lifecycle entries stay listable.
    exited: AtomicBool,
    /// Exit code recorded by the pump when the process ends (`None` while
    /// running, and for exits the OS reports no code for). Only retained
    /// lifecycle entries outlive the exit, so this is what
    /// `terminal_list_for_task` surfaces for finished script runs (7b).
    exit_code: Mutex<Option<u32>>,
    /// Spawn time — the 7b trailing exit line reports elapsed wall time.
    started: Instant,
    handle: Mutex<Box<dyn PtyHandle>>,
    /// Rolling output tail (replayed on frontend reattach after a webview
    /// reload — plain PTYs have no scrollback server, and a tmux session
    /// only redraws on a fresh attach).
    tail: Mutex<Vec<u8>>,
}

/// Cap for the scrollback tail replayed on reattach.
const TAIL_CAP: usize = 64 * 1024;

/// Appends `chunk` to the rolling scrollback tail, enforcing `TAIL_CAP`
/// (oldest bytes drained first — the replay contract is byte-oriented).
fn push_tail(tail: &mut Vec<u8>, chunk: &[u8]) {
    tail.extend_from_slice(chunk);
    if tail.len() > TAIL_CAP {
        let excess = tail.len() - TAIL_CAP;
        tail.drain(..excess);
    }
}

/// Compact mono elapsed for the 7b exit line: "830ms", "4.2s", "1m04s".
fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs_f64();
    if secs < 1.0 {
        format!("{}ms", elapsed.as_millis())
    } else if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let total = elapsed.as_secs();
        format!("{}m{:02}s", total / 60, total % 60)
    }
}

/// 7b log body trailing line: `<scriptType> exited <code> · <elapsed>` —
/// ANSI red (31) for a non-zero (or unreported) code, dim (2) for 0.
/// `\r\n`-framed so it lands on its own line no matter how the script's
/// last output ended.
fn lifecycle_exit_line(script_type: &str, exit_code: Option<u32>, elapsed: Duration) -> Vec<u8> {
    let style = if exit_code == Some(0) {
        "\x1b[2m"
    } else {
        "\x1b[31m"
    };
    let code = exit_code.map_or_else(|| "?".to_string(), |c| c.to_string());
    format!(
        "\r\n{style}{script_type} exited {code} \u{b7} {}\x1b[0m\r\n",
        format_elapsed(elapsed)
    )
    .into_bytes()
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
    /// Extra environment for the spawned process (merged over the
    /// inherited env). The E1-06 lifecycle env contract (`FARTCODE_*` vars)
    /// arrives through here.
    pub env: &'a [(String, String)],
    /// Env vars stripped from the child AFTER env application (ADR-0034):
    /// a claude-login (OAuth) default account must not see
    /// `ANTHROPIC_API_KEY` — its presence flips the CLI to API billing.
    pub remove: &'a [String],
    pub cwd: &'a Path,
    pub rows: u16,
    pub cols: u16,
    /// Script type when this terminal runs a lifecycle script (E1-06):
    /// dedupes in-flight runs and retains the entry after exit so the
    /// finished run stays attachable with its output tail. `None` for
    /// shells and agent terminals (current behavior).
    pub lifecycle: Option<LifecycleScriptType>,
}

/// Owns all live interactive terminals. `R` defaults to the Wry runtime —
/// tests construct `TerminalManager::<tauri::test::MockRuntime>` via
/// `tauri::test::mock_app` (the emit calls are no-ops there).
pub struct TerminalManager<R: tauri::Runtime = tauri::Wry> {
    pty: PortablePtyManager,
    /// E12-05: resolves a task's remote PTY manager when its workspace lives
    /// on an SSH host. `None` in tests and until `with_remote` wires it.
    remote: Option<Arc<crate::remote_pty::RemotePtyRegistry>>,
    app: tauri::AppHandle<R>,
    terminals: Arc<Mutex<HashMap<String, Arc<Entry>>>>,
    /// Tmux slots allocated per task BY THIS PROCESS (ADR-0025). A restart
    /// starts empty; the first open then prefers REUSING a live detached
    /// session over creating a fresh slot (ADR-0028).
    task_slots: Mutex<HashMap<String, HashSet<u32>>>,
}

/// One terminal for `list_for_task` (retained lifecycle terminals whose
/// script already exited are listed too — their tabs reattach the tail).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    /// Provider id for agent terminals; `None` for shells.
    pub agent: Option<String>,
    /// `shell` | `agent` | `lifecycle` — the frontend uses this to restore
    /// tabs: lifecycle terminals surface as script tabs, the rest as plain
    /// terminal tabs.
    pub kind: String,
    /// Script type (`setup`/`run`/`teardown`) when `kind` is `lifecycle`.
    pub script_type: Option<String>,
    /// `false` once the process exited (only retained lifecycle entries are
    /// still listed then).
    pub running: bool,
    /// Exit code for finished retained entries; `None` while running or
    /// when the OS reported none.
    pub exit_code: Option<u32>,
}

impl<R: tauri::Runtime> TerminalManager<R> {
    /// `R` is generic so tests can drive a mock Tauri runtime
    /// (`tauri::test::mock_app`) without a webview.
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self {
            pty: PortablePtyManager,
            remote: None,
            app,
            terminals: Arc::new(Mutex::new(HashMap::new())),
            task_slots: Mutex::new(HashMap::new()),
        }
    }

    /// Enables remote routing (E12-05). Without it every terminal is local,
    /// which is exactly the Phase-0/2 behavior.
    pub fn with_remote(mut self, remote: Arc<crate::remote_pty::RemotePtyRegistry>) -> Self {
        self.remote = Some(remote);
        self
    }

    /// Spawns `spec.program` (with `spec.args`) in `spec.cwd` and starts
    /// the output pump. Returns the terminal id.
    ///
    /// With `spec.tmux` on AND a resolvable tmux binary (ADR-0025) the PTY
    /// runs the create-or-attach shell line for slot `{project}:{task}:
    /// terminal:{slot}` with `spec.program` as the session's foreground
    /// command; non-empty `spec.args` are appended to the session command
    /// (E2-13, #52 — startup commands spawn as `sh -c '<cmd>'`). The tmux
    /// server owns it, so it survives an app crash; the next `open`
    /// reattaches.
    ///
    /// Agent terminals are deduped per task (ADR-0033): a live agent
    /// terminal reattaches instead of spawning a second — enforced here so
    /// every caller (dispatch, ⌘⇧O, comment-tasks) converges on one
    /// session.
    pub fn open(&self, spec: TerminalSpec<'_>) -> Result<String, fartcode_core::Error> {
        let TerminalSpec {
            task_id,
            project_id,
            agent,
            tmux,
            program,
            args,
            env,
            remove,
            cwd,
            rows,
            cols,
            lifecycle,
        } = spec;
        if agent.is_some() {
            if let Some(existing) = self.find_running_agent(task_id) {
                return Ok(existing);
            }
        }
        // E12-05: the workspace row decides the transport. An error here is
        // "remote task, unreachable host" — surfaced, never downgraded to a
        // local spawn against a path that does not exist on this machine.
        let remote_route = match &self.remote {
            Some(registry) => registry.route_for_task(task_id)?,
            None => None,
        };
        let remote_pty = remote_route.as_ref().map(|(manager, _)| manager.clone());
        // tmux durability resolves a binary on THIS machine (ADR-0025), so it
        // cannot describe a remote session — remote tmux is E12-05's tmux
        // criterion, still open.
        let tmux_binary = (tmux && remote_pty.is_none())
            .then(fartcode_core::pty::tmux::resolve_tmux_binary)
            .flatten();
        if tmux && remote_pty.is_some() {
            tracing::debug!(task_id, "remote terminal — tmux durability not yet wired");
        }
        let mut tmux_session_id: Option<String> = None;
        let (spawn_cmd, spawn_args, spawn_env): (String, Vec<String>, Vec<(String, String)>) =
            match tmux_binary {
                Some(binary) => {
                    let slot = self.pick_slot(project_id, task_id);
                    let session_id = format!("{project_id}:{task_id}:terminal:{slot}");
                    let name = fartcode_core::pty::tmux::make_tmux_session_name(&session_id);
                    // E2-13 (#52): startup commands spawn as `sh -c '<cmd>'`,
                    // which the plain builder cannot carry — args go into the
                    // session command too.
                    let inner = if args.is_empty() {
                        fartcode_core::pty::tmux::build_terminal_session_command(cwd, program)
                    } else {
                        fartcode_core::pty::tmux::build_terminal_session_command_args(
                            cwd, program, args,
                        )
                    };
                    let line = fartcode_core::pty::tmux::build_tmux_shell_line(&name, &inner);
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
                    (program.to_string(), args.to_vec(), env.to_vec())
                }
            };
        let pty: &dyn PtyManager = match remote_pty.as_deref() {
            Some(manager) => manager,
            None => &self.pty,
        };
        // E12-05 AC7: the agent/script learns which remote workspace it is
        // in. Local terminals never see the variable.
        let spawn_env: Vec<(String, String)> = match &remote_route {
            Some((_, target)) => spawn_env
                .into_iter()
                .chain([(
                    "REMOTE_WORKSPACE_ID".to_string(),
                    target.workspace_id.clone(),
                )])
                .collect(),
            None => spawn_env,
        };
        let spawn_result = pty.spawn(
            &spawn_cmd,
            &spawn_args,
            cwd,
            &spawn_env,
            PtySize {
                rows: rows.max(1),
                cols: cols.max(2),
            },
            EnvPolicy::Inherit,
            remove,
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
            lifecycle_type: lifecycle.map(|t| t.as_str().to_string()),
            exited: AtomicBool::new(false),
            exit_code: Mutex::new(None),
            started: Instant::now(),
            handle: Mutex::new(handle),
            tail: Mutex::new(Vec::new()),
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
                    push_tail(&mut entry.tail.lock(), &chunk);
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
                    // Record the code BEFORE flipping `exited` so a reader
                    // that observes exited=true never sees a stale None.
                    *entry.exit_code.lock() = exit.exit_code;
                    // 7b log body: a lifecycle run ends with a trailing
                    // "<scriptType> exited <code> · <elapsed>" line (red on
                    // failure, dim on 0) — appended to the retained tail so
                    // a reattached drawer shows how the run ended, and
                    // emitted like normal output for a live-attached one.
                    if let Some(script_type) = &entry.lifecycle_type {
                        let line = lifecycle_exit_line(
                            script_type,
                            exit.exit_code,
                            entry.started.elapsed(),
                        );
                        push_tail(&mut entry.tail.lock(), &line);
                        use base64::Engine as _;
                        let data = base64::engine::general_purpose::STANDARD.encode(&line);
                        use tauri::Emitter as _;
                        let _ = app.emit(
                            "terminal:output",
                            TerminalOutput {
                                terminal_id: pump_id.clone(),
                                data,
                            },
                        );
                    }
                    entry.exited.store(true, Ordering::Relaxed);
                    use tauri::Emitter as _;
                    let _ = app.emit(
                        "terminal:exited",
                        TerminalExited {
                            terminal_id: pump_id.clone(),
                            exit_code: exit.exit_code,
                        },
                    );
                    // E17-03 auto-flip, session-scoped since E18-05: an
                    // agent terminal exiting settles its task's linked
                    // issues, identified by this terminal entry.
                    if entry.agent.is_some() {
                        crate::dispatch::flip_for_exited_agent(&app, &entry.task_id, &pump_id);
                    }
                    // Lifecycle script terminals are retained after exit so
                    // a later tab attach still finds the entry and replays
                    // the output tail (E1-06). Plain terminals drop.
                    if entry.lifecycle_type.is_none() {
                        terminals.lock().remove(&pump_id);
                    }
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
        let live = fartcode_core::pty::tmux::list_tmux_sessions_by_prefix(&prefix);
        let owned = self
            .task_slots
            .lock()
            .get(task_id)
            .cloned()
            .unwrap_or_default();
        let slot = fartcode_core::pty::tmux::choose_terminal_slot(&prefix, &owned, &live);
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
        fartcode_core::pty::tmux::list_tmux_sessions_by_prefix(&prefix)
            .iter()
            .filter(|s| !owned.contains(&s.session_id))
            .count()
    }

    /// Terminals of a task (the diff selection prompt routes to the agent
    /// terminal when one exists). Lifecycle script terminals are listed
    /// even after the script exited (their entry is retained) so a task
    /// open can surface the finished run's tab.
    pub fn list_for_task(&self, task_id: &str) -> Vec<TerminalInfo> {
        self.terminals
            .lock()
            .iter()
            .filter(|(_, e)| e.task_id == task_id)
            .map(|(id, e)| {
                let (kind, script_type) = match &e.lifecycle_type {
                    Some(t) => ("lifecycle".to_string(), Some(t.clone())),
                    None if e.agent.is_some() => ("agent".to_string(), None),
                    None => ("shell".to_string(), None),
                };
                TerminalInfo {
                    id: id.clone(),
                    agent: e.agent.clone(),
                    kind,
                    script_type,
                    running: !e.exited.load(Ordering::Relaxed),
                    exit_code: *e.exit_code.lock(),
                }
            })
            .collect()
    }

    /// Id of the task's IN-FLIGHT lifecycle terminal of `script_type`, if
    /// any (E1-06 dedupe — a rerun while one is running reattaches instead
    /// of spawning a second script).
    pub fn find_running_lifecycle(&self, task_id: &str, script_type: &str) -> Option<String> {
        self.terminals
            .lock()
            .iter()
            .find(|(_, e)| {
                e.task_id == task_id
                    && e.lifecycle_type.as_deref() == Some(script_type)
                    && !e.exited.load(Ordering::Relaxed)
            })
            .map(|(id, _)| id.clone())
    }

    /// Blocks until the terminal's process exits and returns its exit code
    /// (`Some(None)` when the OS reported none). Returns `None` when the
    /// terminal is unknown or its entry is dropped before an exit is
    /// observed (closed tab, plain terminal reaped) — 7b gates the default-
    /// agent launch on the auto-run setup script's clean exit through this.
    pub fn wait_for_exit(&self, id: &str) -> Option<Option<u32>> {
        loop {
            let entry = self.terminals.lock().get(id).cloned()?;
            if entry.exited.load(Ordering::Relaxed) {
                return Some(*entry.exit_code.lock());
            }
            std::thread::sleep(PUMP_INTERVAL);
        }
    }

    /// Id of the task's IN-FLIGHT agent terminal, if any (ADR-0033: one
    /// agent terminal per task — a second open reattaches the live one
    /// instead of stacking another agent tab).
    pub fn find_running_agent(&self, task_id: &str) -> Option<String> {
        self.terminals
            .lock()
            .iter()
            .find(|(_, e)| {
                e.task_id == task_id && e.agent.is_some() && !e.exited.load(Ordering::Relaxed)
            })
            .map(|(id, _)| id.clone())
    }

    /// Base64 scrollback tail for reattach replay (frontend reload), or
    /// `None` for an unknown terminal.
    pub fn tail(&self, id: &str) -> Option<String> {
        let entry = self.terminals.lock().get(id).cloned()?;
        let tail = entry.tail.lock();
        if tail.is_empty() {
            return None;
        }
        use base64::Engine as _;
        Some(base64::engine::general_purpose::STANDARD.encode(tail.as_slice()))
    }

    /// Types into the terminal.
    pub fn write(&self, id: &str, data: &str) -> Result<(), fartcode_core::Error> {
        let entry =
            self.terminals.lock().get(id).cloned().ok_or_else(|| {
                fartcode_core::Error::Internal(format!("terminal not found: {id}"))
            })?;
        let result = entry.handle.lock().write(data);
        result
    }

    /// Resizes the terminal (clamped by the PTY layer).
    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), fartcode_core::Error> {
        let entry =
            self.terminals.lock().get(id).cloned().ok_or_else(|| {
                fartcode_core::Error::Internal(format!("terminal not found: {id}"))
            })?;
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
                let name = fartcode_core::pty::tmux::make_tmux_session_name(session_id);
                if let Err(e) = fartcode_core::pty::tmux::kill_tmux_session(&name) {
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
        let killed = fartcode_core::pty::tmux::kill_tmux_sessions_by_prefix(&format!(
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
