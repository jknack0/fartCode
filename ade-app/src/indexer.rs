//! Event-driven FTS indexer (E1-09): keeps `search_index` current as
//! projects/tasks/conversations are created/renamed/deleted, plus a boot
//! backfill.

use std::sync::Arc;

use ade_core::conversations::ConversationStore;
use ade_core::db::Db;
use ade_core::events::{BroadcastEventBus, EventBus, InternalEvent};
use ade_core::projects::ProjectStore;
use ade_core::tasks::TaskStore;

/// Boot-time backfill + event subscription. Runs for the app's lifetime.
pub fn spawn_search_indexer(
    db: Arc<dyn Db>,
    projects: Arc<dyn ProjectStore>,
    tasks: Arc<dyn TaskStore>,
    conversations: Arc<dyn ConversationStore>,
    event_bus: Arc<BroadcastEventBus>,
) {
    // Backfill from source tables (fresh DBs have no rows). Conversations are
    // included so they stay searchable across restarts.
    match backfill(&db, &projects, &tasks, &conversations) {
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
                Ok(InternalEvent::ConversationCreated {
                    id, task_id, title, ..
                }) => {
                    let _ = ade_core::search::upsert(
                        &db,
                        "conversation",
                        &id,
                        None,
                        Some(&task_id),
                        &title,
                        &[title.as_str()],
                    );
                }
                Ok(InternalEvent::ConversationRenamed { id, title }) => {
                    // UPDATE, not upsert: FTS5 INSERT OR REPLACE would wipe
                    // the task_id/project_id link the palette navigates with.
                    let _ = ade_core::search::update_title(&db, "conversation", &id, &title);
                }
                Ok(InternalEvent::ConversationDeleted { id }) => {
                    let _ = ade_core::search::delete(&db, "conversation", &id);
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
    conversations: &Arc<dyn ConversationStore>,
) -> Result<usize, ade_core::Error> {
    let project_rows: Vec<(String, String)> = projects
        .list()?
        .into_iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let mut task_rows: Vec<(String, String, String)> = Vec::new();
    let mut conversation_rows: Vec<(String, String, String)> = Vec::new();
    for (project_id, _) in &project_rows {
        for t in tasks.list_by_project(project_id)? {
            task_rows.push((t.id.clone(), t.project_id.clone(), t.name.clone()));
            for c in conversations.list_by_task(&t.id)? {
                conversation_rows.push((
                    c.id.clone(),
                    c.task_id.clone().unwrap_or_default(),
                    c.title.clone(),
                ));
            }
        }
    }
    ade_core::search::backfill(db, &project_rows, &task_rows, &conversation_rows)?;
    Ok(project_rows.len() + task_rows.len() + conversation_rows.len())
}
