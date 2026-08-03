//! Task lifecycle helpers (ARCHITECTURE.md §2, ticket E2-01).
//!
//! The reference has **no status-transition allowlist** — `updateTaskStatus`
//! accepts any lifecycle status; the only guards are same-status no-op and
//! not-found. (ARCHITECTURE.md §3's `InvalidStatusTransition` variant exists
//! for a future state machine; nothing enforces one today.) This module owns
//! the status-change mechanics and the telemetry hooks.

use crate::Error;

/// Applies a status change: returns `false` (no-op) when `new == old`,
/// otherwise sets the row's `status`, `status_changed_at`, and `updated_at`.
/// The caller must have already verified the task exists.
pub(crate) fn apply_status_change(
    conn: &rusqlite::Connection,
    id: &str,
    old_status: &str,
    new_status: &str,
) -> Result<bool, Error> {
    if old_status == new_status {
        return Ok(false);
    }
    conn.execute(
        "UPDATE tasks SET status = ?1, status_changed_at = datetime('now'), updated_at = datetime('now') WHERE id = ?2",
        rusqlite::params![new_status, id],
    )?;
    Ok(true)
}

/// Phase-0 telemetry hooks. `ade-telemetry` (allowlisted events) is a Phase 2
/// crate; until then, lifecycle events are recorded via `tracing` so the
/// Phase 2 client can be wired at the same call sites.
pub(crate) fn telemetry(event: &str, fields: &[(&str, String)]) {
    let rendered = fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!(event, "{rendered}");
}
