//! ade-terminal — PTY implementation (E1-06/E2-06).
//!
//! Implements `ade_core::terminals::pty::PtyManager` with portable-pty.
//! The trait lives in `ade-core` so domain services never depend on this
//! crate (ADR-0014 records the deviation from ARCHITECTURE §7's
//! `ade_terminal::PtyManager` name — same wiring, trait moved core-side).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use ade_core::terminals::pty::{PtyExit, PtyHandle, PtyManager, PtySize};
use ade_core::Error;
use portable_pty::{native_pty_system, Child, CommandBuilder, PtyPair};

/// Concrete `PtyManager` backed by portable-pty.
#[derive(Default)]
pub struct PortablePtyManager;

/// Reader-buffer cap: a script that daemonizes a grandchild holding the pty
/// slave never sends EOF, so the background reader would otherwise grow
/// unboundedly. Oldest bytes are dropped (newest kept).
const READER_BUFFER_CAP: usize = 1024 * 1024;

struct PortablePtyHandle {
    writer: Option<Box<dyn std::io::Write + Send>>,
    reader_buffer: Arc<Mutex<Vec<u8>>>,
    reader_thread: Option<JoinHandle<()>>,
    child: Box<dyn Child + Send>,
}

impl PtyManager for PortablePtyManager {
    fn spawn(
        &self,
        cmd: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        size: PtySize,
    ) -> Result<Box<dyn PtyHandle>, Error> {
        let pty_system = native_pty_system();
        let pair: PtyPair = pty_system
            .openpty(portable_pty::PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Pty(format!("openpty: {e}")))?;

        let mut builder = CommandBuilder::new(cmd);
        for arg in args {
            builder.arg(arg);
        }
        builder.cwd(cwd);
        for (key, value) in env {
            builder.env(key, value);
        }

        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| Error::Pty(format!("spawn {cmd}: {e}")))?;
        // Drop the slave so EOF propagates when the child exits.
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::Pty(format!("take_writer: {e}")))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::Pty(format!("try_clone_reader: {e}")))?;

        // Background reader: drains the master continuously so output is
        // never lost while the caller waits on the child (portable-pty 0.9
        // has no non-blocking read on the master itself).
        let reader_buffer = Arc::new(Mutex::new(Vec::new()));
        let reader_thread = {
            let reader_buffer = reader_buffer.clone();
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                loop {
                    match reader.read(&mut chunk) {
                        Ok(0) | Err(_) => break, // EOF or error: reader done
                        Ok(n) => {
                            if let Ok(mut buf) = reader_buffer.lock() {
                                buf.extend_from_slice(&chunk[..n]);
                                if buf.len() > READER_BUFFER_CAP {
                                    let drop = buf.len() - READER_BUFFER_CAP;
                                    buf.drain(..drop);
                                }
                            }
                        }
                    }
                }
            })
        };

        Ok(Box::new(PortablePtyHandle {
            writer: Some(writer),
            reader_buffer,
            reader_thread: Some(reader_thread),
            child,
        }))
    }
}

impl PtyHandle for PortablePtyHandle {
    fn write(&mut self, data: &str) -> Result<(), Error> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| Error::Pty("writer already taken (EOF sent)".into()))?;
        writer
            .write_all(data.as_bytes())
            .map_err(|e| Error::Pty(format!("pty write: {e}")))
    }

    fn try_read(&mut self, buf: &mut Vec<u8>) -> Result<bool, Error> {
        let mut buffer = self
            .reader_buffer
            .lock()
            .map_err(|_| Error::Internal("pty reader buffer poisoned".into()))?;
        if buffer.is_empty() {
            return Ok(false);
        }
        buf.extend_from_slice(&buffer);
        buffer.clear();
        Ok(true)
    }

    fn wait_exit(&mut self, timeout: Duration) -> Result<PtyExit, Error> {
        // portable-pty's Child::try_wait() is non-blocking — poll it with a
        // deadline so timeouts are enforceable (the reference supports
        // optional `timeoutMs` on lifecycle runs).
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return Ok(PtyExit {
                        exit_code: Some(status.exit_code()),
                        signal: status.signal().map(String::from),
                    })
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(Error::LifecycleScriptTimeout("pty wait".into()));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(Error::Pty(format!("wait: {e}"))),
            }
        }
    }

    fn flush(&mut self) -> Result<(), Error> {
        // Give the reader a bounded grace to land the child's final chunk
        // (child reap and pty EOF are separate kernel events). The wait is
        // BOUNDED: a daemonized grandchild holding the slave would otherwise
        // block the join forever.
        if let Some(thread) = &self.reader_thread {
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    }

    fn kill(&mut self) -> Result<(), Error> {
        let _ = self.child.kill();
        // Reap the child so the timeout path doesn't leak a zombie.
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Dropping the writer sends EOF to the slave; the reader thread
        // observes it and exits.
        self.writer.take();
        Ok(())
    }
}

impl Drop for PortablePtyHandle {
    fn drop(&mut self) {
        // Never leave a running child behind — the trait contract must
        // survive callers that drop after a timeout. The reader thread join
        // is BOUNDED so a daemonized grandchild holding the slave can't hang
        // the drop; the reader exits on its own when the pty master is closed.
        let _ = self.child.kill();
        self.writer.take();
        if let Some(thread) = &self.reader_thread {
            let deadline = std::time::Instant::now() + Duration::from_millis(200);
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        self.reader_thread = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn spawns_shell_and_reads_output() {
        let mgr = PortablePtyManager;
        let tmp = tempfile::tempdir().unwrap();
        let mut handle = mgr
            .spawn("/bin/bash", &[], tmp.path(), &[], PtySize::default())
            .unwrap();
        handle.write("echo hello-from-pty\nexit\n").unwrap();
        let exit = handle
            .wait_exit(Duration::from_secs(10))
            .expect("shell exits");
        assert!(exit.is_success(), "{exit:?}");

        let mut buf = Vec::new();
        let mut out = String::new();
        while handle.try_read(&mut buf).unwrap_or(false) {
            out.push_str(&String::from_utf8_lossy(&buf));
            buf.clear();
        }
        assert!(out.contains("hello-from-pty"), "got output: {out:?}");
    }

    #[test]
    fn env_reaches_the_child() {
        let mgr = PortablePtyManager;
        let tmp = tempfile::tempdir().unwrap();
        let mut handle = mgr
            .spawn(
                "/bin/bash",
                &[],
                tmp.path(),
                &[("ADE_TEST_VAR".into(), "is-here".into())],
                PtySize::default(),
            )
            .unwrap();
        handle.write("echo $ADE_TEST_VAR\nexit\n").unwrap();
        handle.wait_exit(Duration::from_secs(10)).unwrap();
        let mut buf = Vec::new();
        let mut out = String::new();
        while handle.try_read(&mut buf).unwrap_or(false) {
            out.push_str(&String::from_utf8_lossy(&buf));
            buf.clear();
        }
        assert!(out.contains("is-here"), "env visible: {out:?}");
    }

    #[test]
    fn timeout_is_enforced() {
        let mgr = PortablePtyManager;
        let tmp = tempfile::tempdir().unwrap();
        let mut handle = mgr
            .spawn("/bin/bash", &[], tmp.path(), &[], PtySize::default())
            .unwrap();
        handle.write("sleep 5\n").unwrap();
        let err = handle.wait_exit(Duration::from_millis(200)).unwrap_err();
        assert!(matches!(err, Error::LifecycleScriptTimeout(_)), "{err:?}");
        handle.kill().unwrap();
    }

    #[test]
    fn flush_is_bounded_with_a_daemonized_grandchild() {
        // E1-06 regression: a script that daemonizes a grandchild holding the
        // pty slave never sends EOF — flush()/Drop must return bounded and
        // the reader buffer must stay capped.
        let mgr = PortablePtyManager;
        let tmp = tempfile::tempdir().unwrap();
        let mut handle = mgr
            .spawn("/bin/bash", &[], tmp.path(), &[], PtySize::default())
            .unwrap();
        // >1 MiB of output (exercises the cap) + a backgrounded grandchild
        // that keeps the slave open after the shell exits.
        handle
            .write("dd if=/dev/zero bs=4096 count=512 2>/dev/null\nsleep 2 &\nexit\n")
            .unwrap();
        handle.wait_exit(Duration::from_secs(10)).unwrap();

        let started = std::time::Instant::now();
        handle.flush().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "flush must be bounded, took {:?}",
            started.elapsed()
        );

        let mut buf = Vec::new();
        let mut total = 0usize;
        while handle.try_read(&mut buf).unwrap_or(false) {
            total += buf.len();
            buf.clear();
        }
        assert!(
            total <= READER_BUFFER_CAP + 8192,
            "reader buffer capped: drained {total} bytes"
        );
        handle.kill().unwrap();
    }

    #[test]
    fn handle_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        let mgr = PortablePtyManager;
        let tmp = tempfile::tempdir().unwrap();
        let h = mgr
            .spawn("/bin/bash", &[], tmp.path(), &[], PtySize::default())
            .unwrap();
        assert_send(&h);
        let _ = Arc::new(mgr); // PtyManager must be Sync for Arc<dyn PtyManager>
    }
}
