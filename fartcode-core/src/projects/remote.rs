//! Remote projects (E12-04): projects whose repository lives on an SSH host.
//!
//! `fartcode-core` is the leaf crate (ADR-0003), so the SSH machinery cannot be
//! imported here. The remote surface is a trait — [`RemoteHost`] — implemented
//! by `fartcode-ssh` over `SshClient` + `RemoteSftp`, exactly the split used
//! for `GitOps`.
//!
//! Two rules the rest of this module leans on:
//!
//! - **Nothing is shell-interpolated.** Remote commands are built from an argv
//!   array through [`crate::shell_escape::single_quote`]; a repo path with a
//!   space, a quote, or a `;` is data, never syntax.
//! - **Paths are contained before they are written.** Remote writes go through
//!   [`ensure_contained`], a lexical normalize + component prefix check (the
//!   same boundary `RemoteSftp` enforces; a remote symlink out of the root is
//!   still not caught — E12-02's note stands).

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::events::{EventBus, InternalEvent};
use crate::projects::model::{self, Project, WorkspaceProviderKind};
use crate::projects::provider::{self, safe_path_segment, FARTCODE_STATE_DIR};
use crate::shell_escape::single_quote;
use crate::Error;

// ── Remote host surface ──────────────────────────────────────

/// What a remote path is, when it exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteFileKind {
    File,
    Dir,
    Symlink,
}

/// One entry in a remote directory listing.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEntry {
    /// Absolute remote path.
    pub path: String,
    pub name: String,
    pub kind: RemoteFileKind,
}

/// Result of a remote command.
#[derive(Debug, Clone)]
pub struct RemoteOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl RemoteOutput {
    pub fn ok(&self) -> bool {
        self.exit_code == 0
    }

    pub fn stdout_trimmed(&self) -> &str {
        self.stdout.trim()
    }
}

/// Filesystem + command surface of a connected SSH host.
///
/// Implemented by `fartcode_ssh::host::SshRemoteHost`. Every method is
/// `&self` so a single connection can be shared; implementations serialize
/// their own SFTP session internally.
#[async_trait::async_trait]
pub trait RemoteHost: Send + Sync {
    /// Canonical absolute path (SFTP `realpath`).
    async fn realpath(&self, path: &str) -> Result<String, Error>;

    /// Directory listing for the picker (dirs first, hidden opt-in).
    async fn list_dir(&self, path: &str, include_hidden: bool) -> Result<Vec<RemoteEntry>, Error>;

    /// `None` when the path does not exist.
    async fn stat(&self, path: &str) -> Result<Option<RemoteFileKind>, Error>;

    /// `mkdir -p` (SFTP, parents included). Idempotent.
    async fn mkdir_all(&self, path: &str) -> Result<(), Error>;

    /// Recursive remove. Idempotent — a missing path is `Ok(())`.
    async fn remove_dir_all(&self, path: &str) -> Result<(), Error>;

    /// Run `argv` (optionally in `cwd`) and collect status + output.
    /// Implementations must build the command line with [`remote_command_line`].
    async fn run(&self, argv: &[&str], cwd: Option<&str>) -> Result<RemoteOutput, Error>;
}

/// Runs `argv` and fails on a nonzero exit, quoting stderr.
pub async fn run_checked(
    host: &dyn RemoteHost,
    argv: &[&str],
    cwd: Option<&str>,
) -> Result<RemoteOutput, Error> {
    let out = host.run(argv, cwd).await?;
    if !out.ok() {
        return Err(Error::Internal(format!(
            "remote command failed ({}): {} — {}",
            out.exit_code,
            argv.first().copied().unwrap_or(""),
            out.stderr.trim()
        )));
    }
    Ok(out)
}

/// Builds a `/bin/sh` command line from an argv array. Every element is
/// single-quoted — this is the only place a remote command line is assembled.
pub fn remote_command_line(argv: &[&str], cwd: Option<&str>) -> String {
    let command = argv
        .iter()
        .map(|a| single_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    match cwd {
        Some(dir) => format!("cd {} && {command}", single_quote(dir)),
        None => command,
    }
}

// ── Path handling ────────────────────────────────────────────

/// Lexical POSIX normalize: collapses `//`, drops `.`, resolves `..`
/// syntactically. Never touches the remote (a `realpath` round trip per
/// component would cost a round trip per component).
pub fn normalize_remote_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if matches!(parts.last(), Some(&last) if last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Component-wise containment (`/a/bc` is NOT inside `/a/b`).
pub fn is_contained(root: &str, candidate: &str) -> bool {
    let root = normalize_remote_path(root);
    let candidate = normalize_remote_path(candidate);
    if candidate == root {
        return true;
    }
    let prefix = if root.ends_with('/') {
        root
    } else {
        format!("{root}/")
    };
    candidate.starts_with(&prefix)
}

/// Guard for every remote write: `path` must stay under `root`.
pub fn ensure_contained(root: &str, path: &str) -> Result<String, Error> {
    if !is_contained(root, path) {
        return Err(Error::Internal(format!(
            "remote path escapes the project root: {path} (root {root})"
        )));
    }
    Ok(normalize_remote_path(path))
}

/// Joins remote path segments POSIX-style (the remote is never Windows —
/// `PathBuf` would use `\` on a Windows desktop and corrupt the path).
pub fn remote_join(base: &str, segment: &str) -> String {
    normalize_remote_path(&format!("{}/{segment}", base.trim_end_matches('/')))
}

// ── Worktrees on the remote ──────────────────────────────────

/// Pool segment for a remote project (#81): keyed on the project's
/// **workspace key** (`ssh:<conn>:<path>`), not its name — two projects with
/// the same directory name on different hosts must never share a pool.
pub fn remote_pool_segment(project: &Project) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider::repository_workspace_key(project).as_bytes());
    let hash8 = &format!("{:x}", hasher.finalize())[..8];
    format!("{}-{hash8}", safe_path_segment(&project.name, &project.id))
}

/// Remote worktree pool: `<project>/.fartCode/worktrees/<segment>`.
///
/// Unlike the local pool (a sibling directory), the remote pool lives *inside*
/// the project — one SSH root, one thing to clean up, and `.fartCode/` is
/// already git-excluded.
pub fn remote_worktree_root(project: &Project) -> String {
    let base = project.path.to_string_lossy();
    remote_join(
        &remote_join(&remote_join(&base, FARTCODE_STATE_DIR), "worktrees"),
        &remote_pool_segment(project),
    )
}

/// Path a branch's worktree gets inside the pool.
pub fn remote_worktree_path(project: &Project, branch: &str) -> Result<String, Error> {
    let root = remote_worktree_root(project);
    let segment = safe_path_segment(branch, "task");
    ensure_contained(&root, &remote_join(&root, &segment))
}

// ── Store ────────────────────────────────────────────────────

/// Creates and tears down SSH-backed projects. Mirrors `DbProjectStore`'s
/// create tail (row + workspace + `project:added`), minus the local-only open
/// flow: `.git/info/exclude` and worktree re-detection touch the local
/// filesystem, which for a remote project is the wrong machine.
pub struct RemoteProjectStore {
    db: Arc<dyn Db>,
    event_bus: Arc<dyn EventBus>,
}

impl RemoteProjectStore {
    pub fn new(db: Arc<dyn Db>, event_bus: Arc<dyn EventBus>) -> Self {
        Self { db, event_bus }
    }

    /// Adds an existing remote repository as a project.
    ///
    /// Validation order matters: `realpath` + `stat` before any row is
    /// written, and the repo check runs `git rev-parse` rather than statting
    /// `.git` (a linked worktree's `.git` is a *file*, and a bare repo has no
    /// `.git` at all).
    pub async fn create_remote(
        &self,
        host: &dyn RemoteHost,
        ssh_connection_id: &str,
        remote_path: &str,
    ) -> Result<Project, Error> {
        if remote_path.trim().is_empty() {
            return Err(Error::Internal("remote path is empty".into()));
        }
        let canonical = host.realpath(remote_path).await?;

        // Duplicate (connection, path) → open the existing row, never insert
        // a second one (same contract as create_local).
        if let Some(existing) = self.get_by_remote_path(ssh_connection_id, &canonical)? {
            return Ok(existing);
        }

        match host.stat(&canonical).await? {
            Some(RemoteFileKind::Dir) => {}
            Some(_) => {
                return Err(Error::Internal(format!(
                    "remote path is not a directory: {canonical}"
                )))
            }
            None => return Err(Error::ProjectPathNotFound(PathBuf::from(canonical))),
        }

        let toplevel = host
            .run(
                &["git", "-C", &canonical, "rev-parse", "--show-toplevel"],
                None,
            )
            .await?;
        if !toplevel.ok() {
            return Err(Error::Internal(format!(
                "not a git repository: {canonical}"
            )));
        }
        let repo_root = normalize_remote_path(toplevel.stdout_trimmed());

        // The toplevel can differ from what the user picked (a subdirectory) —
        // re-check for a duplicate on the resolved root.
        if let Some(existing) = self.get_by_remote_path(ssh_connection_id, &repo_root)? {
            return Ok(existing);
        }

        let base_ref = self.resolve_remote_base_ref(host, &repo_root).await?;
        self.finish_create(&repo_root, Some(base_ref), ssh_connection_id)
    }

    /// Clones `url` into `projects_dir` on the remote, then adds it.
    pub async fn create_remote_clone(
        &self,
        host: &dyn RemoteHost,
        ssh_connection_id: &str,
        url: &str,
        projects_dir: &str,
    ) -> Result<Project, Error> {
        let segment = crate::projects::repo_name_from_url(url);
        let target = remote_join(projects_dir, &segment);

        if let Some(existing) = self.get_by_remote_path(ssh_connection_id, &target)? {
            return Ok(existing);
        }
        if host.stat(&target).await?.is_some() {
            return Err(Error::Internal(format!(
                "clone target already exists: {target}"
            )));
        }
        // Fresh host: the projects directory may not exist yet.
        host.mkdir_all(projects_dir).await?;
        run_checked(host, &["git", "clone", url, &target], None).await?;

        self.create_remote(host, ssh_connection_id, &target).await
    }

    /// Creates (or reuses) the remote worktree for `branch` under the
    /// project's pool. Idempotent: an existing checkout of the branch is
    /// returned as-is.
    pub async fn ensure_remote_worktree(
        &self,
        host: &dyn RemoteHost,
        project: &Project,
        branch: &str,
    ) -> Result<String, Error> {
        let repo = project.path.to_string_lossy().into_owned();
        let root = remote_worktree_root(project);
        let path = remote_worktree_path(project, branch)?;
        host.mkdir_all(&root).await?;

        if host.stat(&path).await?.is_some() {
            let head = host
                .run(
                    &["git", "-C", &path, "rev-parse", "--abbrev-ref", "HEAD"],
                    None,
                )
                .await?;
            if head.ok() && head.stdout_trimmed() == branch {
                return Ok(path);
            }
            return Err(Error::WorktreeBranchConflict(format!(
                "'{path}' is checked out on '{}', expected '{branch}'",
                head.stdout_trimmed()
            )));
        }

        // `-B` so a re-created worktree reuses the branch instead of failing
        // on "already exists".
        run_checked(
            host,
            &["git", "-C", &repo, "worktree", "add", "-B", branch, &path],
            None,
        )
        .await?;
        Ok(path)
    }

    /// Removes a remote worktree and prunes the repo's administrative entries.
    /// Idempotent — a missing worktree is success.
    pub async fn remove_remote_worktree(
        &self,
        host: &dyn RemoteHost,
        project: &Project,
        branch: &str,
    ) -> Result<(), Error> {
        let repo = project.path.to_string_lossy().into_owned();
        let path = remote_worktree_path(project, branch)?;
        // Contained by construction, but the removal is recursive — check the
        // boundary at the call site too, not only where the path was built.
        ensure_contained(&remote_worktree_root(project), &path)?;
        host.remove_dir_all(&path).await?;
        let prune = host
            .run(&["git", "-C", &repo, "worktree", "prune"], None)
            .await?;
        if !prune.ok() {
            tracing::warn!(
                project_id = %project.id,
                stderr = %prune.stderr.trim(),
                "remote worktree prune failed (non-fatal)"
            );
        }
        Ok(())
    }

    // -- internals ----------------------------------------------------------

    /// Remote analogue of `DbProjectStore::resolve_base_ref`, minus the
    /// network-bound refinement: one `symbolic-ref` and one `remote` listing,
    /// no `git remote show` (ADR-0003 dropped it for the same reason — it can
    /// hang inside project creation).
    async fn resolve_remote_base_ref(
        &self,
        host: &dyn RemoteHost,
        repo: &str,
    ) -> Result<String, Error> {
        let remotes = host
            .run(&["git", "-C", repo, "remote"], None)
            .await?
            .stdout
            .lines()
            .map(str::trim)
            .find(|l| {
                !l.is_empty()
                    && !l.contains("://")
                    && l.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            })
            .map(str::to_string);

        let head = host
            .run(
                &["git", "-C", repo, "symbolic-ref", "--short", "HEAD"],
                None,
            )
            .await?;
        // Detached HEAD (or a fresh repo with no commits) has no symbolic ref.
        let branch = if head.ok() && !head.stdout_trimmed().is_empty() {
            head.stdout_trimmed().to_string()
        } else {
            "main".to_string()
        };

        // Reference computeBaseRef: slash-carrying branches stay bare,
        // plain branches take the remote prefix.
        Ok(match (&remotes, branch.contains('/')) {
            (Some(remote), false) => format!("{remote}/{branch}"),
            _ => branch,
        })
    }

    fn get_by_remote_path(
        &self,
        ssh_connection_id: &str,
        path: &str,
    ) -> Result<Option<Project>, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {} FROM projects WHERE ssh_connection_id = ?1 AND path = ?2",
                    model::PROJECT_COLUMNS
                ),
                rusqlite::params![ssh_connection_id, path],
                model::project_from_row,
            )
            .optional()?)
    }

    fn get(&self, id: &str) -> Result<Option<Project>, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
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
    }

    fn finish_create(
        &self,
        repo_root: &str,
        base_ref: Option<String>,
        ssh_connection_id: &str,
    ) -> Result<Project, Error> {
        let project = crate::projects::insert_project_row(
            self.db.as_ref(),
            &PathBuf::from(repo_root),
            WorkspaceProviderKind::Ssh,
            base_ref,
            Some(ssh_connection_id.to_string()),
        )?;
        if let Err(e) = provider::ensure_repository_workspace(self.db.as_ref(), &project) {
            tracing::warn!(project_id = %project.id, error = %e, "ensureRepositoryWorkspace failed (non-fatal)");
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_contains() {
        assert_eq!(normalize_remote_path("/srv//repo/./sub/"), "/srv/repo/sub");
        assert_eq!(normalize_remote_path("/srv/repo/sub/../x"), "/srv/repo/x");
        assert!(is_contained("/srv/repo", "/srv/repo/.fartCode/worktrees"));
        // Component-wise: a sibling with a shared prefix is not inside.
        assert!(!is_contained("/srv/repo", "/srv/repo-2/x"));
        assert!(!is_contained("/srv/repo", "/srv/repo/../etc"));
        assert!(ensure_contained("/srv/repo", "/srv/repo/../etc").is_err());
    }

    #[test]
    fn command_line_quotes_every_argument() {
        let line = remote_command_line(&["git", "clone", "https://x/y.git", "/srv/a b"], None);
        assert_eq!(line, "'git' 'clone' 'https://x/y.git' '/srv/a b'");
        // A path that tries to close the quote and chain a command stays data.
        let evil = remote_command_line(&["git", "-C", "/srv/'; rm -rf /; '"], Some("/srv/x"));
        assert!(evil.starts_with("cd '/srv/x' && "));
        assert!(!evil.contains("&& rm -rf"));
    }
}
