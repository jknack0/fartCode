//! Project model (ARCHITECTURE.md §2, ticket E1-03).
//!
//! `Project` is the domain model; `ProjectDto` is the flat serializable
//! shape returned to the frontend (never DB rows — see ARCHITECTURE.md §4).

use std::path::PathBuf;

use rusqlite::Row;
use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceProviderKind {
    Local,
    Ssh,
}

impl WorkspaceProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkspaceProviderKind::Local => "local",
            WorkspaceProviderKind::Ssh => "ssh",
        }
    }

    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "local" => Ok(WorkspaceProviderKind::Local),
            "ssh" => Ok(WorkspaceProviderKind::Ssh),
            other => Err(Error::Internal(format!(
                "unknown workspace_provider: {other}"
            ))),
        }
    }
}

/// A registered project (mirrors the reference `shared/projects.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub workspace_provider: WorkspaceProviderKind,
    /// `Some("origin/main")` when resolved, else `None` (reads as "main").
    pub base_ref: Option<String>,
    pub ssh_connection_id: Option<String>,
    pub repository_workspace_id: Option<String>,
    /// Worktree pool directory segment (#81). NULL until the adoption pass or
    /// the first pool resolution stamps it.
    pub worktree_pool_segment: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl Project {
    /// Effective base ref; `None` reads as `"main"` (reference `getProjects`).
    pub fn base_ref(&self) -> &str {
        self.base_ref.as_deref().unwrap_or("main")
    }
}

/// Flat, serializable project shape for Tauri commands / the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub workspace_provider: String,
    pub base_ref: Option<String>,
    pub ssh_connection_id: Option<String>,
    pub repository_workspace_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl From<&Project> for ProjectDto {
    fn from(p: &Project) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            path: p.path.to_string_lossy().into_owned(),
            workspace_provider: p.workspace_provider.as_str().into(),
            base_ref: p.base_ref.clone(),
            ssh_connection_id: p.ssh_connection_id.clone(),
            repository_workspace_id: p.repository_workspace_id.clone(),
            created_at: p.created_at.clone(),
            updated_at: p.updated_at.clone(),
        }
    }
}

impl From<Project> for ProjectDto {
    fn from(p: Project) -> Self {
        ProjectDto::from(&p)
    }
}

/// Maps a `projects` row to a `Project` (columns must match the SELECT order
/// used by the store: id, name, path, workspace_provider, base_ref,
/// ssh_connection_id, repository_workspace_id, worktree_pool_segment,
/// created_at, updated_at).
pub(crate) fn project_from_row(row: &Row<'_>) -> rusqlite::Result<Project> {
    let workspace_provider = row.get::<_, String>(3)?;
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        path: PathBuf::from(row.get::<_, String>(2)?),
        workspace_provider: WorkspaceProviderKind::parse(&workspace_provider)
            .unwrap_or(WorkspaceProviderKind::Local),
        base_ref: row.get(4)?,
        ssh_connection_id: row.get(5)?,
        repository_workspace_id: row.get(6)?,
        worktree_pool_segment: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

pub(crate) const PROJECT_COLUMNS: &str =
    "id, name, path, workspace_provider, base_ref, ssh_connection_id, \
     repository_workspace_id, worktree_pool_segment, created_at, updated_at";
