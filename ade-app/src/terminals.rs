//! Interactive task terminals (E2-12): PTY-backed shells in the task view.
//!
//! Spawns `$SHELL` (fallback `/bin/sh`) in the task's workspace with the
//! **inherited** env (`EnvPolicy::Inherit` — interactive shells get the
//! user's env, unlike agent launches which are allowlisted-only), pumps
//! output to the frontend as `terminal:output` events (base64 chunks), and
//! reports `terminal:exited` when the shell ends.

use std::collections::HashMap;
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
}

impl TerminalManager {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            pty: PortablePtyManager,
            app,
            terminals: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawns `program` (with `args`) in `cwd` and starts the output pump.
    /// Returns the terminal id.
    pub fn open(
        &self,
        task_id: &str,
        program: &str,
        args: &[String],
        cwd: &Path,
        rows: u16,
        cols: u16,
    ) -> Result<String, ade_core::Error> {
        let handle = self.pty.spawn(
            program,
            args,
            cwd,
            &[],
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
    pub fn close_task(&self, task_id: &str) {
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
    }
}
