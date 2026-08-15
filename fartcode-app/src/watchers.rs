//! E4-01: workspace watch lifecycle — boot backfill + event subscription.
//!
//! Boot registers every live (non-archived) task's workspace with the
//! `FsWatchService`; afterwards `TaskProvisioned` and `TaskRestored`
//! register, `TaskArchived` and `TaskDeleted` unregister (deletion tears
//! the watch down before the worktree is pruned events-wise — a pruned root
//! simply stops producing).
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
    backfill(db.as_ref(), &fs_watch);
    let rx = event_bus.subscribe();
    tauri::async_runtime::spawn(watch_task_events(db, fs_watch, rx));
}

/// Registers every live task's workspace from the DB (`register_task` is
/// idempotent per task, so re-runs only add what's missing). Runs at boot,
/// and again when the subscription lags: this loop is the only runtime
/// registration path, so a `TaskProvisioned` among the dropped frames would
/// otherwise leave its workspace unwatched until the next app start.
fn backfill(db: &dyn Db, fs_watch: &FsWatchService) {
    match fs_watch::boot_targets(db) {
        Ok(targets) => {
            let total = targets.len();
            let registered = targets
                .into_iter()
                .filter(|t| register(fs_watch, t))
                .count();
            tracing::info!(registered, total, "workspace watches backfilled");
        }
        Err(e) => tracing::warn!(error = %e, "workspace watch backfill failed (non-fatal)"),
    }
}

/// Subscription loop, split from the spawn so tests can drive it against a
/// tiny-capacity bus without a Tauri app. Only a closed bus ends it.
async fn watch_task_events(
    db: Arc<dyn Db>,
    fs_watch: Arc<FsWatchService>,
    mut rx: tokio::sync::broadcast::Receiver<InternalEvent>,
) {
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
            // Restore mirrors provisioning, but the event carries no
            // workspace id — resolve it from the task row.
            Ok(InternalEvent::TaskRestored { id }) => {
                match fs_watch::target_for_task(db.as_ref(), &id) {
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
            Ok(InternalEvent::TaskArchived { id }) => fs_watch.unregister_task(&id),
            Ok(InternalEvent::TaskDeleted { id }) => fs_watch.unregister_task(&id),
            Ok(_) => {}
            // Dropped frames may have included a TaskProvisioned whose
            // workspace was never registered — and `resync_all` reaches only
            // *registered* layouts. Re-run the DB backfill first so those
            // workspaces are watched, then mark everything git-dirty so
            // consumers refetch instead of trusting state cached across the
            // gap. A missed TaskDeleted needs nothing: a leaked watch on a
            // pruned root is inert (the root produces no events).
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(
                    dropped = n,
                    "workspace watcher lagged; re-registering and forcing resync"
                );
                backfill(db.as_ref(), &fs_watch);
                fs_watch.resync_all();
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use fartcode_core::db::SqliteDb;
    use fartcode_core::events::EventBus;
    use std::path::Path;
    use std::time::{Duration, Instant};

    fn seed_task(db: &dyn Db, task_id: &str, workspace_id: &str, worktree: &Path) {
        let conn = db.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO workspaces (id, kind, path) VALUES (?1, 'worktree', ?2)",
            [workspace_id, worktree.to_str().unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, project_id, name, status, workspace_id)
             VALUES (?1, 'p1', ?1, 'running', ?2)",
            [task_id, workspace_id],
        )
        .unwrap();
    }

    /// Collects `GitChanged` workspace ids until both appear or the deadline
    /// passes (the fs_watch dispatcher debounces, so the fan-out is async).
    fn git_changed_workspaces(
        rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>,
        want: usize,
    ) -> std::collections::BTreeSet<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw = std::collections::BTreeSet::new();
        while saw.len() < want && Instant::now() < deadline {
            match rx.try_recv() {
                Ok(InternalEvent::GitChanged { workspace_id, .. }) => {
                    saw.insert(workspace_id);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        saw
    }

    /// #137: archiving a task stops its watch; restoring re-registers it,
    /// so the Changes panel refreshes again without an app restart.
    #[tokio::test(flavor = "multi_thread")]
    async fn archive_unregisters_and_restore_reregisters() {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        db.conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'P', '/proj')",
                [],
            )
            .unwrap();
        let wt = tempfile::tempdir().unwrap();
        seed_task(db.as_ref(), "t1", "w1", wt.path());

        let out_bus = Arc::new(BroadcastEventBus::new(256));
        let fs_watch = Arc::new(FsWatchService::new(out_bus.clone() as Arc<dyn EventBus>).unwrap());

        let t1 = fs_watch::target_for(db.as_ref(), "t1", "w1")
            .unwrap()
            .expect("t1 has a local workspace");
        assert!(register(&fs_watch, &t1));

        // Archive: the loop must drop the watch.
        let bus = BroadcastEventBus::new(8);
        let rx = bus.subscribe();
        bus.send(InternalEvent::TaskArchived { id: "t1".into() });
        drop(bus);
        watch_task_events(db.clone(), fs_watch.clone(), rx).await;

        // The tempdir is not a git repo, so a watched write surfaces as
        // `FilesChanged` (git: None layout emits no GitChanged).
        let mut out_rx = out_bus.subscribe();
        std::fs::write(wt.path().join("archived.txt"), "unwatched\n").unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let mut saw_while_archived = false;
        while let Ok(ev) = out_rx.try_recv() {
            if matches!(
                ev,
                InternalEvent::FilesChanged { .. } | InternalEvent::GitChanged { .. }
            ) {
                saw_while_archived = true;
            }
        }
        assert!(
            !saw_while_archived,
            "archived task's worktree must be unwatched"
        );

        // Restore: the loop must re-register from the task row alone.
        let bus = BroadcastEventBus::new(8);
        let rx = bus.subscribe();
        bus.send(InternalEvent::TaskRestored { id: "t1".into() });
        drop(bus);
        watch_task_events(db.clone(), fs_watch.clone(), rx).await;

        std::fs::write(wt.path().join("restored.txt"), "watched again\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut rewatched = false;
        while !rewatched && Instant::now() < deadline {
            match out_rx.try_recv() {
                Ok(InternalEvent::FilesChanged { workspace_id, .. }) if workspace_id == "w1" => {
                    rewatched = true;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        assert!(rewatched, "restored task's worktree must be watched again");
    }

    /// The review scenario: a `TaskProvisioned` among the dropped frames.
    /// The lag self-heal must register the missed workspace (resync alone
    /// reaches only registered layouts) and mark every workspace git-dirty.
    #[tokio::test(flavor = "multi_thread")]
    async fn lagged_watcher_registers_missed_provisions_and_resyncs() {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        db.conn()
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'P', '/proj')",
                [],
            )
            .unwrap();
        let wt1 = tempfile::tempdir().unwrap();
        let wt2 = tempfile::tempdir().unwrap();
        seed_task(db.as_ref(), "t1", "w1", wt1.path());
        seed_task(db.as_ref(), "t2", "w2", wt2.path());

        let out_bus = Arc::new(BroadcastEventBus::new(256));
        let fs_watch = Arc::new(FsWatchService::new(out_bus.clone() as Arc<dyn EventBus>).unwrap());
        let mut out_rx = out_bus.subscribe();

        // t1 registered normally; t2's TaskProvisioned is about to be lost.
        let t1 = fs_watch::target_for(db.as_ref(), "t1", "w1")
            .unwrap()
            .expect("t1 has a local workspace");
        assert!(register(&fs_watch, &t1));

        // Capacity-1 bus: an un-drained receiver lags once the ring wraps.
        // Dropping the bus closes it so the loop ends after draining.
        let lag_bus = BroadcastEventBus::new(1);
        let rx = lag_bus.subscribe();
        for i in 0..3 {
            lag_bus.send(InternalEvent::ProjectDeleted {
                id: format!("p{i}"),
            });
        }
        drop(lag_bus);

        watch_task_events(db.clone(), fs_watch.clone(), rx).await;

        let saw = git_changed_workspaces(&mut out_rx, 2);
        assert!(
            saw.contains("w2"),
            "workspace provisioned during the lag gap must be registered and resynced: {saw:?}"
        );
        assert!(
            saw.contains("w1"),
            "already-registered workspace must be marked dirty too: {saw:?}"
        );
    }
}
