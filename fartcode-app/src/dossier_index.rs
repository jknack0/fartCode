//! Feature-dossier ⌘K indexing (E19-03, #72; ADR-0038 item 4).
//!
//! `fartcode_core::dossier_index` turns dossier TEXT into `feature` rows.
//! This module is the half that needs the wired App: deciding WHICH copy of
//! a dossier to read, and when.
//!
//! **Two copies, one row set.** A dossier is born inside the worktree and
//! rides the feature branch (ADR-0038 item 1), then lands on main when the
//! branch merges (item 5). So the same file exists in up to two places at
//! different times, and the rows they produce are keyed by card + section
//! heading — identical either way. [`reindex_issue`] prefers the worktree
//! copy while one exists (it is strictly fresher: the agent is writing into
//! it right now) and falls back to the project checkout for landed
//! features, whose worktree is long gone.
//!
//! **When.** Step settle (`step_engine::settle_issues_for_task` — the
//! moment after the agent appended its `## <Column> — <date>` section and
//! before the card moves on) and project pull (`project_git_pull` — the
//! moment merged dossiers appear in the main checkout), exactly as ADR-0038
//! item 4 specifies, plus a boot sweep so a fresh DB and a torn-down
//! worktree both converge.
//!
//! **Never a gate, and never a second consent check.** Dossiers only exist
//! under consent, so indexing follows the data: if the file is there, it
//! was consented to; if it is not, there is nothing to index. Every entry
//! point here returns `()` and logs — a reindex must not fail a settle or a
//! pull.

use std::path::PathBuf;

use fartcode_core::dossier_index as core_index;
use fartcode_core::events::InternalEvent;
use fartcode_core::issues::Issue;
use fartcode_core::projects::ProjectStore;

use crate::app::App;
use crate::dossiers::task_worktree;

/// Teardown arm of the search indexer's subscription: drops `feature` rows
/// whose card or project no longer exists.
///
/// "The same delete path tasks use today" (ADR-0038) — literally the same
/// subscription, in `crate::indexer`. Synchronous and self-contained so
/// tests can drive it without the bus, matching
/// [`crate::dossiers::TimelineAppender::handle`].
///
/// A left-behind row is not cosmetic: ⌘K would return a hit whose Enter
/// opens the card detail of a deleted issue.
pub fn handle_event(app: &App, event: &InternalEvent) {
    let (id, dropped) = match event {
        InternalEvent::IssueDeleted { id, .. } => (id, core_index::forget_issue(&app.db, id)),
        // A project delete cascades its issues in SQL WITHOUT emitting
        // `IssueDeleted` for each, so the arm above never sees them.
        InternalEvent::ProjectDeleted { id } => (id, core_index::forget_project(&app.db, id)),
        _ => return,
    };
    match dropped {
        Ok(0) => {}
        Ok(rows) => tracing::info!(id = %id, rows, "feature rows dropped"),
        Err(e) => tracing::warn!(id = %id, error = %e, "dropping feature rows failed"),
    }
}

/// Brings one card's `feature` rows in line with its dossier on disk.
///
/// Resolves the freshest copy of the file, reindexes from it, and prunes
/// rows for sections that are no longer in it. When NEITHER copy exists the
/// card's rows are dropped: the dossier left with a deleted unmerged branch
/// (ADR-0038 item 5), and a row pointing at content nobody can open is
/// worse than no row.
///
/// A card with no `dossier_path` is a no-op — it never had rows.
pub fn reindex_issue(app: &App, issue: &Issue) {
    let Some(rel) = issue
        .dossier_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    else {
        return;
    };

    let Some(path) = dossier_source(app, issue, rel) else {
        match core_index::forget_issue(&app.db, &issue.id) {
            Ok(0) => {}
            Ok(rows) => {
                tracing::info!(issue = %issue.id, rows, "dossier gone — feature rows dropped")
            }
            Err(e) => tracing::warn!(issue = %issue.id, error = %e, "dropping feature rows failed"),
        }
        return;
    };

    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        // Unreadable (permissions, mid-rename, binary): leave the rows
        // alone rather than delete a section list we could not confirm is
        // gone. The next reindex settles it.
        Err(e) => {
            tracing::warn!(issue = %issue.id, path = %path.display(), error = %e, "dossier unreadable");
            return;
        }
    };

    match core_index::reindex(&app.db, &issue.id, &issue.project_id, &content) {
        Ok(rows) => {
            tracing::debug!(issue = %issue.id, rows, path = %path.display(), "dossier indexed")
        }
        Err(e) => tracing::warn!(issue = %issue.id, error = %e, "dossier index write failed"),
    }
}

/// The dossier copy to index: the live worktree's first, else the project
/// checkout's. `None` when the file exists in neither.
fn dossier_source(app: &App, issue: &Issue, rel: &str) -> Option<PathBuf> {
    let live = issue
        .linked_task_id
        .as_deref()
        .and_then(|task_id| task_worktree(app, task_id))
        .map(|root| join_rel(root, rel))
        .filter(|p| p.is_file());
    if live.is_some() {
        return live;
    }
    let project = app.projects.get(&issue.project_id).ok().flatten()?;
    let landed = join_rel(project.path, rel);
    landed.is_file().then_some(landed)
}

/// `dossier_path` is a REPO path — always forward slashes, app-generated
/// from a `[a-z0-9-]` slug — so it is joined segment by segment rather than
/// handed to `Path::join`, which would treat it as a native path.
fn join_rel(root: PathBuf, rel: &str) -> PathBuf {
    let mut path = root;
    for segment in rel.split('/').filter(|s| !s.is_empty()) {
        path.push(segment);
    }
    path
}

/// Reindexes every dossier-bearing card in a project — the project-pull
/// hook (ADR-0038 item 4: "reindex on project pull (main-branch copy)"),
/// run after the pull so freshly merged dossiers are visible.
///
/// Whole-project rather than per-card because a pull lands many branches'
/// dossiers at once and the app is not told which.
pub fn reindex_project(app: &App, project_id: &str) {
    let issues = match app.issues.list_for_project(project_id) {
        Ok(issues) => issues,
        Err(e) => {
            tracing::warn!(project_id, error = %e, "dossier reindex: issue list failed");
            return;
        }
    };
    for issue in issues.iter().filter(|i| i.dossier_path.is_some()) {
        reindex_issue(app, issue);
    }
}

/// Boot sweep. The `search_index` backfill clears the whole table, so
/// without this the `feature` rows would vanish on every launch and only
/// come back at the next settle or pull.
///
/// It is also the cleanup that catches a worktree torn down while the app
/// was closed: `reindex_issue` drops the rows of a card whose dossier is in
/// neither copy.
///
/// Bounded by the number of dossier-bearing cards, one small file read
/// each — but still filesystem work, so callers run it off the main thread.
pub fn reindex_all(app: &App) {
    let projects = match app.projects.list() {
        Ok(projects) => projects,
        Err(e) => {
            tracing::warn!(error = %e, "dossier reindex: project list failed");
            return;
        }
    };
    for project in projects {
        reindex_project(app, &project.id);
    }
}
