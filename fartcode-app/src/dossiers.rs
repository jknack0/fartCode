//! Feature-dossier lifecycle (E19-01, #70; ADR-0038 items 1–2).
//!
//! The content and file primitives live in `fartcode_core::dossiers`. This
//! module is the half that needs the wired App: the consent gate, worktree
//! resolution, `issues.dossier_path`, and the event subscriber that appends
//! machine breadcrumbs to `## Timeline`.
//!
//! **Two invariants, both load-bearing.**
//!
//! 1. *Creation never fails a dispatch.* [`create_for_task`] returns
//!    `Option<String>` and swallows every error into a `tracing::warn!`.
//!    A read-only repo, a declined project, a vanished worktree — all leave
//!    `dossier_path` NULL and the agent running. The feature is memory,
//!    not a gate (ADR-0038 item 3: declining still dispatches).
//! 2. *Appending never blocks the emitting path.* The subscriber owns its
//!    own task and does its filesystem work inside `spawn_blocking`, per
//!    AGENTS.md "Tauri commands and the main thread" — the bus sender never
//!    waits on a disk write, and a failed append logs instead of
//!    propagating.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::OptionalExtension;

use fartcode_core::dossiers;
use fartcode_core::events::{EventBus, InternalEvent};
use fartcode_core::issues::columns::ColumnKind;
use fartcode_core::issues::{Issue, IssuePatch};
use fartcode_core::projects::ProjectStore;
use fartcode_core::tasks::TaskStore;

use crate::app::App;

/// Whether this project consented to dossier writes (ADR-0038 item 3).
///
/// `None` (never asked) resolves to **true** — the interim behavior
/// documented on [`fartcode_core::settings::BaseProjectSettings::feature_dossiers`],
/// where the reasoning lives. An unreadable settings row resolves to
/// **false**: when consent cannot be established, do not write to someone's
/// repo.
fn consented(app: &App, project_id: &str) -> bool {
    let Ok(Some(project)) = app.projects.get(project_id) else {
        return false;
    };
    match app
        .settings
        .get_project_settings(project_id, std::path::Path::new(&project.path))
    {
        Ok(settings) => settings.feature_dossiers.unwrap_or(true),
        Err(e) => {
            tracing::warn!(project_id, error = %e, "dossier consent unreadable — not writing");
            false
        }
    }
}

/// The materialized worktree root of a task, or `None` when it has no
/// workspace row / no path / the directory is gone.
fn task_worktree(app: &App, task_id: &str) -> Option<PathBuf> {
    let conn = app.db.conn().lock().ok()?;
    let path: Option<String> = conn
        .query_row(
            "SELECT w.path FROM tasks t
               JOIN workspaces w ON w.id = t.workspace_id
              WHERE t.id = ?1",
            [task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .ok()?
        .flatten();
    drop(conn);
    let path = PathBuf::from(path?);
    if path.as_os_str().is_empty() || !path.is_dir() {
        return None;
    }
    Some(path)
}

/// Births the feature dossier alongside the freshly provisioned worktree
/// (ADR-0038 item 1) and records its repo-relative path on the card.
///
/// Called from `dispatch::provision_issue_task` — the ONE provisioning
/// helper both the legacy board dispatch and the step engine's first
/// `agent_step` entry share — AFTER the worktree exists, so the file is
/// born on the feature branch and travels with it. Because provisioning
/// only happens when the card has no live linked task, a second step
/// column cannot mint a second dossier: later entries reuse the same
/// task/worktree and the `dossier_path` already on the row.
///
/// Returns the repo-relative path when the dossier is in place (freshly
/// written or adopted), `None` in every refusal or failure case. Never
/// returns `Err`: a dispatch must not die because a markdown file could
/// not be written.
pub fn create_for_task(app: &App, issue: &Issue, task_id: &str) -> Option<String> {
    if !consented(app, &issue.project_id) {
        tracing::debug!(issue = %issue.id, "feature dossiers off for this project — not writing");
        return None;
    }
    let worktree = task_worktree(app, task_id)?;

    // A re-provisioned card keeps the dossier it already has: reuse the
    // stored path rather than re-deriving a slug from a title that may
    // have been edited since.
    let rel = issue
        .dossier_path
        .clone()
        .unwrap_or_else(|| dossiers::dossier_relative_path(issue));

    // Only an `agent_step` column is named in the birth line. The legacy
    // `issue_dispatch` path provisions BEFORE it moves the card, so the
    // card is still sitting on a shelf here — naming that shelf would
    // claim a step entry that has not happened. The engine path (the
    // board's real one) has already entered the step column, so it names
    // it correctly.
    let column_name = issue
        .column_id
        .as_deref()
        .and_then(|id| app.columns.get(id).ok().flatten())
        .filter(|c| c.kind == ColumnKind::AgentStep)
        .map(|c| c.name);
    let header = dossiers::backfilled_header(issue, column_name.as_deref());

    match dossiers::create_dossier(&worktree, &rel, &header) {
        Ok(true) => tracing::info!(issue = %issue.id, path = %rel, "feature dossier created"),
        Ok(false) => {
            tracing::info!(issue = %issue.id, path = %rel, "feature dossier adopted (already present)")
        }
        Err(e) => {
            // ADR-0038: memory, not a gate. Log and leave the path NULL.
            tracing::warn!(issue = %issue.id, path = %rel, error = %e, "feature dossier write failed");
            return None;
        }
    }

    if issue.dossier_path.as_deref() != Some(rel.as_str()) {
        if let Err(e) = app.issues.update(
            &issue.id,
            IssuePatch {
                dossier_path: Some(Some(rel.clone())),
                ..Default::default()
            },
        ) {
            tracing::warn!(issue = %issue.id, error = %e, "recording dossier_path failed");
            return None;
        }
    }
    Some(rel)
}

// ---------------------------------------------------------------------------
// Timeline appender
// ---------------------------------------------------------------------------

/// Subscriber that appends machine breadcrumbs under `## Timeline`
/// (ADR-0038 item 2) from events the app ALREADY emits.
///
/// Wired facts: step launched (column · provider · model), step settled,
/// column moves, PR opened/merged. Creation and the pre-worktree history
/// are backfilled into the header at birth ([`create_for_task`]) rather
/// than appended, because they predate the file.
///
/// Writes only while a worktree exists: every append resolves the card's
/// linked task → workspace path → dossier file, and a missing link, a
/// pruned worktree, or a deleted file all mean "record nothing"
/// (ADR-0038 item 2: post-teardown events go unrecorded).
pub struct TimelineAppender {
    app: Arc<App>,
    /// Last column observed per issue — a column MOVE is a change in this
    /// map, since `IssueUpdated` is emitted for every field/edge/link write
    /// and carries no column. Seeded from the DB at [`Self::seed`] so a
    /// move made right after a restart is still recorded.
    last_column: Mutex<HashMap<String, String>>,
}

impl TimelineAppender {
    pub fn new(app: Arc<App>) -> Self {
        Self {
            app,
            last_column: Mutex::new(HashMap::new()),
        }
    }

    /// Boot backfill of the column map (mirrors the search indexer's
    /// pattern): only cards that HAVE a dossier can ever be appended to,
    /// so only those are tracked.
    pub fn seed(&self) {
        let Ok(conn) = self.app.db.conn().lock() else {
            return;
        };
        let Ok(mut stmt) =
            conn.prepare("SELECT id, column_id FROM issues WHERE dossier_path IS NOT NULL")
        else {
            return;
        };
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        });
        let Ok(rows) = rows else { return };
        let mut map = match self.last_column.lock() {
            Ok(map) => map,
            Err(_) => return,
        };
        for entry in rows.flatten() {
            if let (id, Some(column)) = entry {
                map.insert(id, column);
            }
        }
    }

    /// Resolves a card to `(worktree root, repo-relative dossier path)`, or
    /// `None` when there is nothing writable — no dossier, no linked task,
    /// no worktree on disk, or the file itself is gone with the branch.
    fn target(&self, issue_id: &str) -> Option<(PathBuf, String)> {
        let issue = self.app.issues.get(issue_id).ok().flatten()?;
        let rel = issue.dossier_path.clone()?;
        let task_id = issue.linked_task_id.as_deref()?;
        // A deleted task clears the link (ON DELETE SET NULL), but check
        // anyway: the row may be mid-teardown.
        self.app.tasks.get(task_id).ok().flatten()?;
        let worktree = task_worktree(&self.app, task_id)?;
        if !worktree.join(&rel).is_file() {
            return None;
        }
        Some((worktree, rel))
    }

    fn append(&self, issue_id: &str, fact: &str, once_key: Option<&str>) {
        let Some((worktree, rel)) = self.target(issue_id) else {
            return;
        };
        let line = dossiers::timeline_line(fact);
        if let Err(e) = dossiers::append_timeline(&worktree, &rel, &line, once_key) {
            tracing::warn!(issue = issue_id, path = %rel, error = %e, "dossier timeline append failed");
        }
    }

    fn column_name(&self, column_id: &str) -> String {
        self.app
            .columns
            .get(column_id)
            .ok()
            .flatten()
            .map(|c| c.name)
            .unwrap_or_else(|| column_id.to_string())
    }

    /// Handles one event. Synchronous and self-contained so tests can drive
    /// it without the bus; the spawned loop calls it inside `spawn_blocking`.
    pub fn handle(&self, event: &InternalEvent) {
        match event {
            // A reattach is a focus, not a launch — nothing new happened.
            InternalEvent::StepLaunch {
                issue_id,
                column_id,
                provider,
                model,
                reattached: false,
                ..
            } => {
                let mut fact = format!("{} · launched · {provider}", self.column_name(column_id));
                if let Some(model) = model.as_deref().filter(|m| !m.is_empty()) {
                    fact.push_str(" · ");
                    fact.push_str(model);
                }
                self.append(issue_id, &fact, None);
            }
            InternalEvent::StepSettled {
                issue_id,
                column_id,
                ..
            } => {
                let fact = format!("{} · settled", self.column_name(column_id));
                self.append(issue_id, &fact, None);
            }
            // `IssueUpdated` fires for every field/edge/link write and says
            // nothing about columns, so the move is detected by diffing the
            // card's current column against the last one seen.
            InternalEvent::IssueUpdated { id, .. } => self.on_issue_updated(id),
            InternalEvent::PrUpdated {
                workspace_id,
                pr_url,
            } => self.on_pr_updated(workspace_id, pr_url),
            _ => {}
        }
    }

    fn on_issue_updated(&self, issue_id: &str) {
        let Ok(Some(issue)) = self.app.issues.get(issue_id) else {
            return;
        };
        let Some(column_id) = issue.column_id.clone() else {
            return;
        };
        let previous = match self.last_column.lock() {
            Ok(mut map) => map.insert(issue_id.to_string(), column_id.clone()),
            Err(_) => return,
        };
        // First sighting (a card created after boot) seeds the map without
        // inventing a move it never saw.
        let Some(previous) = previous else { return };
        if previous == column_id {
            return;
        }
        let fact = format!("column → {}", self.column_name(&column_id));
        self.append(issue_id, &fact, None);
    }

    fn on_pr_updated(&self, workspace_id: &str, pr_url: &str) {
        // `PrUpdated` fires on every payload change (checks, comments), so
        // the interesting facts — opened, merged — are deduped by
        // `once_key` against the file itself. That keeps it stateless and
        // restart-safe: no in-memory "last status" to lose.
        let Ok(conn) = self.app.db.conn().lock() else {
            return;
        };
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM pull_requests WHERE url = ?1",
                [pr_url],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let issue_ids: Vec<String> = conn
            .prepare(
                "SELECT i.id FROM issues i
                   JOIN tasks t ON t.id = i.linked_task_id
                  WHERE t.workspace_id = ?1 AND i.dossier_path IS NOT NULL",
            )
            .and_then(|mut stmt| {
                stmt.query_map([workspace_id], |row| row.get::<_, String>(0))?
                    .collect()
            })
            .unwrap_or_default();
        drop(conn);

        let verb = match status.as_deref() {
            Some("open") => "pr opened",
            Some("merged") => "pr merged",
            // `closed` without a merge is not a lifecycle fact worth a line.
            _ => return,
        };
        let fact = format!("{verb} · {pr_url}");
        for issue_id in issue_ids {
            self.append(&issue_id, &fact, Some(&fact));
        }
    }
}

/// Boot wiring: seed the column map, then subscribe for the app's lifetime.
///
/// The filesystem work runs on the blocking pool, never on the async
/// runtime's worker (AGENTS.md) — the event bus is a broadcast channel, so
/// a slow subscriber only lags itself, but a blocked runtime worker is
/// everyone's problem.
pub fn spawn_dossier_timeline(app: Arc<App>) {
    let appender = Arc::new(TimelineAppender::new(app.clone()));
    appender.seed();
    tauri::async_runtime::spawn(async move {
        let mut rx = app.event_bus.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let appender = appender.clone();
                    let _ =
                        tauri::async_runtime::spawn_blocking(move || appender.handle(&event)).await;
                }
                // A lagging subscriber drops events but must survive; only
                // a closed bus ends the loop.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
