//! Git operations trait (ARCHITECTURE.md §6.4).
//!
//! **Placement note (deviation from §6.4):** the trait lives in `fartcode-core`
//! (not `fartcode-git`) so that `fartcode-core` domain modules (`projects`, later
//! `workspaces`) can depend on it without violating the crate-graph rule that
//! `fartcode-core` is the leaf (depends on nothing internal). The concrete
//! implementation (`fartcode_git::CliGit`) lives in `fartcode-git`, which depends on
//! `fartcode-core` and re-exports the trait.
//!
//! `CliGit` shells out to the `git` CLI via `std::process::Command` with
//! argument arrays — no shell, no quoting (see AGENTS.md "no ad-hoc shell
//! quoting"). git2 bindings arrive with E2-02 for worktree lifecycle
//! operations that need libgit2.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::Error;

/// A git branch (local or remote), from `git branch -a`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    /// Short ref name, e.g. `main`, `origin/main`.
    pub name: String,
    /// Upstream short name when the branch tracks one.
    pub upstream: Option<String>,
    /// True for remote-tracking branches (`refs/remotes/...`).
    pub is_remote: bool,
    /// Commit object name.
    pub object: String,
}

/// Parsed from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: String,
    /// `Some("refs/heads/xxx")` when on a branch; `None` when detached.
    pub branch: Option<String>,
    pub bare: bool,
    pub locked: bool,
}

/// Low-level git operations. Phase 0 implementation: `fartcode_git::CliGit`
/// (git CLI, no shell).
pub trait GitOps: Send + Sync {
    /// `git -C <path> rev-parse --is-inside-work-tree`.
    fn is_git_repo(&self, path: &Path) -> Result<bool, Error>;

    /// `git init` at `path` (a directory).
    fn init(&self, path: &Path) -> Result<(), Error>;

    /// `git clone <url> <target>`.
    fn clone(&self, url: &str, target: &Path) -> Result<(), Error>;

    /// `git -C <path> rev-parse --show-toplevel` — the repository root.
    fn show_toplevel(&self, path: &Path) -> Result<PathBuf, Error>;

    /// `git -C <path> rev-parse --git-dir` — the git dir (`.git` dir or file).
    fn git_dir(&self, path: &Path) -> Result<PathBuf, Error>;

    /// `git -C <path> remote` — configured remotes (origin first if present).
    fn remotes(&self, path: &Path) -> Result<Vec<String>, Error>;

    /// `git -C <path> branch --show-current` — the checked-out branch, if any.
    fn current_branch(&self, path: &Path) -> Result<Option<String>, Error>;

    /// Resolve a remote's default branch: `git symbolic-ref
    /// refs/remotes/<remote>/HEAD --short`, falling back to `git remote show
    /// <remote>`'s "HEAD branch:" line. `None` when unresolvable.
    fn remote_head(&self, path: &Path, remote: &str) -> Result<Option<String>, Error>;

    /// `git -C <path> rev-parse --verify <refname>` — does the ref exist?
    fn verify_ref(&self, path: &Path, refname: &str) -> Result<bool, Error>;

    /// `git -C <path> branch -a --format=...` — local and remote branches.
    fn branches(&self, path: &Path) -> Result<Vec<BranchRef>, Error>;

    /// `git -C <path> rev-parse --abbrev-ref HEAD` or resolve the default
    /// branch name (`main`/`master`/...), whichever applies.
    fn default_branch(&self, path: &Path) -> Result<String, Error>;

    /// `git -C <path> worktree list --porcelain`.
    fn worktree_list(&self, repo_path: &Path) -> Result<Vec<WorktreeEntry>, Error>;

    /// Check if `path` is inside a git worktree.
    ///
    /// Phase 0: implemented as `rev-parse --is-inside-work-tree` (true for the
    /// main checkout too). E2-02's worktree manager should distinguish
    /// linked worktrees from the main checkout via `worktree_list`.
    fn is_worktree(&self, path: &Path) -> Result<bool, Error>;

    /// `git -C <repo> fetch <remote>`.
    fn fetch(&self, repo_path: &Path, remote: &str) -> Result<(), Error>;

    /// `git -C <repo> worktree add <target_path> <branch>`.
    fn worktree_add(&self, repo_path: &Path, target_path: &Path, branch: &str)
        -> Result<(), Error>;

    /// `git -C <repo> worktree prune`.
    fn worktree_prune(&self, repo_path: &Path) -> Result<(), Error>;

    /// Remove a worktree directory + `git worktree prune`.
    fn worktree_remove(&self, repo_path: &Path, worktree_path: &Path) -> Result<(), Error>;

    /// `git -C <repo> worktree prune` with a hard deadline — cleanup paths
    /// (task deletion, E2-09) must never hang on a wedged git. Returns
    /// `Error::GitTimeout` when prune does not finish within `timeout`.
    fn worktree_prune_timed(&self, repo_path: &Path, timeout: Duration) -> Result<(), Error>;
    /// `git -C <worktree> status --porcelain` — empty output means the
    /// worktree is clean (no uncommitted changes or untracked files). Used as
    /// a dirty-check before `rm -rf` (E2-07 follow-up).
    fn is_worktree_clean(&self, repo: &Path, worktree: &Path) -> Result<bool, Error>;

    /// `git -C <repo> branch --no-track <name> <start_point>` (no upstream set —
    /// the reference's create flow uses `--no-track`).
    fn branch_create(&self, repo_path: &Path, name: &str, start_point: &str) -> Result<(), Error>;

    /// `git -C <repo> branch -d -- <name>` (`-D` when `force`). Used by task
    /// deletion (E2-09) when the workspace provisioned its own branch.
    fn branch_delete(&self, repo_path: &Path, name: &str, force: bool) -> Result<(), Error>;

    // -- E2-02 additions -----------------------------------------------------

    /// `git -C <repo> config --get <key>` (None when unset).
    fn config_get(&self, repo_path: &Path, key: &str) -> Result<Option<String>, Error>;

    /// `git -C <repo> config <key> <value>`.
    fn config_set(&self, repo_path: &Path, key: &str, value: &str) -> Result<(), Error>;

    /// `git -C <repo> push [-u] <remote> <branch>`.
    fn push(
        &self,
        repo_path: &Path,
        remote: &str,
        branch: &str,
        set_upstream: bool,
    ) -> Result<(), Error>;

    /// Is `rel_path` (repo-relative) tracked by git? (`git ls-files
    /// --error-unmatch` — false for untracked/ignored files.)
    fn is_tracked(&self, repo_path: &Path, rel_path: &str) -> Result<bool, Error>;
}
