//! Task model (ARCHITECTURE.md §2, ticket E2-01).
//!
//! Mirrors the reference `shared/core/tasks/tasks.ts`: the lifecycle status
//! enum, the `Task` row type, and the flat `TaskDto` for the frontend.
//! DB column names are used verbatim (including the `type` column, hence
//! `r#type`).

use rusqlite::Row;
use serde::{Deserialize, Serialize};

use crate::db::versioned_json::Versioned;
use crate::Error;

/// Task lifecycle status (reference `taskLifecycleStatuses`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Todo,
    InProgress,
    Review,
    Done,
    Cancelled,
    Backlog,
    Duplicate,
    Triage,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Todo => "todo",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Review => "review",
            TaskStatus::Done => "done",
            TaskStatus::Cancelled => "cancelled",
            TaskStatus::Backlog => "backlog",
            TaskStatus::Duplicate => "duplicate",
            TaskStatus::Triage => "triage",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "todo" => Ok(TaskStatus::Todo),
            "in_progress" => Ok(TaskStatus::InProgress),
            "review" => Ok(TaskStatus::Review),
            "done" => Ok(TaskStatus::Done),
            "cancelled" => Ok(TaskStatus::Cancelled),
            "backlog" => Ok(TaskStatus::Backlog),
            "duplicate" => Ok(TaskStatus::Duplicate),
            "triage" => Ok(TaskStatus::Triage),
            other => Err(Error::Internal(format!("unknown task status: {other}"))),
        }
    }
}

/// `tasks.type`: `'task'` (default) or `'automation-run'` (created by an
/// automation run — E11 hooks in later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskType {
    #[serde(rename = "task")]
    Task,
    #[serde(rename = "automation-run")]
    AutomationRun,
}

impl TaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskType::Task => "task",
            TaskType::AutomationRun => "automation-run",
        }
    }
}

/// A linked issue, stored in `tasks.linked_issue` as versioned JSON
/// (reference `shared/core/linked-issue.ts` — the port wraps it in the
/// `{version, data}` envelope).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedIssue {
    pub provider: String,
    pub url: String,
    pub title: String,
    pub identifier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

impl Versioned for LinkedIssue {
    const VERSION: u32 = 1;
}

/// A task row (columns mirror `tasks` verbatim).
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub status: TaskStatus,
    pub linked_issue: Option<LinkedIssue>,
    pub archived_at: Option<String>,
    pub is_pinned: bool,
    pub last_interacted_at: Option<String>,
    pub status_changed_at: Option<String>,
    pub workspace_id: Option<String>,
    /// Legacy column — never written by the current flow (reference
    /// `resolve-workspace-intent.ts` reads it only as a fallback).
    pub workspace_intent: Option<String>,
    pub created_by: String,
    pub r#type: TaskType,
    pub automation_run_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Flat, serializable task shape for Tauri commands / the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDto {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub status: TaskStatus,
    pub linked_issue: Option<LinkedIssue>,
    pub archived_at: Option<String>,
    pub is_pinned: bool,
    pub last_interacted_at: Option<String>,
    pub status_changed_at: Option<String>,
    pub workspace_id: Option<String>,
    pub created_by: String,
    pub r#type: TaskType,
    pub automation_run_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl From<&Task> for TaskDto {
    fn from(t: &Task) -> Self {
        Self {
            id: t.id.clone(),
            project_id: t.project_id.clone(),
            name: t.name.clone(),
            status: t.status,
            linked_issue: t.linked_issue.clone(),
            archived_at: t.archived_at.clone(),
            is_pinned: t.is_pinned,
            last_interacted_at: t.last_interacted_at.clone(),
            status_changed_at: t.status_changed_at.clone(),
            workspace_id: t.workspace_id.clone(),
            created_by: t.created_by.clone(),
            r#type: t.r#type,
            automation_run_id: t.automation_run_id.clone(),
            created_at: t.created_at.clone(),
            updated_at: t.updated_at.clone(),
        }
    }
}

/// Maps a `tasks` row to a `Task` (SELECT order must match `TASK_COLUMNS`).
pub(crate) fn task_from_row(row: &Row<'_>) -> rusqlite::Result<Task> {
    let status = row.get::<_, String>(3)?;
    let r#type = row.get::<_, String>(12)?;
    Ok(Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        status: TaskStatus::parse(&status).unwrap_or(TaskStatus::InProgress),
        linked_issue: row.get::<_, Option<String>>(4)?.and_then(|raw| {
            crate::db::versioned_json::parse_versioned::<LinkedIssue>(
                "tasks.linked_issue",
                Some(&raw),
            )
        }),
        archived_at: row.get(5)?,
        is_pinned: row.get::<_, i64>(6)? != 0,
        last_interacted_at: row.get(7)?,
        status_changed_at: row.get(8)?,
        workspace_id: row.get(9)?,
        workspace_intent: row.get(10)?,
        created_by: row.get(11)?,
        r#type: match r#type.as_str() {
            "automation-run" => TaskType::AutomationRun,
            _ => TaskType::Task,
        },
        automation_run_id: row.get::<_, Option<String>>(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

pub(crate) const TASK_COLUMNS: &str = "id, project_id, name, status, linked_issue, archived_at, \
     is_pinned, last_interacted_at, status_changed_at, workspace_id, workspace_intent, \
     created_by, type, automation_run_id, created_at, updated_at";
