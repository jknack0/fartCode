//! Event-driven FTS indexer (E1-09): keeps `search_index` current as
//! projects/tasks are created/renamed/deleted, plus a boot backfill.
//!
//! E19-03 (#72) adds the `feature` rows dossier sections produce. Their
//! WRITE path is elsewhere — step settle and project pull, in
//! `crate::dossier_index` — but their teardown belongs here, on the same
//! subscription that already drops a deleted project's and task's rows.

use std::sync::Arc;

use fartcode_core::db::Db;
use fartcode_core::events::{EventBus, InternalEvent};
use fartcode_core::projects::ProjectStore;
use fartcode_core::tasks::TaskStore;

use crate::app::App;

/// Boot-time backfill + event subscription. Runs for the app's lifetime.
pub fn spawn_search_indexer(app: Arc<App>) {
    // Backfill from source tables (fresh DBs have no rows). Cheap, bounded
    // SQL — safe to run before the window comes up.
    match backfill(&app) {
        Ok(n) => tracing::info!(docs = n, "search index backfilled"),
        Err(e) => tracing::warn!(error = %e, "search backfill failed (non-fatal)"),
    }

    tauri::async_runtime::spawn(async move {
        // Subscribe BEFORE the dossier sweep so deletions that land during
        // it are not missed.
        let mut rx = app.event_bus.subscribe();

        // `backfill` clears the whole table, so the `feature` rows have to
        // be rebuilt from the dossier files. That is filesystem work —
        // off the main thread, and after the window is already up.
        {
            let app = app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                crate::dossier_index::reindex_all(&app)
            })
            .await;
        }

        let db: Arc<dyn Db> = app.db.clone();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                // Best-effort observer: a dropped frame leaves the index
                // stale only until the next boot backfill rebuilds it. Log
                // the gap and keep consuming; only a closed bus ends the
                // loop.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        dropped = n,
                        "search indexer lagged; stale until next backfill"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            // E19-03: `feature`-row teardown (issue / project deletion)
            // rides the same subscription that already drops a deleted
            // project's and task's rows.
            crate::dossier_index::handle_event(&app, &event);
            match event {
                InternalEvent::ProjectAdded { id, name, .. } => {
                    let _ = fartcode_core::search::upsert(
                        &db,
                        "project",
                        &id,
                        None,
                        None,
                        &name,
                        &[name.as_str()],
                    );
                }
                InternalEvent::ProjectDeleted { id } => {
                    let _ = fartcode_core::search::delete(&db, "project", &id);
                }
                InternalEvent::TaskCreated {
                    id,
                    project_id,
                    name,
                } => {
                    let _ = fartcode_core::search::upsert(
                        &db,
                        "task",
                        &id,
                        Some(&project_id),
                        None,
                        &name,
                        &[name.as_str()],
                    );
                }
                InternalEvent::TaskRenamed { id, name, .. } => {
                    // #142: the rename path is the ONLY caller of
                    // `update_title` — a plain upsert would wipe the
                    // project/task link columns, and a no-op left the
                    // old title in the index forever.
                    let _ = fartcode_core::search::update_title(&db, "task", &id, &name);
                }
                InternalEvent::TaskDeleted { id } => {
                    let _ = fartcode_core::search::delete(&db, "task", &id);
                }
                _ => {}
            }
        }
    });
}

fn backfill(app: &App) -> Result<usize, fartcode_core::Error> {
    let project_rows: Vec<(String, String)> = app
        .projects
        .list()?
        .into_iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let mut task_rows: Vec<(String, String, String)> = Vec::new();
    for (project_id, _) in &project_rows {
        for t in app.tasks.list_by_project(project_id)? {
            task_rows.push((t.id.clone(), t.project_id.clone(), t.name.clone()));
        }
    }
    fartcode_core::search::backfill(&app.db, &project_rows, &task_rows)?;
    Ok(project_rows.len() + task_rows.len())
}
