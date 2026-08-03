//! Tasks domain: durable task rows with lifecycle operations and events
//! (ARCHITECTURE.md §2, ticket E2-01).

pub mod lifecycle;
pub mod model;

use std::sync::Arc;

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::events::{EventBus, InternalEvent};
use crate::Error;

pub use model::{LinkedIssue, Task, TaskDto, TaskStatus, TaskType};

/// Options for `TaskStore::create` (reference `CreateTaskParams`, Phase 0
/// subset — workspace config / task config versioned blobs arrive with E2-02
/// and E2-04).
#[derive(Debug, Clone)]
pub struct CreateTaskOptions {
    pub project_id: String,
    pub name: String,
    /// Override the generated id (renderer-optimistic flows). Default: uuid v4.
    pub id: Option<String>,
    /// Reference `taskConfig.initialStatus ?? 'in_progress'`.
    pub initial_status: Option<TaskStatus>,
    pub linked_issue: Option<LinkedIssue>,
    /// When set, an initial conversation row is created in the same
    /// transaction (reference `taskConfig.initialConversation`).
    pub initial_conversation: Option<InitialConversation>,
    /// When set, the task is `type='automation-run'` (E11 hooks in later).
    pub automation_run_id: Option<String>,
}

impl CreateTaskOptions {
    pub fn new(project_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            project_id: project_id.into(),
            name: name.into(),
            id: None,
            initial_status: None,
            linked_issue: None,
            initial_conversation: None,
            automation_run_id: None,
        }
    }

    pub fn with_initial_status(mut self, status: TaskStatus) -> Self {
        self.initial_status = Some(status);
        self
    }

    pub fn with_linked_issue(mut self, issue: LinkedIssue) -> Self {
        self.linked_issue = Some(issue);
        self
    }

    pub fn with_initial_conversation(
        mut self,
        title: impl Into<String>,
        provider: Option<&str>,
    ) -> Self {
        self.initial_conversation = Some(InitialConversation {
            title: title.into(),
            provider: provider.map(String::from),
            config: None,
        });
        self
    }

    pub fn with_automation_run(mut self, run_id: impl Into<String>) -> Self {
        self.automation_run_id = Some(run_id.into());
        self
    }
}

/// An initial conversation created atomically with the task.
#[derive(Debug, Clone)]
pub struct InitialConversation {
    pub title: String,
    pub provider: Option<String>,
    /// Versioned conversation config (raw JSON); the conversations module
    /// (E2-05) owns the versioned schema.
    pub config: Option<serde_json::Value>,
}

/// Task store surface used by the Tauri layer (ARCHITECTURE.md §7).
pub trait TaskStore: Send + Sync {
    /// Creates a task + workspace row (+ initial conversation) atomically.
    /// Emits `task:created` (+ `conversation:created`) after commit.
    fn create(&self, options: CreateTaskOptions) -> Result<Task, Error>;

    fn get(&self, id: &str) -> Result<Option<Task>, Error>;
    fn list_by_project(&self, project_id: &str) -> Result<Vec<Task>, Error>;

    /// Status change: any lifecycle status is accepted (no allowlist —
    /// reference `updateTaskStatus`); same-status is a no-op. Updates
    /// `status_changed_at`.
    fn update_status(&self, id: &str, status: TaskStatus) -> Result<Task, Error>;

    fn set_pinned(&self, id: &str, pinned: bool) -> Result<Task, Error>;

    /// Archive keeps the row (non-destructive; the reference reaps the
    /// session in 'archive' mode — E2-05). Restore clears `archived_at`.
    fn archive(&self, id: &str) -> Result<Task, Error>;
    fn restore(&self, id: &str) -> Result<Task, Error>;

    /// Hard delete; conversations/terminals cascade via FK.
    fn delete(&self, id: &str) -> Result<(), Error>;

    /// Idempotent provision fast-path: re-provision re-fires
    /// `task:provisioned` and touches `last_interacted_at` (reference
    /// `provisionWorkspace` fast path). Real workspace bootstrap is E2-02.
    fn provision(&self, id: &str) -> Result<(), Error>;
}

/// SQLite-backed task store (ARCHITECTURE.md §7 wiring:
/// `DbTaskStore::new(db, event_bus)`).
pub struct DbTaskStore {
    db: Arc<dyn Db>,
    event_bus: Arc<dyn EventBus>,
}

impl DbTaskStore {
    pub fn new(db: Arc<dyn Db>, event_bus: Arc<dyn EventBus>) -> Self {
        Self { db, event_bus }
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        f(&conn)
    }
}

impl TaskStore for DbTaskStore {
    fn create(&self, options: CreateTaskOptions) -> Result<Task, Error> {
        let CreateTaskOptions {
            project_id,
            name,
            id: requested_id,
            initial_status,
            linked_issue,
            initial_conversation,
            automation_run_id,
        } = options;

        // Validate the project exists (reference: project-not-found).
        let project_exists: bool = self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT 1 FROM projects WHERE id = ?1",
                    [&project_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        })?;
        if !project_exists {
            return Err(Error::ProjectNotFound(project_id));
        }

        let id = requested_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let status = initial_status.unwrap_or(TaskStatus::InProgress);
        let workspace_id = uuid::Uuid::new_v4().to_string();
        let task_type = if automation_run_id.is_some() {
            TaskType::AutomationRun
        } else {
            TaskType::Task
        };
        let linked_issue_json = match &linked_issue {
            Some(li) => Some(crate::db::versioned_json::serialize_versioned(li)?),
            None => None,
        };
        let conv_id = initial_conversation
            .as_ref()
            .map(|_| uuid::Uuid::new_v4().to_string());

        // Atomic boundary: task + workspace + initial conversation (reference
        // commitCreateTask inside a single db.transaction). The guard + tx are
        // scoped so they drop before the re-fetch below (mutex is not
        // reentrant).
        {
            let mut conn = self
                .db
                .conn()
                .lock()
                .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO tasks
                     (id, project_id, name, status, linked_issue, workspace_id, type,
                      automation_run_id, created_by, status_changed_at, last_interacted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'user', datetime('now'), datetime('now'))",
                rusqlite::params![
                    id,
                    project_id,
                    name,
                    status.as_str(),
                    linked_issue_json,
                    workspace_id,
                    task_type.as_str(),
                    automation_run_id,
                ],
            )?;
            tx.execute(
                "INSERT INTO workspaces (id, type, kind, location) VALUES (?1, 'local', 'worktree', 'local')",
                [&workspace_id],
            )?;
            if let (Some(conv), Some(conv_id)) = (&initial_conversation, &conv_id) {
                tx.execute(
                    "INSERT INTO conversations
                         (id, project_id, task_id, title, provider, config,
                          is_initial_conversation, last_interacted_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, datetime('now'))",
                    rusqlite::params![
                        conv_id,
                        project_id,
                        id,
                        conv.title,
                        conv.provider,
                        conv.config.as_ref().map(|c| c.to_string()),
                    ],
                )?;
            }
            tx.commit()?;
        }

        // Post-commit events, non-fatal (reference finalizeCreateTask).
        self.event_bus.send(InternalEvent::TaskCreated {
            id: id.clone(),
            project_id: project_id.clone(),
            name: name.clone(),
        });
        if let (Some(conv_id), Some(conv)) = (&conv_id, &initial_conversation) {
            self.event_bus.send(InternalEvent::ConversationCreated {
                id: conv_id.clone(),
                task_id: id.clone(),
                provider: conv.provider.clone().unwrap_or_default(),
            });
        }
        lifecycle::telemetry(
            "task_created",
            &[("task_id", id.clone()), ("project_id", project_id)],
        );

        self.get(&id)?
            .ok_or_else(|| Error::Internal("created task vanished".into()))
    }

    fn get(&self, id: &str) -> Result<Option<Task>, Error> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    &format!("SELECT {} FROM tasks WHERE id = ?1", model::TASK_COLUMNS),
                    [id],
                    model::task_from_row,
                )
                .optional()?)
        })
    }

    fn list_by_project(&self, project_id: &str) -> Result<Vec<Task>, Error> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM tasks WHERE project_id = ?1 ORDER BY updated_at DESC",
                model::TASK_COLUMNS
            ))?;
            let rows = stmt.query_map([project_id], model::task_from_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    fn update_status(&self, id: &str, status: TaskStatus) -> Result<Task, Error> {
        let task = self
            .get(id)?
            .ok_or_else(|| Error::TaskNotFound(id.into()))?;
        let old = task.status;
        let changed = self.with_conn(|conn| {
            lifecycle::apply_status_change(conn, id, old.as_str(), status.as_str())
        })?;
        if changed {
            self.event_bus.send(InternalEvent::TaskStatusChanged {
                id: id.into(),
                old_status: old.as_str().into(),
                new_status: status.as_str().into(),
            });
            lifecycle::telemetry(
                "task_status_changed",
                &[
                    ("task_id", id.into()),
                    ("from", old.as_str().into()),
                    ("to", status.as_str().into()),
                ],
            );
        }
        self.get(id)?.ok_or_else(|| Error::TaskNotFound(id.into()))
    }

    fn set_pinned(&self, id: &str, pinned: bool) -> Result<Task, Error> {
        self.get(id)?
            .ok_or_else(|| Error::TaskNotFound(id.into()))?;
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE tasks SET is_pinned = ?1, updated_at = datetime('now') WHERE id = ?2",
                rusqlite::params![pinned as i64, id],
            )?;
            Ok(())
        })?;
        self.get(id)?.ok_or_else(|| Error::TaskNotFound(id.into()))
    }

    fn archive(&self, id: &str) -> Result<Task, Error> {
        self.get(id)?
            .ok_or_else(|| Error::TaskNotFound(id.into()))?;
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE tasks SET archived_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1",
                [id],
            )?;
            Ok(())
        })?;
        self.event_bus
            .send(InternalEvent::TaskArchived { id: id.into() });
        lifecycle::telemetry("task_archived", &[("task_id", id.into())]);
        self.get(id)?.ok_or_else(|| Error::TaskNotFound(id.into()))
    }

    fn restore(&self, id: &str) -> Result<Task, Error> {
        self.get(id)?
            .ok_or_else(|| Error::TaskNotFound(id.into()))?;
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE tasks SET archived_at = NULL, updated_at = datetime('now') WHERE id = ?1",
                [id],
            )?;
            Ok(())
        })?;
        self.get(id)?.ok_or_else(|| Error::TaskNotFound(id.into()))
    }

    fn delete(&self, id: &str) -> Result<(), Error> {
        // Hard delete; conversations/terminals cascade via FK. (Session
        // teardown + workspace/worktree cleanup arrive with E2-05/E2-02.)
        self.with_conn(|conn| {
            conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
            Ok(())
        })?;
        self.event_bus
            .send(InternalEvent::TaskDeleted { id: id.into() });
        lifecycle::telemetry("task_deleted", &[("task_id", id.into())]);
        Ok(())
    }

    fn provision(&self, id: &str) -> Result<(), Error> {
        let task = self
            .get(id)?
            .ok_or_else(|| Error::TaskNotFound(id.into()))?;
        // Idempotent fast path: re-provision just re-fires the ready event
        // (reference provisionWorkspace fast path). Workspace bootstrap is
        // E2-02; Phase 0 touches recency and emits.
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE tasks SET last_interacted_at = datetime('now') WHERE id = ?1",
                [id],
            )?;
            Ok(())
        })?;
        self.event_bus.send(InternalEvent::TaskProvisioned {
            id: id.into(),
            workspace_id: task.workspace_id.clone().unwrap_or_default(),
        });
        lifecycle::telemetry("task_provisioned", &[("task_id", id.into())]);
        Ok(())
    }
}
