//! ade-runtime — out-of-process workers (E2-11-2).
//!
//! The ACP runtime runs OUTSIDE the Tauri main process (mirrors the
//! reference's out-of-process design; keeps protocol state out of the
//! main process):
//!
//! - [`protocol`] — the ade-app ↔ worker wire schema. The renderer-facing
//!   [`protocol::SessionInput`] includes an `env` field that is ALWAYS
//!   discarded by [`protocol::prepare_session`] — env reaching the adapter
//!   comes only from server-side provider settings (E3-07 keyring).
//! - [`process_host`] — adapter spawn abstraction: local now, SSH seam
//!   for Phase 3.
//! - [`session_host`] — app-side driver of the `ade-acp-runtime` worker
//!   binary.
//! - `bin/ade-acp-runtime` — the worker: hosts one ACP session against an
//!   adapter binary, forwards `session/update` streams, and routes
//!   permission requests back to the host.
//! - [`attachments`] — per-conversation attachments dir under app data.

pub mod attachments;
pub mod error;
pub mod process_host;
pub mod protocol;
pub mod session_host;

pub use attachments::attachments_dir;
pub use error::Error;
pub use process_host::{AdapterProcessConfig, LocalProcessHost, ProcessHost, RestartPolicy};
pub use protocol::{prepare_session, SessionInput, WorkerSession};
pub use session_host::SessionHost;
