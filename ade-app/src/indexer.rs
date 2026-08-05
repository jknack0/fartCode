//! Event-driven FTS indexer (E1-09): keeps `search_index` current as
//! projects/tasks are created/renamed/deleted, plus a boot backfill.

use std::sync::Arc;

use ade_core::db::Db;
use ade_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use ade_core::projects::ProjectStore;
use ade_core::tasks::TaskStore;

/// Boot-time backfill + event subscription. Runs for the app's lifetime.
pub fn spawn_search_indexer(
    db: Arc<dyn Db>,
    projects: Arc<dyn ProjectStore>,
    tasks: Arc<dyn TaskStore>,
    event_bus: Arc<BroadcastEventBus>,
) {
    // Backfill from source tables (fresh DBs have no rows).
    match backfill(&db, &projects, &tasks) {
        Ok(n) => tracing::info!(docs = n, "search index backfilled"),
        Err(e) => tracing::warn!(error = %e, "search backfill failed (non-fatal)"),
    }

    tauri::async_runtime::spawn(async move {
        let mut rx = event_bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(InternalEvent::ProjectAdded { id, name, .. }) => {
                    let _ = ade_core::search::upsert(
                        &db,
                        "project",
                        &id,
                        None,
                        None,
                        &name,
                        &[name.as_str()],
                    );
                }
                Ok(InternalEvent::ProjectDeleted { id }) => {
                    let _ = ade_core::search::delete(&db, "project", &id);
                }
                Ok(InternalEvent::TaskCreated {
                    id,
                    project_id,
                    name,
                }) => {
                    let _ = ade_core::search::upsert(
                        &db,
                        "task",
                        &id,
                        Some(&project_id),
                        None,
                        &name,
                        &[name.as_str()],
                    );
                }
                Ok(InternalEvent::TaskDeleted { id }) => {
                    let _ = ade_core::search::delete(&db, "task", &id);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn backfill(
    db: &Arc<dyn Db>,
    projects: &Arc<dyn ProjectStore>,
    tasks: &Arc<dyn TaskStore>,
) -> Result<usize, ade_core::Error> {
    let project_rows: Vec<(String, String)> = projects
        .list()?
        .into_iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let mut task_rows: Vec<(String, String, String)> = Vec::new();
    for (project_id, _) in &project_rows {
        for t in tasks.list_by_project(project_id)? {
            task_rows.push((t.id.clone(), t.project_id.clone(), t.name.clone()));
        }
    }
    ade_core::search::backfill(db, &project_rows, &task_rows)?;
    Ok(project_rows.len() + task_rows.len())
}
