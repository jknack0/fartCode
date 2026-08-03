//! Projects domain: create (local/clone), open/close lifecycle, CRUD
//! (ARCHITECTURE.md §2, ticket E1-03).

pub mod model;
pub mod provider;
pub mod worktrees;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::events::{EventBus, InternalEvent};
use crate::git::GitOps;
use crate::settings::SettingsStore;
use crate::Error;

pub use model::{Project, ProjectDto, WorkspaceProviderKind};
pub use provider::{close_project, open_project, OpenProjectResult};

/// Project store surface used by the Tauri layer (ARCHITECTURE.md §7).
pub trait ProjectStore: Send + Sync {
    /// Adds a local directory as a project. When `init_if_missing` is true a
    /// missing git repo is initialized. Duplicate paths open the existing
    /// project instead of creating a new row.
    fn create_local(&self, path: &Path, init_if_missing: bool) -> Result<Project, Error>;

    /// Clones `url` into the configured projects directory, then runs the
    /// same open flow.
    fn create_clone(&self, url: &str) -> Result<Project, Error>;

    fn get(&self, id: &str) -> Result<Option<Project>, Error>;
    fn get_by_path(&self, path: &Path) -> Result<Option<Project>, Error>;
    fn list(&self) -> Result<Vec<Project>, Error>;

    /// Deletes a project row (only user-initiated deletes remove projects;
    /// FK cascade cleans up dependent rows).
    fn delete(&self, id: &str) -> Result<(), Error>;

    /// Opens a project: seeds settings, excludes `.ade/`, re-detects
    /// worktrees.
    fn open(&self, id: &str) -> Result<OpenProjectResult, Error>;

    /// Closes a project (Phase 0 stub — see provider::close_project).
    fn close(&self, id: &str) -> Result<(), Error>;
}

/// SQLite-backed project store. Wires the db, settings, git, and event bus
/// exactly as ARCHITECTURE.md §7 sketches.
pub struct DbProjectStore {
    db: Arc<dyn Db>,
    settings: Arc<dyn SettingsStore>,
    git: Arc<dyn GitOps>,
    event_bus: Arc<dyn EventBus>,
}

impl DbProjectStore {
    pub fn new(
        db: Arc<dyn Db>,
        settings: Arc<dyn SettingsStore>,
        git: Arc<dyn GitOps>,
        event_bus: Arc<dyn EventBus>,
    ) -> Self {
        Self {
            db,
            settings,
            git,
            event_bus,
        }
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

    // -- create flows --------------------------------------------------------

    /// Common tail of the local + clone flows: insert the row, seed settings,
    /// open (`.ade/` exclusion + worktree re-detection), ensure the
    /// repository workspace (non-fatal), emit `project:added`.
    fn finish_create(
        &self,
        repo_root: &Path,
        provider_kind: WorkspaceProviderKind,
        base_ref: Option<String>,
        ssh_connection_id: Option<String>,
    ) -> Result<Project, Error> {
        let project = self.insert_row(repo_root, provider_kind, base_ref, ssh_connection_id)?;
        self.open_result(&project)?;
        if let Err(e) = provider::ensure_repository_workspace(self.db.as_ref(), &project) {
            tracing::warn!(project_id = %project.id, error = %e, "ensureRepositoryWorkspace failed (non-fatal)");
        }
        // Re-fetch so the returned project reflects repository_workspace_id.
        let project = self
            .get(&project.id)?
            .ok_or_else(|| Error::Internal("inserted project vanished".into()))?;
        self.event_bus.send(InternalEvent::ProjectAdded {
            id: project.id.clone(),
            name: project.name.clone(),
            path: project.path.to_string_lossy().into_owned(),
        });
        Ok(project)
    }

    fn insert_row(
        &self,
        repo_root: &Path,
        provider_kind: WorkspaceProviderKind,
        base_ref: Option<String>,
        ssh_connection_id: Option<String>,
    ) -> Result<Project, Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let name = repo_root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.clone());
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO projects
                     (id, name, path, workspace_provider, base_ref, ssh_connection_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    id,
                    name,
                    repo_root.to_string_lossy(),
                    provider_kind.as_str(),
                    base_ref,
                    ssh_connection_id
                ],
            )?;
            Ok(())
        })?;
        self.get(&id)?
            .ok_or_else(|| Error::Internal("inserted project vanished".into()))
    }

    /// Resolves the base ref: `computeBaseRef` + refinement via remote HEAD /
    /// refs (reference `create-project-utils.ts` + `getDefaultBranch`).
    fn resolve_base_ref(&self, repo: &Path) -> Result<String, Error> {
        let remotes = self.git.remotes(repo)?;
        // Reference computeBaseRef: remoteName must be a plain name (no '://').
        let remote_name = remotes
            .first()
            .map(String::as_str)
            .filter(|r| {
                !r.contains("://")
                    && r.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            })
            .map(|r| r.to_string());
        let current = self.git.current_branch(repo)?;

        // Port of computeBaseRef(undefined, remote, branch) -> normalize(branch):
        // slash-containing branches stay bare (`feature/x`), plain branches get
        // the remote prefix (`origin/main`), '://' refs are dropped.
        let normalize = |value: Option<&str>| -> Option<String> {
            let value = value?.trim();
            if value.is_empty() || value.contains("://") {
                return None;
            }
            if let Some(slash) = value.find('/') {
                let head = &value[..slash];
                let branch_part = value[slash + 1..].trim_start_matches('/');
                if !head.is_empty() && !branch_part.is_empty() {
                    return Some(format!("{head}/{branch_part}"));
                }
                if head.is_empty() && !branch_part.is_empty() {
                    return remote_name.as_ref().map(|r| format!("{r}/{branch_part}"));
                }
                return None;
            }
            let suffix = value.trim_start_matches('/');
            match &remote_name {
                Some(r) => Some(format!("{r}/{suffix}")),
                None => Some(suffix.to_string()),
            }
        };
        let default_ref = match &remote_name {
            Some(r) => format!("{r}/main"),
            None => "main".to_string(),
        };
        let detected = normalize(current.as_deref()).unwrap_or(default_ref);

        // Refinement (resolveProjectBaseRef): the remote is derived from the
        // DETECTED ref, and the default branch wins only if the remote
        // actually tracks it.
        let Some(ref_remote) = remote_name_from_qualified_ref(&detected) else {
            return Ok(detected);
        };
        let default = self
            .git
            .remote_head(repo, &ref_remote)?
            .or_else(|| {
                ["main", "master", "develop", "trunk"]
                    .iter()
                    .find_map(|candidate| {
                        self.git
                            .verify_ref(repo, &format!("refs/heads/{candidate}"))
                            .ok()
                            .filter(|exists| *exists)
                            .map(|_| candidate.to_string())
                    })
            })
            .unwrap_or_else(|| "main".to_string());
        let default_name = bare_ref_name(&default);
        let qualified = format!("{ref_remote}/{default_name}");
        let branches = self.git.branches(repo)?;
        if branches.iter().any(|b| b.is_remote && b.name == qualified) {
            Ok(qualified)
        } else {
            Ok(detected)
        }
    }

    /// `open_project` with the store's dependencies (used by create + trait).
    fn open_result(&self, project: &Project) -> Result<OpenProjectResult, Error> {
        open_project(
            self.db.as_ref(),
            self.settings.as_ref(),
            self.git.as_ref(),
            project,
        )
    }

    // -- CRUD ---------------------------------------------------------------

    fn get(&self, id: &str) -> Result<Option<Project>, Error> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    &format!(
                        "SELECT {} FROM projects WHERE id = ?1",
                        model::PROJECT_COLUMNS
                    ),
                    [id],
                    model::project_from_row,
                )
                .optional()?)
        })
    }

    fn get_by_path(&self, path: &Path) -> Result<Option<Project>, Error> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    &format!(
                        "SELECT {} FROM projects WHERE path = ?1",
                        model::PROJECT_COLUMNS
                    ),
                    [path.to_string_lossy()],
                    model::project_from_row,
                )
                .optional()?)
        })
    }

    fn list(&self) -> Result<Vec<Project>, Error> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM projects ORDER BY created_at",
                model::PROJECT_COLUMNS
            ))?;
            let rows = stmt.query_map([], model::project_from_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    fn delete(&self, id: &str) -> Result<(), Error> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
            Ok(())
        })?;
        self.event_bus
            .send(InternalEvent::ProjectDeleted { id: id.into() });
        Ok(())
    }
}

impl ProjectStore for DbProjectStore {
    fn create_local(&self, path: &Path, init_if_missing: bool) -> Result<Project, Error> {
        // Validate the directory (realpath so duplicate detection is exact).
        let canonical = std::fs::canonicalize(path)
            .map_err(|_| Error::ProjectPathNotFound(path.to_path_buf()))?;
        if !canonical.is_dir() {
            return Err(Error::ProjectPathNotFound(canonical));
        }

        // Duplicate path → open existing (reference createProject dispatch).
        if let Some(existing) = self.get_by_path(&canonical)? {
            return Ok(existing);
        }

        // Ensure a git repository (init when asked to).
        if !self.git.is_git_repo(&canonical)? {
            if !init_if_missing {
                return Err(Error::Internal(format!(
                    "not a git repository: {}",
                    canonical.display()
                )));
            }
            self.git.init(&canonical)?;
        }
        let repo_root = self.git.show_toplevel(&canonical)?;

        let base_ref = self.resolve_base_ref(&repo_root)?;
        self.finish_create(
            &repo_root,
            WorkspaceProviderKind::Local,
            Some(base_ref),
            None,
        )
    }

    fn create_clone(&self, url: &str) -> Result<Project, Error> {
        let local = self.settings.get_json("localProject").and_then(|json| {
            serde_json::from_value::<crate::settings::LocalProjectGroup>(json).map_err(|e| {
                Error::InvalidSettingValue {
                    key: "localProject".into(),
                    reason: e.to_string(),
                }
            })
        })?;
        let segment = repo_name_from_url(url);
        let target = PathBuf::from(local.default_projects_directory).join(segment);

        // Duplicate by clone target path → open existing.
        if let Some(existing) = self.get_by_path(&target)? {
            return Ok(existing);
        }
        if target.exists() {
            return Err(Error::Internal(format!(
                "clone target already exists: {}",
                target.display()
            )));
        }
        // The configured projects dir may not exist on a fresh install
        // (reference cloneProjectRepository: ensureAbsoluteDir(parent)).
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }

        self.git.as_ref().clone(url, &target)?;
        let repo_root = target;
        let base_ref = self.resolve_base_ref(&repo_root)?;
        self.finish_create(
            &repo_root,
            WorkspaceProviderKind::Local,
            Some(base_ref),
            None,
        )
    }

    fn get(&self, id: &str) -> Result<Option<Project>, Error> {
        self.get(id)
    }

    fn get_by_path(&self, path: &Path) -> Result<Option<Project>, Error> {
        self.get_by_path(path)
    }

    fn list(&self) -> Result<Vec<Project>, Error> {
        self.list()
    }

    fn delete(&self, id: &str) -> Result<(), Error> {
        self.delete(id)
    }

    fn open(&self, id: &str) -> Result<OpenProjectResult, Error> {
        let project = self
            .get(id)?
            .ok_or_else(|| Error::ProjectNotFound(id.into()))?;
        self.open_result(&project)
    }

    fn close(&self, id: &str) -> Result<(), Error> {
        let project = self
            .get(id)?
            .ok_or_else(|| Error::ProjectNotFound(id.into()))?;
        close_project(
            self.db.as_ref(),
            self.settings.as_ref(),
            self.git.as_ref(),
            &project,
        )
    }
}

/// Derives a clone directory name from a git URL: the last path segment,
/// minus a trailing `.git` (slugified for filesystem safety).
pub fn repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let last = trimmed
        .rsplit(['/', ':'])
        .find(|s| !s.is_empty())
        .unwrap_or("project");
    let name = last.strip_suffix(".git").unwrap_or(last);
    provider::safe_path_segment(name, "project")
}

/// Reference `remoteNameFromQualifiedRef`: the head of a `remote/branch` ref
/// (`origin/main` -> `origin`); `None` for bare refs.
pub fn remote_name_from_qualified_ref(ref_name: &str) -> Option<String> {
    let trimmed = ref_name.trim();
    let slash = trimmed.find('/')?;
    if slash == 0 {
        return None;
    }
    Some(trimmed[..slash].to_string())
}

/// Reference `bareRefName`: everything after the first slash (`origin/main` ->
/// `main`; `main` -> `main`).
pub fn bare_ref_name(ref_name: &str) -> String {
    match ref_name.find('/') {
        Some(slash) => ref_name[slash + 1..].to_string(),
        None => ref_name.to_string(),
    }
}
