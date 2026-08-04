//! Task deletion / teardown (E2-09, reference `deleteTask.ts` +
//! `task-lifecycle-utils.ts`).
//!
//! Order of operations (reference-faithful):
//!
//! 1. Fetch the task — a vanished row is a clean no-op (double-delete and
//!    delete-during-provision safety).
//! 2. Capture the workspace row BEFORE the task row disappears (metadata for
//!    worktree/branch cleanup; `workspaces` has no FK to `tasks`).
//! 3. Teardown: cancel every running PTY session of the task (registry
//!    cancel flag + tmux kill). Best-effort — a teardown failure warns and
//!    deletion continues (reference catches and proceeds).
//! 4. Delete the view-state keys (`task:<id>`, `task:<id>:tabs`).
//! 5. Delete the task row (conversations/terminals cascade via FK) + the
//!    workspace row when no sibling references it (`project-root` rows are
//!    never deleted).
//! 6. Remove the worktree when requested: sibling tasks block removal, the
//!    project root is never removed, paths outside the pool are refused, and
//!    `git worktree prune` runs with a bounded timeout (reference 5s).
//! 7. Delete the provisioned branch only when `delete_branch` is set AND the
//!    worktree was actually removed AND the branch was created from a
//!    DIFFERENT branch (never delete the branch the workspace merely used).

use std::sync::Arc;
use std::time::Duration;

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::conversations::{make_pty_session_id, ConversationStore};
use crate::db::Db;
use crate::projects::worktrees::WorktreeManager;
use crate::projects::ProjectStore;
use crate::pty::sessions::SessionRegistry;
use crate::pty::tmux::{kill_tmux_session, make_tmux_session_name};
use crate::tasks::TaskStore;
use crate::view_state;
use crate::Error;

/// Reference `TASK_TIMEOUT_MS` is 600s because the reference waits for
/// teardown SCRIPTS; Phase 0 teardown only reaps PTY children (near-instant),
/// so the wait window stays short and bounded.
const TEARDOWN_WAIT: Duration = Duration::from_secs(5);

/// Options for destructive task deletion (reference `DeleteTaskOptions`).
#[derive(Debug, Clone, Copy)]
pub struct DeleteTaskOptions {
    /// Remove the task's worktree when no sibling task still uses it.
    pub delete_worktree: bool,
    /// Also delete the provisioned branch (only when the worktree was
    /// removed and the branch was created from a different branch).
    pub delete_branch: bool,
}

impl Default for DeleteTaskOptions {
    fn default() -> Self {
        Self {
            delete_worktree: true,
            delete_branch: false,
        }
    }
}

/// Workspace row snapshot captured before the task row is deleted.
#[derive(Debug, Clone)]
struct WorkspaceSnapshot {
    id: String,
    kind: Option<String>,
    path: Option<String>,
    branch_name: Option<String>,
    config: Option<Value>,
}

/// E2-09 operation service. Wired once in the `App` struct (ARCHITECTURE §7).
pub struct TaskDeletionService {
    db: Arc<dyn Db>,
    tasks: Arc<dyn TaskStore>,
    conversations: Arc<dyn ConversationStore>,
    projects: Arc<dyn ProjectStore>,
    worktrees: WorktreeManager,
    sessions: Arc<SessionRegistry>,
}

impl TaskDeletionService {
    pub fn new(
        db: Arc<dyn Db>,
        tasks: Arc<dyn TaskStore>,
        conversations: Arc<dyn ConversationStore>,
        projects: Arc<dyn ProjectStore>,
        worktrees: WorktreeManager,
        sessions: Arc<SessionRegistry>,
    ) -> Self {
        Self {
            db,
            tasks,
            conversations,
            projects,
            worktrees,
            sessions,
        }
    }

    /// Deletes a task with full teardown (reference `deleteTask`). Idempotent:
    /// a vanished task is a clean no-op. All process/git cleanup failures are
    /// non-fatal warnings — the rows are the contract.
    pub fn delete_task(
        &self,
        project_id: &str,
        task_id: &str,
        options: &DeleteTaskOptions,
    ) -> Result<(), Error> {
        // 1. Idempotent early return (double-delete, delete-during-provision).
        if self.tasks.get(task_id)?.is_none() {
            return Ok(());
        }

        // 2. Workspace metadata BEFORE the row vanishes.
        let ws = self.workspace_snapshot(task_id)?;

        // 3. Teardown running sessions (PTY cancel + tmux kill), best-effort.
        self.teardown_sessions(project_id, task_id);

        // 4. View state (fire-and-forget; boot prune is the backstop).
        if let Err(e) = view_state::delete(&self.db, &format!("view-state:task:{task_id}")) {
            tracing::warn!(error = %e, "view-state delete failed (non-fatal)");
        }
        if let Err(e) = view_state::delete(&self.db, &format!("view-state:task:{task_id}:tabs")) {
            tracing::warn!(error = %e, "view-state tabs delete failed (non-fatal)");
        }

        // 5. Rows: task + (workspace when unused) + cascade + events.
        self.tasks.delete(task_id)?;

        // 6. Worktree removal (only for worktree-kind workspaces).
        let mut worktree_removed = false;
        if options.delete_worktree {
            if let Some(ws) = &ws {
                if ws.kind.as_deref() == Some("worktree") {
                    match self.remove_worktree_if_unused(project_id, task_id, ws) {
                        Ok(removed) => worktree_removed = removed,
                        Err(e) => {
                            tracing::warn!(
                                task_id,
                                workspace_id = %ws.id,
                                error = %e,
                                "worktree cleanup failed (non-fatal)"
                            );
                        }
                    }
                }
            }
        }

        // 7. Branch deletion (reference gate: worktreeRemoved &&
        //    deleteBranch && provisioned != fromBranch).
        if worktree_removed && options.delete_branch {
            if let Some(ws) = &ws {
                self.delete_provisioned_branch(project_id, ws);
            }
        }

        Ok(())
    }

    fn workspace_snapshot(&self, task_id: &str) -> Result<Option<WorkspaceSnapshot>, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        let row: Option<WorkspaceSnapshot> = conn
            .query_row(
                "SELECT w.id, w.kind, w.path, w.branch_name, w.config
                 FROM tasks t JOIN workspaces w ON w.id = t.workspace_id
                 WHERE t.id = ?1",
                [task_id],
                |row| {
                    let config: Option<String> = row.get(4)?;
                    Ok(WorkspaceSnapshot {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        path: row.get(2)?,
                        branch_name: row.get(3)?,
                        config: config.as_deref().and_then(|c| serde_json::from_str(c).ok()),
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Reference `teardownTask(taskId, 'terminate')` + `cleanupDetachedSessions`:
    /// cancel every tracked PTY session and kill the matching tmux sessions.
    /// Every failure is a warning — deletion must not be blocked by a wedged
    /// process.
    fn teardown_sessions(&self, project_id: &str, task_id: &str) {
        let mut leaf_ids: Vec<String> = match self.conversations.list_by_task(task_id) {
            Ok(convs) => convs.into_iter().map(|c| c.id).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "teardown: conversation list failed (non-fatal)");
                Vec::new()
            }
        };
        // Terminal leaves (reference getTaskSessionLeafIds includes terminals).
        // Best-effort: no panics, no errors — a terminal query failure just
        // narrows the kill set.
        if let Ok(conn) = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))
        {
            if let Ok(mut stmt) = conn.prepare("SELECT id FROM terminals WHERE task_id = ?1") {
                if let Ok(rows) = stmt.query_map([task_id], |row| row.get::<_, String>(0)) {
                    let ids: Vec<String> = rows.flatten().collect();
                    leaf_ids.extend(ids);
                }
            }
        }

        for leaf in leaf_ids {
            let session_id = make_pty_session_id(project_id, task_id, &leaf);
            self.sessions.terminate(&session_id, TEARDOWN_WAIT);
            // tmux durability path: the session may live in the tmux server
            // (survives an ade kill). Best-effort — absence is Ok.
            if let Err(e) = kill_tmux_session(&make_tmux_session_name(&session_id)) {
                tracing::warn!(session = %session_id, error = %e, "tmux kill failed (non-fatal)");
            }
        }
    }

    /// Reference `removeWorktreeIfUnused` + the fs fallback: sibling tasks
    /// block removal (returns `Ok(false)`); the guards inside
    /// `WorktreeManager::remove_worktree` refuse the project root and paths
    /// outside the pool. `force = true`: deletion is user-confirmed, so the
    /// dirty-check is bypassed (the confirmation dialog carries the warning).
    fn remove_worktree_if_unused(
        &self,
        project_id: &str,
        task_id: &str,
        ws: &WorkspaceSnapshot,
    ) -> Result<bool, Error> {
        let Some(path) = ws.path.as_ref().map(std::path::PathBuf::from) else {
            return Ok(false);
        };
        let Some(project) = self.projects.get(project_id)? else {
            tracing::warn!(project_id, "worktree cleanup skipped: project row missing");
            return Ok(false);
        };
        self.worktrees.remove_worktree(
            &project, task_id, &ws.id, &path,
            true, // user-confirmed delete: bypass the dirty-check
        )
    }

    /// Reference branch gate: delete only a branch the workspace CREATED
    /// (`create-branch` intent) and only when it differs from its source.
    /// Safe delete (`-d`); failures warn (reference catches).
    fn delete_provisioned_branch(&self, project_id: &str, ws: &WorkspaceSnapshot) {
        let provisioned = provisioned_branch(ws);
        let Some(branch) = provisioned else {
            return;
        };
        let from_branch = ws
            .config
            .as_ref()
            .filter(|c| {
                c.get("git")
                    .and_then(|g| g.get("kind"))
                    .and_then(Value::as_str)
                    == Some("create-branch")
            })
            .and_then(|c| c.pointer("/git/fromBranch/branch"))
            .and_then(Value::as_str);
        if from_branch == Some(branch.as_str()) {
            return; // the workspace merely used an existing branch
        }
        let Some(project) = self.projects.get(project_id).ok().flatten() else {
            return;
        };
        let git = self.worktrees.git();
        if let Err(e) = git.branch_delete(&project.path, &branch, false) {
            tracing::warn!(branch = %branch, error = %e, "branch deletion failed (non-fatal)");
        }
    }
}

/// Reference `getProvisionedWorkspaceBranch`: config-derived branch first,
/// then the recorded `branch_name` for worktree rows; project-root/byoi have
/// none.
fn provisioned_branch(ws: &WorkspaceSnapshot) -> Option<String> {
    match ws.kind.as_deref()? {
        "project-root" | "byoi" => None,
        _ => {
            if let Some(config) = &ws.config {
                let git = config.get("git")?;
                let kind = git.get("kind").and_then(Value::as_str)?;
                return match kind {
                    "use-branch" | "create-branch" => git
                        .get("branchName")
                        .and_then(Value::as_str)
                        .map(String::from),
                    _ => None, // "none" and unknown kinds provision no branch
                };
            }
            ws.branch_name.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioned_branch_follows_config_then_column() {
        // create-branch config wins over the column.
        let ws = WorkspaceSnapshot {
            id: "w".into(),
            kind: Some("worktree".into()),
            path: None,
            branch_name: Some("stale".into()),
            config: Some(serde_json::json!({
                "version": "2",
                "git": { "kind": "create-branch", "branchName": "ade/task",
                         "fromBranch": { "type": "local", "branch": "main" } },
                "workspace": { "kind": "new-worktree" }
            })),
        };
        assert_eq!(provisioned_branch(&ws).as_deref(), Some("ade/task"));

        // No config → the recorded column.
        let mut ws_col = ws.clone();
        ws_col.config = None;
        ws_col.branch_name = Some("ade/col".into());
        assert_eq!(provisioned_branch(&ws_col).as_deref(), Some("ade/col"));

        // project-root never has a provisioned branch.
        let mut ws_root = ws.clone();
        ws_root.kind = Some("project-root".into());
        assert_eq!(provisioned_branch(&ws_root), None);
    }
}
