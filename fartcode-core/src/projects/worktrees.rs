//! Worktree manager (ticket E2-02, ARCHITECTURE.md §8.3).
//!
//! Every task gets an isolated worktree on its own branch. This manager
//! creates, validates, reuses, and cleans them safely. Ports the reference
//! `worktree-service.ts` + `workspace-setup-spec.ts` create flow:
//!
//! `fetch (remote source) → create branch (--no-track) → set
//! branch.<name>.base config → add worktree → copy preserved files →
//! push (non-fatal)`, with reuse of valid existing worktrees, stale-path
//! removal, a never-remove-project-root guard, sibling-task blocking, and
//! prune + re-detection on restart.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::git::{GitOps, WorktreeEntry};
use crate::projects::model::Project;
use crate::projects::provider::worktree_pool_path;
use crate::settings::{SettingsStore, DEFAULT_PRESERVE_PATTERNS};
use crate::Error;

/// Reference `pruneGitWorktrees`: cleanup prunes are bounded (5s) +
/// non-interactive so a wedged git cannot hang task teardown (E2-09).
pub const WORKTREE_PRUNE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What a task's workspace turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceKind {
    Worktree,
    ProjectRoot,
    Byoi,
}

impl WorkspaceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkspaceKind::Worktree => "worktree",
            WorkspaceKind::ProjectRoot => "project-root",
            WorkspaceKind::Byoi => "byoi",
        }
    }
}

/// Outcome of `WorktreeManager::ensure_worktree`.
#[derive(Debug, Clone)]
pub struct EnsureWorktreeResult {
    pub path: PathBuf,
    pub kind: WorkspaceKind,
    /// Isolation warning when the task runs in the project root (worktree
    /// disabled) instead of a worktree.
    pub warning: Option<String>,
    pub reused: bool,
}

/// Options for `ensure_worktree` (reference `checkoutBranchWorktree` /
/// `checkoutExistingBranch`).
#[derive(Debug, Clone)]
pub struct EnsureWorktreeOptions<'a> {
    pub project: &'a Project,
    pub task_id: &'a str,
    /// The task's workspace row id (created by the task create flow).
    pub workspace_id: &'a str,
    /// Task branch name (E2-03 `resolve_task_branch_name`).
    pub branch_name: &'a str,
    /// Source base ref: `"origin/main"` / `"main"`. `None` = existing-branch
    /// flow (fetch candidates, track a remote branch).
    pub source_ref: Option<&'a str>,
    /// `tasks.createBranchAndWorktree` — when false, run in the project root.
    pub worktree_enabled: bool,
}

/// Creates and manages task worktrees.
pub struct WorktreeManager {
    db: Arc<dyn Db>,
    settings: Arc<dyn SettingsStore>,
    git: Arc<dyn GitOps>,
}

impl WorktreeManager {
    pub fn new(db: Arc<dyn Db>, settings: Arc<dyn SettingsStore>, git: Arc<dyn GitOps>) -> Self {
        Self { db, settings, git }
    }

    /// The git backend (E2-09 branch deletion drives it directly).
    pub fn git(&self) -> &Arc<dyn GitOps> {
        &self.git
    }

    /// `git worktree prune` for the project (constructor-prune behavior;
    /// also the first half of restart re-detection).
    pub fn prune(&self, project: &Project) -> Result<(), Error> {
        self.git.worktree_prune(&project.path)
    }

    /// Re-detects the project's worktrees: prune stale metadata, then list
    /// entries inside the pool. Restart-safe — never creates duplicates.
    pub fn reconcile(&self, project: &Project) -> Result<Vec<WorktreeEntry>, Error> {
        self.prune(project)?;
        let entries = self.git.worktree_list(&project.path)?;
        let pool = worktree_pool_path(self.db.as_ref(), self.settings.as_ref(), project)?;
        let pool_real = std::fs::canonicalize(&pool).unwrap_or(pool);
        Ok(entries
            .into_iter()
            .filter(|e| e.path.starts_with(&pool_real))
            .collect())
    }

    /// Ensures the task has a worktree on `branch_name` (or falls back to the
    /// project root when worktrees are disabled). Idempotent: reuses a valid
    /// existing worktree for the branch.
    pub fn ensure_worktree(
        &self,
        opts: &EnsureWorktreeOptions<'_>,
    ) -> Result<EnsureWorktreeResult, Error> {
        if !opts.worktree_enabled {
            // Worktree-less tasks run in the project root (acceptance 2).
            self.set_workspace_row(
                opts.workspace_id,
                Some(&opts.project.path),
                WorkspaceKind::ProjectRoot,
                None,
            )?;
            return Ok(EnsureWorktreeResult {
                path: opts.project.path.clone(),
                kind: WorkspaceKind::ProjectRoot,
                warning: Some(format!(
                    "worktrees disabled — task runs directly in the project root ({}), not an isolated worktree",
                    opts.project.path.display()
                )),
                reused: false,
            });
        }

        let pool = worktree_pool_path(self.db.as_ref(), self.settings.as_ref(), opts.project)?;
        let worktree_path = pool.join(opts.branch_name);
        self.prune(opts.project)?;

        // 1. Reuse an existing valid worktree for this branch anywhere in the pool.
        let entries = self.git.worktree_list(&opts.project.path)?;
        if let Some(entry) = entries.iter().find(|e| {
            e.branch.as_deref() == Some(&format!("refs/heads/{}", opts.branch_name))
                && self.is_valid_worktree(&e.path)
        }) {
            let path = entry.path.clone();
            self.set_workspace_row(
                opts.workspace_id,
                Some(&path),
                WorkspaceKind::Worktree,
                Some(opts.branch_name),
            )?;
            return Ok(EnsureWorktreeResult {
                path,
                kind: WorkspaceKind::Worktree,
                warning: None,
                reused: true,
            });
        }

        // 2. The expected path exists? Reuse if valid (and on the right
        // branch), else remove stale.
        if worktree_path.exists() {
            if self.is_valid_worktree(&worktree_path) {
                let current = self.git.current_branch(&worktree_path)?;
                if current.as_deref() != Some(opts.branch_name) {
                    // Reference conflict: the path holds a different branch.
                    return Err(Error::WorktreeBranchConflict(format!(
                        "'{}' is checked out on '{}', expected '{}'",
                        worktree_path.display(),
                        current.unwrap_or_else(|| "(detached)".into()),
                        opts.branch_name
                    )));
                }
                self.set_workspace_row(
                    opts.workspace_id,
                    Some(&worktree_path),
                    WorkspaceKind::Worktree,
                    Some(opts.branch_name),
                )?;
                return Ok(EnsureWorktreeResult {
                    path: worktree_path,
                    kind: WorkspaceKind::Worktree,
                    warning: None,
                    reused: true,
                });
            }
            self.remove_stale_path(&pool, &worktree_path)?;
            self.prune(opts.project)?;
        }

        // 3. Branch must exist (create from source if not).
        let branch_exists = self.git.verify_ref(
            &opts.project.path,
            &format!("refs/heads/{}", opts.branch_name),
        )?;
        if !branch_exists {
            self.create_branch(opts)?;
        }

        // 4. branch.<name>.base config (metadata for external tooling; non-fatal).
        self.ensure_branch_base_config(opts)?;

        // 5. Add the worktree.
        if let Some(parent) = worktree_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.git
            .worktree_add(&opts.project.path, &worktree_path, opts.branch_name)?;

        // 6. Copy preserved files (non-fatal).
        if let Err(e) = self.copy_preserved_files(opts.project, &worktree_path) {
            tracing::warn!(error = %e, "copyPreservedFiles failed (non-fatal)");
        }

        // 7. Record the path + branch on the workspace row.
        self.set_workspace_row(
            opts.workspace_id,
            Some(&worktree_path),
            WorkspaceKind::Worktree,
            Some(opts.branch_name),
        )?;

        // 8. Push the branch (non-fatal, remote-source flow only).
        self.push_branch_nonfatal(opts);

        Ok(EnsureWorktreeResult {
            path: worktree_path,
            kind: WorkspaceKind::Worktree,
            warning: None,
            reused: false,
        })
    }

    /// Removes a task's worktree. Guards: never the project root, never a
    /// path outside the pool, and never a workspace shared with sibling
    /// tasks. Returns `Ok(true)` when the worktree was removed, `Ok(false)`
    /// when siblings kept it (reference `removeWorktreeIfUnused`).
    ///
    /// `force`: skip the dirty-check — task DELETION is user-confirmed and
    /// the reference removes unconditionally; non-deletion flows pass
    /// `false` so uncommitted agent work is never destroyed silently
    /// (E2-07 follow-up). Cleanup prunes with a bounded timeout (reference
    /// 5s + non-interactive env) so a wedged git cannot hang teardown.
    pub fn remove_worktree(
        &self,
        project: &Project,
        task_id: &str,
        workspace_id: &str,
        worktree_path: &Path,
        force: bool,
    ) -> Result<bool, Error> {
        let project_real =
            std::fs::canonicalize(&project.path).unwrap_or_else(|_| project.path.clone());
        let worktree_real =
            std::fs::canonicalize(worktree_path).unwrap_or_else(|_| worktree_path.to_path_buf());
        if worktree_real == project_real {
            return Err(Error::CannotRemoveProjectRoot);
        }
        let pool = worktree_pool_path(self.db.as_ref(), self.settings.as_ref(), project)?;
        let pool_real = std::fs::canonicalize(&pool).unwrap_or(pool);
        if !worktree_real.starts_with(&pool_real) {
            return Err(Error::Internal(format!(
                "refusing to remove worktree outside pool: {}",
                worktree_path.display()
            )));
        }

        // Sibling tasks sharing this workspace block removal (acceptance 3).
        if self.workspace_has_remaining_tasks(workspace_id, task_id)? {
            tracing::warn!(
                workspace_id,
                "workspace has sibling tasks — keeping shared worktree"
            );
            return Ok(false);
        }

        // Dirty-check (E2-07 follow-up): uncommitted work in the worktree is
        // the product's core premise. Destroying it outside a confirmed
        // delete is data loss.
        if !force
            && !self
                .git
                .is_worktree_clean(&project.path, worktree_path)
                .unwrap_or(false)
        {
            return Err(Error::DirtyWorktree(format!(
                "worktree {} has uncommitted changes — refusing to remove",
                worktree_path.display()
            )));
        }
        if worktree_path.exists() {
            std::fs::remove_dir_all(worktree_path)?;
        }
        // Bounded prune (reference pruneGitWorktrees): failure is non-fatal —
        // the directory is already gone; stale metadata heals on next prune.
        if let Err(e) = self
            .git
            .worktree_prune_timed(&project.path, WORKTREE_PRUNE_TIMEOUT)
        {
            tracing::warn!(error = %e, "worktree prune after removal failed (non-fatal)");
        }
        Ok(true)
    }

    // -- internals -----------------------------------------------------------

    fn is_valid_worktree(&self, path: &Path) -> bool {
        // `.git` file/dir exists AND rev-parse --is-inside-work-tree (reference
        // isValidWorktree).
        let git_file = path.join(".git");
        if !git_file.exists() {
            return false;
        }
        self.git.is_git_repo(path).unwrap_or(false)
    }

    fn create_branch(&self, opts: &EnsureWorktreeOptions<'_>) -> Result<(), Error> {
        let repo = &opts.project.path;
        match opts.source_ref {
            Some(source) => {
                // Fetch the remote when the source is remote-qualified (non-fatal).
                if let Some(remote) = remote_of(source, &self.git.remotes(repo)?) {
                    if let Err(e) = self.git.fetch(repo, &remote) {
                        tracing::warn!(error = %e, "fetch failed (non-fatal)");
                    }
                }
                // Verify the source ref exists before branching from it.
                let verify = if source.contains('/') {
                    format!("refs/remotes/{source}")
                } else {
                    format!("refs/heads/{source}")
                };
                if !self.git.verify_ref(repo, &verify)? {
                    return Err(Error::Git(format!("source ref not found: {source}")));
                }
                self.git.branch_create(repo, opts.branch_name, source)?;
                Ok(())
            }
            None => {
                // Existing-branch flow: fetch candidates [baseRemote, origin],
                // then branch --no-track from the first remote that has it.
                let mut candidates: Vec<String> = self.git.remotes(repo)?;
                if !candidates.iter().any(|r| r == "origin") {
                    candidates.push("origin".into());
                }
                for remote in candidates {
                    if self.git.fetch(repo, &remote).is_err() {
                        continue;
                    }
                    let refname = format!("refs/remotes/{remote}/{}", opts.branch_name);
                    if self.git.verify_ref(repo, &refname)? {
                        self.git.branch_create(
                            repo,
                            opts.branch_name,
                            &format!("{remote}/{}", opts.branch_name),
                        )?;
                        return Ok(());
                    }
                }
                Err(Error::Git(format!(
                    "branch {} not found locally or on any remote",
                    opts.branch_name
                )))
            }
        }
    }

    /// `branch.<name>.base` — write-only metadata (idempotent, non-fatal).
    fn ensure_branch_base_config(&self, opts: &EnsureWorktreeOptions<'_>) -> Result<(), Error> {
        let key = format!("branch.{}.base", opts.branch_name);
        if self.git.config_get(&opts.project.path, &key)?.is_some() {
            return Ok(());
        }
        let base = match opts.source_ref {
            Some(source) => source.to_string(),
            None => opts.branch_name.to_string(),
        };
        if let Err(e) = self.git.config_set(&opts.project.path, &key, &base) {
            tracing::warn!(error = %e, "set branch base config failed (non-fatal)");
        }
        Ok(())
    }

    fn push_branch_nonfatal(&self, opts: &EnsureWorktreeOptions<'_>) {
        let Some(source) = opts.source_ref else {
            return;
        };
        let Some(remote) = remote_of(
            source,
            &self.git.remotes(&opts.project.path).unwrap_or_default(),
        ) else {
            return;
        };
        if let Err(e) = self
            .git
            .push(&opts.project.path, &remote, opts.branch_name, true)
        {
            tracing::warn!(error = %e, "push failed (non-fatal)");
        }
    }

    /// Copies untracked, non-`.fartCode.json` files matching the effective
    /// preserve patterns from the project into the worktree (reference
    /// `copyPreservedFiles`). All failures are non-fatal (warn).
    fn copy_preserved_files(&self, project: &Project, worktree: &Path) -> Result<(), Error> {
        let effective = self
            .settings
            .get_project_settings(&project.id, &project.path)
            .unwrap_or_default();
        let patterns = effective.preserve_patterns.unwrap_or_else(|| {
            DEFAULT_PRESERVE_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect()
        });

        for pattern in &patterns {
            if !is_safe_preserve_pattern(pattern) {
                continue;
            }
            let full_pattern = project.path.join(pattern);
            let options = glob::MatchOptions {
                case_sensitive: true,
                require_literal_separator: false,
                require_literal_leading_dot: false,
            };
            for entry in glob::glob_with(&full_pattern.to_string_lossy(), options)
                .map_err(|e| Error::Internal(format!("bad preserve pattern {pattern}: {e}")))?
                .flatten()
            {
                let rel = entry
                    .strip_prefix(&project.path)
                    .unwrap_or(&entry)
                    .to_path_buf();
                if rel.as_os_str() == ".fartCode.json" {
                    continue;
                }
                // Never copy git internals into the worktree (reference skips
                // paths starting with .git/; with require_literal_leading_dot
                // off, a broad pattern could otherwise match .git contents).
                let rel_str = rel.to_string_lossy();
                if rel_str == ".git" || rel_str.starts_with(".git/") {
                    continue;
                }
                if !entry.is_file() {
                    continue; // symlinks + dirs skipped (reference)
                }
                if self
                    .git
                    .is_tracked(&project.path, &rel_str)
                    .unwrap_or(false)
                {
                    continue; // only untracked files are copied
                }
                let dest = worktree.join(&rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if let Err(e) = std::fs::copy(&entry, &dest) {
                    tracing::warn!(error = %e, path = %rel.display(), "copy preserved file failed (non-fatal)");
                }
            }
        }
        Ok(())
    }

    fn set_workspace_row(
        &self,
        workspace_id: &str,
        path: Option<&Path>,
        kind: WorkspaceKind,
        branch_name: Option<&str>,
    ) -> Result<(), Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        conn.execute(
            "UPDATE workspaces SET path = ?1, branch_name = ?2, kind = ?3, updated_at = datetime('now') WHERE id = ?4",
            rusqlite::params![path.map(|p| p.to_string_lossy().into_owned()), branch_name, kind.as_str(), workspace_id],
        )?;
        Ok(())
    }

    /// `SELECT 1 FROM tasks WHERE workspace_id = ? AND id != ? LIMIT 1` —
    /// any other task sharing this workspace (reference
    /// `workspaceHasRemainingTasks`).
    fn workspace_has_remaining_tasks(
        &self,
        workspace_id: &str,
        exclude_task_id: &str,
    ) -> Result<bool, Error> {
        let conn = self
            .db
            .conn()
            .lock()
            .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM tasks WHERE workspace_id = ?1 AND id != ?2 LIMIT 1",
                rusqlite::params![workspace_id, exclude_task_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    fn remove_stale_path(&self, pool: &Path, target: &Path) -> Result<(), Error> {
        let pool_real = std::fs::canonicalize(pool).unwrap_or_else(|_| pool.to_path_buf());
        let target_real = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
        if !target_real.starts_with(&pool_real) {
            return Err(Error::Internal(format!(
                "refusing to remove worktree path outside pool: {}",
                target.display()
            )));
        }
        if target.exists() {
            std::fs::remove_dir_all(target)?;
        }
        Ok(())
    }
}

/// The remote name when `ref` is remote-qualified (`origin/main` -> origin),
/// else `None`.
fn remote_of(r#ref: &str, remotes: &[String]) -> Option<String> {
    let (head, _) = r#ref.split_once('/')?;
    if remotes.iter().any(|r| r == head) {
        Some(head.to_string())
    } else {
        None
    }
}

/// Reference `isSafePreservePattern`: non-blank, not absolute, no `..` segment.
fn is_safe_preserve_pattern(pattern: &str) -> bool {
    let trimmed = pattern.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with('/')
        && !Path::new(trimmed)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}
