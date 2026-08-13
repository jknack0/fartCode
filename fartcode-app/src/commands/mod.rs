//! Tauri command modules (E1-04): thin wrappers over the domain services.
//! Commands map errors to `String` and return DTOs (AGENTS.md: commands are
//! thin).
//!
//! Blocking command bodies go through [`off_main_thread`] — the one
//! sanctioned #80 shape (AGENTS.md § "Tauri commands and the main thread").

pub mod columns;
pub mod conversations;
pub mod dependencies;
pub mod dossiers;
pub mod files;
pub mod git;
pub mod github;
pub mod issue_proposals;
pub mod issues;
pub mod lifecycle;
pub mod line_comments;
pub mod port_forwards;
pub mod projects;
pub mod provider_accounts;
pub mod remote_projects;
pub mod search;
pub mod serde_util;
pub mod settings;
pub mod ssh_connections;
pub mod steps;
pub mod tasks;
pub mod telemetry;
pub mod terminals;
pub mod view_state;

/// Runs `work` on the blocking pool and awaits it, so the calling command
/// never occupies the IPC (main) thread while a subprocess, SQLite, or the
/// filesystem is busy (#80). `async` alone is not the fix — it only moves
/// the stall onto an async-runtime worker; the blocking body has to leave
/// the thread.
///
/// The closure owns everything it touches — commands clone the `Arc<App>`
/// out of `State` first, since `State<'_, _>` borrows the invoke scope and
/// cannot cross a thread boundary. A join failure (panic inside the
/// closure) becomes a plain command error rather than a lost invoke that
/// leaves the UI waiting forever.
pub(crate) async fn off_main_thread<T, F>(work: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("command did not complete: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::off_main_thread;
    use std::thread::ThreadId;

    #[test]
    fn off_main_thread_leaves_the_caller_and_propagates_both_outcomes() {
        let caller = std::thread::current().id();
        let ran_on = tauri::async_runtime::block_on(off_main_thread(move || {
            Ok::<ThreadId, String>(std::thread::current().id())
        }))
        .unwrap();
        assert_ne!(caller, ran_on, "closure ran on the calling thread");

        let err = tauri::async_runtime::block_on(off_main_thread(|| {
            Err::<(), String>("verbatim failure".into())
        }))
        .unwrap_err();
        assert_eq!(err, "verbatim failure");

        // A panic must become an error, not a promise the UI waits on forever.
        let err = tauri::async_runtime::block_on(off_main_thread(|| {
            panic!("boom");
            #[allow(unreachable_code)]
            Ok::<(), String>(())
        }))
        .unwrap_err();
        assert!(err.starts_with("command did not complete"), "got: {err}");
    }
}
