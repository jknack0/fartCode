//! E4-01: workspace watch lifecycle — boot backfill + event subscription.
//!
//! Boot registers every live (non-archived) task's workspace with the
//! `FsWatchService`; afterwards `TaskProvisioned` registers and
//! `TaskDeleted` unregisters (deletion tears the watch down before the
//! worktree is pruned events-wise — a pruned root simply stops producing).
//! Registration failures are non-fatal: a stale workspace row (worktree
//! gone from disk) must never block boot or provisioning.

use std::sync::Arc;

use fartcode_core::db::Db;
use fartcode_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use fartcode_core::fs_watch::{self, FsWatchService, WatchTarget};

/// Boot-time backfill + event subscription. Runs for the app's lifetime.
pub fn spawn_workspace_watchers(
    db: Arc<dyn Db>,
    fs_watch: Arc<FsWatchService>,
    event_bus: Arc<BroadcastEventBus>,
) {
    match fs_watch::boot_targets(db.as_ref()) {
        Ok(targets) => {
            let total = targets.len();
            let registered = targets
                .into_iter()
                .filter(|t| register(&fs_watch, t))
                .count();
            tracing::info!(registered, total, "workspace watches backfilled");
        }
        Err(e) => tracing::warn!(error = %e, "workspace watch backfill failed (non-fatal)"),
    }

    tauri::async_runtime::spawn(async move {
        let mut rx = event_bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(InternalEvent::TaskProvisioned { id, workspace_id }) => {
                    match fs_watch::target_for(db.as_ref(), &id, &workspace_id) {
                        Ok(Some(target)) => {
                            register(&fs_watch, &target);
                        }
                        Ok(None) => {} // no local path (remote/BYOI)
                        Err(e) => tracing::warn!(
                            task_id = %id,
                            error = %e,
                            "watch target lookup failed"
                        ),
                    }
                }
                Ok(InternalEvent::TaskDeleted { id }) => fs_watch.unregister_task(&id),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn register(fs_watch: &FsWatchService, target: &WatchTarget) -> bool {
    match fs_watch.register_task(
        &target.task_id,
        &target.project_id,
        &target.workspace_id,
        &target.worktree,
    ) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(
                task_id = %target.task_id,
                worktree = %target.worktree.display(),
                error = %e,
                "workspace watch registration failed (non-fatal)"
            );
            false
        }
    }
}
