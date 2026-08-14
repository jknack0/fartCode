//! Ship verb (pipeline overhaul): squash-merge a task branch into the
//! project-root checkout, commit, and push. Free functions over paths,
//! the commit.rs shape. A failed merge is cleaned up (`git reset
//! --merge`) before the typed error surfaces, so the root checkout never
//! sits half-merged. The frontend drives the surrounding flow (dirty
//! dialog → ship → board move → worktree-cleanup dialog).

use std::path::Path;

use fartcode_core::git::GitOps;
use fartcode_core::Error;
use serde::Serialize;

use crate::{git_cmd, output_bounded, CliGit, GitTimeout};

/// Result of [`squash_merge_and_push`] (camelCase → frontend).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShipOutcome {
    pub source_branch: String,
    pub target_branch: String,
    /// The squash commit on the target branch.
    pub merge_commit: String,
    /// False when the push remote is not configured (local-only repo) —
    /// the merge still happened.
    pub pushed: bool,
}

/// Squash-merges `branch` into the branch checked out at `root`, commits
/// with `message`, and pushes to `push_remote` when that remote exists.
///
/// Refusals (all typed `Error::Git`, no side effects left behind):
/// - detached HEAD at the root, or `branch` IS the root checkout;
/// - a dirty root checkout (the merge target must be clean);
/// - a conflicting merge (cleaned up with `git reset --merge`);
/// - a branch fully contained in the target (nothing to merge).
pub fn squash_merge_and_push(
    root: &Path,
    branch: &str,
    message: &str,
    push_remote: &str,
) -> Result<ShipOutcome, Error> {
    let target = CliGit
        .current_branch(root)?
        .ok_or_else(|| Error::Git("cannot ship: the project root is on a detached HEAD".into()))?;
    if target == branch {
        return Err(Error::Git(format!(
            "cannot ship: {branch} is the branch checked out at the project root"
        )));
    }
    let porcelain = run(root, &["status", "--porcelain"])?;
    if !porcelain.trim().is_empty() {
        return Err(Error::Git(format!(
            "cannot ship: the project root checkout ({target}) has uncommitted changes — \
             commit or stash them first"
        )));
    }
    if let Err(e) = run(root, &["merge", "--squash", branch]) {
        // Conflict (or unrelated histories): the squash left a half-merged
        // index/worktree — clean it before surfacing, best-effort.
        let _ = run(root, &["reset", "--merge"]);
        return Err(Error::Git(format!(
            "squash merge of {branch} into {target} failed — resolve on the branch and re-ship: {e}"
        )));
    }
    // `git diff --cached --quiet` exits 0 when NOTHING staged — the branch
    // is already contained in the target.
    if staged_is_empty(root)? {
        return Err(Error::Git(format!(
            "nothing to ship: {branch} has no changes beyond {target}"
        )));
    }
    let merge_commit = crate::commit::commit(root, message)?;
    let pushed = if CliGit.remotes(root)?.iter().any(|r| r == push_remote) {
        crate::commit::push(root, push_remote)?;
        true
    } else {
        false
    };
    Ok(ShipOutcome {
        source_branch: branch.to_string(),
        target_branch: target,
        merge_commit,
        pushed,
    })
}

/// `git diff --cached --quiet`: exit 0 = empty stage, 1 = staged changes;
/// anything else is a real failure.
fn staged_is_empty(root: &Path) -> Result<bool, Error> {
    let mut cmd = git_cmd(Some(root));
    cmd.args(["diff", "--cached", "--quiet"]);
    let output = output_bounded(cmd, GitTimeout::Local, "diff")?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(Error::Git(format!(
            "git diff --cached --quiet failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// Runs git in `root` capturing both streams; non-zero exit is
/// `Error::Git` carrying stderr (the commit.rs idiom).
fn run(root: &Path, args: &[&str]) -> Result<String, Error> {
    let mut cmd = git_cmd(Some(root));
    cmd.args(args);
    let label = args.first().copied().unwrap_or("command");
    let output = output_bounded(cmd, GitTimeout::Local, label)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        Ok(format!("{stdout}{stderr}"))
    } else {
        Err(Error::Git(format!(
            "git {label} failed: {}",
            stderr.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn init_repo(tmp: &tempfile::TempDir) -> PathBuf {
        let dir = tmp.path().join("repo");
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-b", "main"]);
        git(&dir, &["config", "user.email", "t@t"]);
        git(&dir, &["config", "user.name", "t"]);
        fs::write(dir.join("a.txt"), "base\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-m", "base"]);
        dir
    }

    #[test]
    fn squash_merges_a_branch_and_reports_no_push_without_a_remote() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = init_repo(&tmp);
        git(&dir, &["checkout", "-b", "feat"]);
        fs::write(dir.join("b.txt"), "feature\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-m", "feature"]);
        git(&dir, &["checkout", "main"]);

        let outcome = squash_merge_and_push(&dir, "feat", "Ship: feature", "origin").unwrap();
        assert_eq!(outcome.source_branch, "feat");
        assert_eq!(outcome.target_branch, "main");
        assert!(!outcome.pushed, "no remote configured");
        assert!(dir.join("b.txt").exists());
        // One squash commit, clean tree.
        let log = Command::new("git").current_dir(&dir).args(["log", "--oneline"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&log.stdout).lines().count(), 2);
        let status = Command::new("git").current_dir(&dir).args(["status", "--porcelain"]).output().unwrap();
        assert!(status.stdout.is_empty(), "root left clean");
    }

    #[test]
    fn conflict_is_typed_and_the_root_is_cleaned_up() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = init_repo(&tmp);
        git(&dir, &["checkout", "-b", "feat"]);
        fs::write(dir.join("a.txt"), "feature side\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-m", "feature"]);
        git(&dir, &["checkout", "main"]);
        fs::write(dir.join("a.txt"), "main side\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-m", "diverge"]);

        let err = squash_merge_and_push(&dir, "feat", "Ship: feature", "origin").unwrap_err();
        assert!(err.to_string().contains("failed"), "typed conflict: {err}");
        // Cleanup: no half-merged state survives.
        let status = Command::new("git").current_dir(&dir).args(["status", "--porcelain"]).output().unwrap();
        assert!(status.stdout.is_empty(), "reset --merge cleaned the root");
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "main side\n");
    }

    #[test]
    fn contained_branch_and_dirty_root_are_typed_refusals() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = init_repo(&tmp);
        git(&dir, &["branch", "noop"]);
        let err = squash_merge_and_push(&dir, "noop", "Ship: noop", "origin").unwrap_err();
        assert!(err.to_string().contains("nothing to ship"), "{err}");

        fs::write(dir.join("a.txt"), "dirty\n").unwrap();
        let err = squash_merge_and_push(&dir, "noop", "Ship: noop", "origin").unwrap_err();
        assert!(err.to_string().contains("uncommitted changes"), "{err}");

        // Shipping the checked-out branch itself is refused.
        git(&dir, &["checkout", "--", "a.txt"]);
        let err = squash_merge_and_push(&dir, "main", "Ship: main", "origin").unwrap_err();
        assert!(err.to_string().contains("checked out at the project root"), "{err}");
    }
}
