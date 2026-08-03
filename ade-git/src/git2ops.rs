//! git2-backed worktree operations (ticket E2-02 git strategy).
//!
//! `git2::Repository` is `!Sync`, so no repository handle is ever held across
//! calls: each method opens the repo, does its work, and drops it. Mutating /
//! listing worktree ops are additionally serialized through a `Mutex<()>` (the
//! ticket's "all git ops through a single-threaded queue" — libgit2 writes to
//! the same worktree metadata). Everything else (fetch, branch, rev-parse,
//! config, ...) is delegated to `CliGit`, per the ticket's "fall back to the
//! git CLI for operations git2 doesn't cover or that are simpler via CLI".

use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use ade_core::git::{BranchRef, GitOps, WorktreeEntry};
use ade_core::Error;

use crate::CliGit;

/// git2-backed worktree ops with CLI delegation for the rest.
#[derive(Debug)]
pub struct Git2Ops {
    cli: CliGit,
    worktree_lock: Mutex<()>,
}

impl Default for Git2Ops {
    fn default() -> Self {
        Self::new()
    }
}

impl Git2Ops {
    pub fn new() -> Self {
        Self {
            cli: CliGit,
            worktree_lock: Mutex::new(()),
        }
    }

    fn open_repo(path: &Path) -> Result<git2::Repository, Error> {
        git2::Repository::open(path)
            .map_err(|e| Error::Git(format!("git2 open {}: {}", path.display(), e.message())))
    }
}

fn git2_err(context: &str, e: &git2::Error) -> Error {
    Error::Git(format!("git2 {context}: {}", e.message()))
}

/// Prune stale worktree entries. Callers must hold `worktree_lock`. Matches
/// `git worktree prune`: drops entries whose working dir is gone (which
/// covers a broken gitdir link, since `wt.path()` is derived from it), skips
/// locked worktrees, and surfaces the first prune failure instead of
/// swallowing it.
fn prune_locked(repo: &git2::Repository) -> Result<(), Error> {
    let names = repo.worktrees().map_err(|e| git2_err("worktrees()", &e))?;
    for name in names.iter() {
        let name = name
            .map_err(|e| git2_err("worktree name", &e))?
            .ok_or_else(|| Error::Internal("null worktree name".into()))?;
        let wt = repo
            .find_worktree(name)
            .map_err(|e| git2_err(&format!("find_worktree({name})"), &e))?;
        if wt
            .is_locked()
            .is_ok_and(|s| s != git2::WorktreeLockStatus::Unlocked)
        {
            continue; // locked worktrees are never pruned
        }
        let stale = !wt.path().exists();
        if stale {
            let mut opts = git2::WorktreePruneOptions::new();
            wt.prune(Some(&mut opts))
                .map_err(|e| git2_err(&format!("prune worktree '{name}'"), &e))?;
        }
    }
    Ok(())
}

impl GitOps for Git2Ops {
    fn worktree_list(&self, repo_path: &Path) -> Result<Vec<WorktreeEntry>, Error> {
        let _guard = self
            .worktree_lock
            .lock()
            .map_err(|_| Error::Internal("worktree mutex poisoned".into()))?;
        let repo = Self::open_repo(repo_path)?;

        let mut entries = Vec::new();

        // Main worktree: libgit2's worktree list yields LINKED worktrees only,
        // but the CLI (`git worktree list`) and the reference include the main
        // repo — synthesize it for parity.
        if let Some(workdir) = repo.workdir() {
            let (head, branch) = repo_head_and_branch(&repo);
            entries.push(WorktreeEntry {
                path: std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf()),
                head,
                branch,
                bare: repo.is_bare(),
                locked: false,
            });
        }

        let names = repo.worktrees().map_err(|e| git2_err("worktrees()", &e))?;
        for name in names.iter() {
            let name = name
                .map_err(|e| git2_err("worktree name", &e))?
                .ok_or_else(|| Error::Internal("null worktree name".into()))?;
            let wt = repo
                .find_worktree(name)
                .map_err(|e| git2_err(&format!("find_worktree({name})"), &e))?;
            let path = wt.path().to_path_buf();
            let wt_repo = git2::Repository::open(&path);
            let (head, branch, bare, locked) = match wt_repo {
                Ok(wt_repo) => {
                    let bare = wt_repo.is_bare();
                    let (head, branch) = repo_head_and_branch(&wt_repo);
                    (
                        head,
                        branch,
                        bare,
                        wt.is_locked()
                            .is_ok_and(|s| s != git2::WorktreeLockStatus::Unlocked),
                    )
                }
                Err(_) => (
                    String::new(),
                    None::<String>,
                    false,
                    wt.is_locked()
                        .is_ok_and(|s| s != git2::WorktreeLockStatus::Unlocked),
                ),
            };
            entries.push(WorktreeEntry {
                path,
                head,
                branch,
                bare,
                locked,
            });
        }
        Ok(entries)
    }

    fn worktree_add(
        &self,
        repo_path: &Path,
        target_path: &Path,
        branch: &str,
    ) -> Result<(), Error> {
        let _guard = self
            .worktree_lock
            .lock()
            .map_err(|_| Error::Internal("worktree mutex poisoned".into()))?;
        let repo = Self::open_repo(repo_path)?;
        let name = target_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("worktree");

        let mut opts = git2::WorktreeAddOptions::new();
        let reference = repo
            .find_reference(&format!("refs/heads/{branch}"))
            .map_err(|e| git2_err(&format!("find ref refs/heads/{branch}"), &e))?;
        // Refuse when the branch is already checked out elsewhere — parity
        // with `git worktree add` ("already checked out at ..."), which the
        // reference treats as a conflict.
        let refname = format!("refs/heads/{branch}");
        let mut checked_out: Vec<String> = Vec::new();
        if let Ok(head) = repo.head() {
            if head.name().ok() == Some(refname.as_str()) {
                checked_out.push(
                    repo.workdir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(main)".into()),
                );
            }
        }
        for wt_name in repo
            .worktrees()
            .map_err(|e| git2_err("worktrees()", &e))?
            .iter()
        {
            let wt_name = wt_name
                .map_err(|e| git2_err("worktree name", &e))?
                .ok_or_else(|| Error::Internal("null worktree name".into()))?;
            let wt = repo
                .find_worktree(wt_name)
                .map_err(|e| git2_err(&format!("find_worktree({wt_name})"), &e))?;
            if let Ok(wt_repo) = git2::Repository::open(wt.path()) {
                if let Ok(head) = wt_repo.head() {
                    if head.name().ok() == Some(refname.as_str()) {
                        checked_out.push(wt.path().display().to_string());
                    }
                }
            }
        }
        if let Some(where_else) = checked_out.first() {
            return Err(Error::Git(format!(
                "branch '{branch}' is already checked out at '{where_else}'"
            )));
        }

        opts.reference(Some(&reference));
        let _wt = repo
            .worktree(name, target_path, Some(&opts))
            .map_err(|e| git2_err("worktree add", &e))?;

        // `git worktree add` also checks the branch out into the worktree;
        // libgit2 creates the metadata only, so do the checkout ourselves.
        let wt_repo =
            git2::Repository::open(target_path).map_err(|e| git2_err("open new worktree", &e))?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force().remove_untracked(true);
        wt_repo
            .checkout_head(Some(&mut checkout))
            .map_err(|e| git2_err("checkout in new worktree", &e))?;
        Ok(())
    }

    fn worktree_prune(&self, repo_path: &Path) -> Result<(), Error> {
        let _guard = self
            .worktree_lock
            .lock()
            .map_err(|_| Error::Internal("worktree mutex poisoned".into()))?;
        let repo = Self::open_repo(repo_path)?;
        prune_locked(&repo)
    }

    fn is_worktree_clean(&self, repo: &Path, worktree: &Path) -> Result<bool, Error> {
        // Delegates to the shell — git2's status API requires index
        // manipulation; the CLI porcelain is simpler and reference-parity.
        self.cli.is_worktree_clean(repo, worktree)
    }

    fn worktree_remove(&self, repo_path: &Path, worktree_path: &Path) -> Result<(), Error> {
        let _guard = self
            .worktree_lock
            .lock()
            .map_err(|_| Error::Internal("worktree mutex poisoned".into()))?;
        if worktree_path.exists() {
            std::fs::remove_dir_all(worktree_path)?;
        }
        let repo = Self::open_repo(repo_path)?;
        prune_locked(&repo)
    }

    // -- everything else delegates to the CLI --------------------------------

    fn is_git_repo(&self, path: &Path) -> Result<bool, Error> {
        self.cli.is_git_repo(path)
    }
    fn init(&self, path: &Path) -> Result<(), Error> {
        self.cli.init(path)
    }
    fn clone(&self, url: &str, target: &Path) -> Result<(), Error> {
        self.cli.clone(url, target)
    }
    fn show_toplevel(&self, path: &Path) -> Result<PathBuf, Error> {
        self.cli.show_toplevel(path)
    }
    fn git_dir(&self, path: &Path) -> Result<PathBuf, Error> {
        self.cli.git_dir(path)
    }
    fn remotes(&self, path: &Path) -> Result<Vec<String>, Error> {
        self.cli.remotes(path)
    }
    fn current_branch(&self, path: &Path) -> Result<Option<String>, Error> {
        self.cli.current_branch(path)
    }
    fn remote_head(&self, path: &Path, remote: &str) -> Result<Option<String>, Error> {
        self.cli.remote_head(path, remote)
    }
    fn verify_ref(&self, path: &Path, refname: &str) -> Result<bool, Error> {
        self.cli.verify_ref(path, refname)
    }
    fn branches(&self, path: &Path) -> Result<Vec<BranchRef>, Error> {
        self.cli.branches(path)
    }
    fn default_branch(&self, path: &Path) -> Result<String, Error> {
        self.cli.default_branch(path)
    }
    fn is_worktree(&self, path: &Path) -> Result<bool, Error> {
        self.cli.is_worktree(path)
    }
    fn fetch(&self, repo_path: &Path, remote: &str) -> Result<(), Error> {
        self.cli.fetch(repo_path, remote)
    }
    fn branch_create(&self, repo_path: &Path, name: &str, start_point: &str) -> Result<(), Error> {
        self.cli.branch_create(repo_path, name, start_point)
    }
    fn config_get(&self, repo_path: &Path, key: &str) -> Result<Option<String>, Error> {
        self.cli.config_get(repo_path, key)
    }
    fn config_set(&self, repo_path: &Path, key: &str, value: &str) -> Result<(), Error> {
        self.cli.config_set(repo_path, key, value)
    }
    fn push(
        &self,
        repo_path: &Path,
        remote: &str,
        branch: &str,
        set_upstream: bool,
    ) -> Result<(), Error> {
        self.cli.push(repo_path, remote, branch, set_upstream)
    }
    fn is_tracked(&self, repo_path: &Path, rel_path: &str) -> Result<bool, Error> {
        self.cli.is_tracked(repo_path, rel_path)
    }
}

/// `(head_oid, branch_name)` for a repository — branch is the checked-out
/// `refs/heads/*` ref name, if any.
fn repo_head_and_branch(repo: &git2::Repository) -> (String, Option<String>) {
    match repo.head() {
        Ok(head_ref) => {
            let head = head_ref
                .target()
                .map(|oid| oid.to_string())
                .unwrap_or_default();
            // name() -> Result<&str>: only branches count.
            let branch = match head_ref.name() {
                Ok(name) if name.starts_with("refs/heads/") => Some(name.to_string()),
                _ => None,
            };
            (head, branch)
        }
        Err(_) => (String::new(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git_ok(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed in {repo:?}");
    }

    fn init_repo_with_commit(tmp: &tempfile::TempDir) -> PathBuf {
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = Git2Ops::new();
        git.init(&repo).unwrap();
        std::fs::write(repo.join("README.md"), "# test\n").unwrap();
        git_ok(&repo, &["add", "."]);
        git_ok(
            &repo,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@t",
                "commit",
                "-m",
                "init",
            ],
        );
        git_ok(&repo, &["branch", "-M", "main"]);
        repo
    }

    #[test]
    fn git2_worktree_add_list_remove_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo_with_commit(&tmp);
        let git = Git2Ops::new();

        git.branch_create(&repo, "feature", "main").unwrap();
        let wt_raw = tmp.path().join("wt-feature");
        git.worktree_add(&repo, &wt_raw, "feature").unwrap();

        // The checked-out files must be present (parity with `git worktree add`).
        assert!(
            wt_raw.join("README.md").exists(),
            "worktree must be checked out"
        );

        let entries = git.worktree_list(&repo).unwrap();
        let wt = std::fs::canonicalize(&wt_raw).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.path == wt && e.branch.as_deref() == Some("refs/heads/feature")),
            "entries: {entries:?}"
        );
        // The main worktree is reported too.
        assert!(entries
            .iter()
            .any(|e| e.path == std::fs::canonicalize(&repo).unwrap()));

        git.worktree_remove(&repo, &wt_raw).unwrap();
        assert!(!wt_raw.exists());
        let entries = git.worktree_list(&repo).unwrap();
        assert!(!entries.iter().any(|e| e.path == wt));
    }

    #[test]
    fn git2_prune_removes_stale_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo_with_commit(&tmp);
        let git = Git2Ops::new();

        git.branch_create(&repo, "feature", "main").unwrap();
        let wt = tmp.path().join("wt-feature");
        git.worktree_add(&repo, &wt, "feature").unwrap();
        assert_eq!(git.worktree_list(&repo).unwrap().len(), 2);

        // Simulate a manually deleted worktree dir: prune must drop its entry.
        std::fs::remove_dir_all(&wt).unwrap();
        git.worktree_prune(&repo).unwrap();
        let entries = git.worktree_list(&repo).unwrap();
        assert_eq!(
            entries.len(),
            1,
            "stale worktree metadata pruned: {entries:?}"
        );
    }

    #[test]
    fn git2_worktree_add_refuses_checked_out_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo_with_commit(&tmp);
        let git = Git2Ops::new();

        git.branch_create(&repo, "feature", "main").unwrap();
        let wt1 = tmp.path().join("wt1");
        git.worktree_add(&repo, &wt1, "feature").unwrap();

        // Adding the same branch at a second path must fail like `git worktree add`.
        let wt2 = tmp.path().join("wt2");
        let err = git.worktree_add(&repo, &wt2, "feature").unwrap_err();
        assert!(
            err.to_string().contains("already checked out"),
            "err: {err}"
        );
    }

    #[test]
    fn git2_prune_skips_locked_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo_with_commit(&tmp);
        let git = Git2Ops::new();

        git.branch_create(&repo, "feature", "main").unwrap();
        let wt = tmp.path().join("wt-feature");
        git.worktree_add(&repo, &wt, "feature").unwrap();

        // Lock, then delete the dir: prune must keep the (locked) entry.
        let repo_handle = git2::Repository::open(&repo).unwrap();
        repo_handle
            .find_worktree("wt-feature")
            .unwrap()
            .lock(None)
            .unwrap();
        std::fs::remove_dir_all(&wt).unwrap();
        git.worktree_prune(&repo).unwrap();
        let entries = git.worktree_list(&repo).unwrap();
        assert_eq!(entries.len(), 2, "locked worktree must not be pruned");

        // Unlock -> prune now removes it.
        repo_handle
            .find_worktree("wt-feature")
            .unwrap()
            .unlock()
            .unwrap();
        git.worktree_prune(&repo).unwrap();
        let entries = git.worktree_list(&repo).unwrap();
        assert_eq!(entries.len(), 1, "unlocked stale worktree pruned");
    }
}
