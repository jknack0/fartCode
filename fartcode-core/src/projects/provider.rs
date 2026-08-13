//! Project provider lifecycle + repository setup helpers (ARCHITECTURE.md §2,
//! ticket E1-03).
//!
//! Phase 0 scope: opening a project seeds its settings, excludes `.fartCode/`
//! from git, re-detects worktrees (close/open restores state), and ensures
//! the repository workspace row. Session/workspace/preview-server teardown
//! arrives with their tickets (E2-05, E2-02, E13) — `close_project` is a
//! stub now, and the tmux detach-vs-terminate decision is documented there.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::git::{GitOps, WorktreeEntry};
use crate::settings::SettingsStore;
use crate::Error;

/// fartCode's per-project state directory — excluded from git on project open.
pub const FARTCODE_STATE_DIR: &str = ".fartCode";

/// Adds `.fartCode/` to the repo's `.git/info/exclude` (local, per-repo — never a
/// tracked `.gitignore`), matching the reference's
/// `ensureEmdashGitExcludedSafe`. In a linked worktree `--git-dir` resolves to
/// the per-worktree dir, so the entry lands in that worktree's exclude file —
/// acceptable (the reference writes to the common dir; E2-02 can align this).
/// Skips when the git dir is a file (exotic layouts). Idempotent.
pub fn ensure_fartcode_git_excluded(git: &dyn GitOps, repo_path: &Path) -> Result<(), Error> {
    let git_dir = git.git_dir(repo_path)?;
    if !git_dir.is_dir() {
        return Ok(());
    }
    let exclude_path = git_dir.join("info").join("exclude");
    let pattern = format!("{FARTCODE_STATE_DIR}/");
    let existing = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == pattern) {
        return Ok(());
    }
    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&pattern);
    content.push('\n');
    std::fs::create_dir_all(git_dir.join("info"))?;
    std::fs::write(&exclude_path, content)?;
    Ok(())
}

/// Key for a project's repository workspace: `sha256("local:<path>")` (or
/// `"ssh:<conn_id>:<path>"` once SSH projects land), reference
/// `computeWorkspaceKey`.
pub fn repository_workspace_key(project: &crate::projects::model::Project) -> String {
    let mut hasher = Sha256::new();
    match project.ssh_connection_id.as_deref() {
        Some(conn) => hasher.update(format!("ssh:{conn}:{}", project.path.display()).as_bytes()),
        None => hasher.update(format!("local:{}", project.path.display()).as_bytes()),
    }
    format!("{:x}", hasher.finalize())
}

/// Ensures the `project-root` workspace row exists (race-safe, reused by key)
/// and points `projects.repository_workspace_id` at it. Returns the
/// workspace id. Non-fatal in the create flow (callers warn and continue).
pub fn ensure_repository_workspace(
    db: &dyn Db,
    project: &crate::projects::model::Project,
) -> Result<Option<String>, Error> {
    if let Some(existing) = &project.repository_workspace_id {
        return Ok(Some(existing.clone()));
    }
    let key = repository_workspace_key(project);
    let mut conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    let tx = conn.transaction()?;
    let existing = crate::workspaces::id_by_key(&tx, &key)?;
    let workspace_id = match existing {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            // A remote project's repository workspace lives on the host: the
            // row records that (and the connection), or a later rehydrate has
            // no way back to the machine the files are on (E12-06).
            let path = project.path.to_string_lossy();
            let ssh = project.ssh_connection_id.as_deref();
            crate::workspaces::insert_row(
                &tx,
                &crate::workspaces::NewWorkspace {
                    id: &id,
                    key: Some(&key),
                    r#type: if ssh.is_some() {
                        "project-ssh"
                    } else {
                        "local"
                    },
                    kind: "project-root",
                    location: if ssh.is_some() { "remote" } else { "local" },
                    path: Some(&path),
                    ssh_connection_id: ssh,
                    config: None,
                },
            )?;
            id
        }
    };
    tx.execute(
        "UPDATE projects SET repository_workspace_id = ?1 WHERE id = ?2",
        rusqlite::params![workspace_id, project.id],
    )?;
    tx.commit()?;
    Ok(Some(workspace_id))
}

/// Slugifies `name` into a safe path segment (reference `safePathSegment`):
/// takes the LAST path segment (split on `/` or `\`, ignoring empties),
/// replaces path-hostile characters with `-`, trims, and falls back to
/// `fallback` for empty/dot/Windows-reserved names.
pub fn safe_path_segment(name: &str, fallback: &str) -> String {
    let last = name
        .split(['/', '\\'])
        .rfind(|s| !s.is_empty())
        .unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| {
            if matches!(
                c,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\u{0}'..='\u{1f}'
            ) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches(['.', ' ']).to_string();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." || is_windows_reserved(&trimmed) {
        fallback.to_string()
    } else {
        trimmed
    }
}

fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        stem.as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

/// New-scheme pool segment (#81): `<safe_path_segment(name,id)>-<hash8>`,
/// hash8 = first 8 hex chars of sha256 over the project's STORED path
/// (canonicalized at create time — see ADR-0039). Human-navigable and unique
/// per project, unlike the legacy name-only segment.
pub fn new_pool_segment(project: &crate::projects::model::Project) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project.path.to_string_lossy().as_bytes());
    let hash8 = &format!("{:x}", hasher.finalize())[..8];
    format!("{}-{hash8}", safe_path_segment(&project.name, &project.id))
}

/// Persists `segment` on the project row when unset (lazy stamping — never
/// overwrites a segment the adoption pass chose, e.g. an adopted legacy one).
fn stamp_pool_segment(db: &dyn Db, project_id: &str, segment: &str) -> Result<(), Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    conn.execute(
        "UPDATE projects SET worktree_pool_segment = ?1, updated_at = datetime('now')
         WHERE id = ?2 AND worktree_pool_segment IS NULL",
        rusqlite::params![segment, project_id],
    )?;
    Ok(())
}

/// Local worktree pool path (#81): `join(root, segment)` where
/// - root = the project's `worktree_directory` override (already normalized +
///   invalid-fallback handled by `get_project_settings`) when set, else the
///   app-level `default_worktree_directory`;
/// - segment = the project's stored `worktree_pool_segment`, stamped on first
///   resolve with the deterministic `new_pool_segment` scheme when missing.
///
/// SSH pools land in Phase 3.
pub fn worktree_pool_path(
    db: &dyn Db,
    settings: &dyn SettingsStore,
    project: &crate::projects::model::Project,
) -> Result<PathBuf, Error> {
    let root = match settings
        .get_project_settings(&project.id, &project.path)
        .ok()
        .and_then(|ps| ps.worktree_directory)
    {
        Some(dir) => PathBuf::from(dir),
        None => {
            let json = settings.get_json("localProject")?;
            let local: crate::settings::LocalProjectGroup =
                serde_json::from_value(json).map_err(|e| Error::InvalidSettingValue {
                    key: "localProject".into(),
                    reason: e.to_string(),
                })?;
            PathBuf::from(local.default_worktree_directory)
        }
    };
    let segment = match &project.worktree_pool_segment {
        Some(segment) => segment.clone(),
        None => {
            let segment = new_pool_segment(project);
            stamp_pool_segment(db, &project.id, &segment)?;
            segment
        }
    };
    Ok(root.join(segment))
}

/// Result of opening a project: the project itself plus re-detected state.
#[derive(Debug, Clone)]
pub struct OpenProjectResult {
    pub project: crate::projects::model::Project,
    /// Worktrees re-detected via `git worktree list` (close/open restore).
    pub worktrees: Vec<WorktreeEntry>,
    pub repository_workspace_id: Option<String>,
}

/// Opens a project (Phase 0): seeds settings, excludes `.fartCode/` from git,
/// re-detects worktrees, ensures the repository workspace.
pub fn open_project(
    db: &dyn Db,
    settings: &dyn SettingsStore,
    git: &dyn GitOps,
    project: &crate::projects::model::Project,
) -> Result<OpenProjectResult, Error> {
    settings.seed_project_settings(&project.id, &project.path)?;
    ensure_fartcode_git_excluded(git, &project.path)?;
    let worktrees = git.worktree_list(&project.path)?;
    let repository_workspace_id = ensure_repository_workspace(db, project)
        .map_err(|e| {
            tracing::warn!(project_id = %project.id, error = %e, "ensureRepositoryWorkspace failed");
            e
        })
        .ok()
        .flatten();
    Ok(OpenProjectResult {
        project: project.clone(),
        worktrees,
        repository_workspace_id,
    })
}

/// Closes a project (Phase 0 stub).
///
/// The reference tears down sessions, workspaces, and preview servers with a
/// teardown mode of `tmux ? 'detach' : 'terminate'` (project-manager.ts:
/// `dispose`). Those subsystems arrive with E2-05 (sessions), E2-02
/// (workspaces), and E13 (preview); this is where that teardown will hook in.
pub fn close_project(
    _db: &dyn Db,
    _settings: &dyn SettingsStore,
    _git: &dyn GitOps,
    _project: &crate::projects::model::Project,
) -> Result<(), Error> {
    Ok(())
}

/// Stub for GitHub "new repo" creation (ticket E1-03: "stub the GitHub call
/// behind a trait in Phase 0"). E8 wires a real implementation.
pub trait RepoHostProvider: Send + Sync {
    /// Creates a repository on the host and returns its clone URL.
    fn create_repository(&self, name: &str, description: &str) -> Result<String, Error>;
}

pub struct StubRepoHost;

impl RepoHostProvider for StubRepoHost {
    fn create_repository(&self, _name: &str, _description: &str) -> Result<String, Error> {
        Err(Error::Internal(
            "GitHub repository creation arrives with E8 — stubbed in Phase 0".into(),
        ))
    }
}
