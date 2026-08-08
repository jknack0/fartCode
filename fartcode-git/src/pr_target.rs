//! PR target resolution (E4-07, #47): worktree → `(owner, repo, branch)`
//! for GitHub API calls. Reads the push remote's URL and the checked-out
//! branch with the git CLI; non-GitHub remotes and detached HEADs resolve
//! to `None` (the UI renders the matching empty state).

use std::path::Path;

use fartcode_core::git::GitOps;
use fartcode_core::github::parse_github_slug;
use fartcode_core::Error;
use serde::Serialize;

use crate::{remote, CliGit};

/// Where to point the PR query for a worktree.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrTarget {
    pub owner: String,
    pub repo: String,
    /// `owner/repo` — the empty-state label.
    pub repository: String,
    pub branch: String,
}

/// Resolves the PR target for `worktree` against `push_remote` (falling
/// back to `origin` when that remote is missing). `None` when the repo has
/// no usable GitHub remote or HEAD is detached/unborn.
pub fn resolve_pr_target(worktree: &Path, push_remote: &str) -> Result<Option<PrTarget>, Error> {
    let cli = CliGit;
    let Some(branch) = cli.current_branch(worktree)? else {
        return Ok(None);
    };
    let remotes = cli.remotes(worktree)?;
    let remote_name = if remotes.iter().any(|r| r == push_remote) {
        push_remote
    } else if remotes.iter().any(|r| r == "origin") {
        "origin"
    } else {
        return Ok(None);
    };
    let Some(url) = remote::remote_url(worktree, remote_name) else {
        return Ok(None);
    };
    let Some((owner, repo)) = parse_github_slug(&url) else {
        return Ok(None);
    };
    Ok(Some(PrTarget {
        repository: format!("{owner}/{repo}"),
        owner,
        repo,
        branch,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::tests::{git, repo_fixture};

    #[test]
    fn resolves_github_origin() {
        let repo = repo_fixture();
        git(
            repo.path(),
            &["remote", "add", "origin", "git@github.com:o/r.git"],
        );
        let target = resolve_pr_target(repo.path(), "origin").unwrap().unwrap();
        assert_eq!(target.owner, "o");
        assert_eq!(target.repo, "r");
        assert_eq!(target.repository, "o/r");
        assert!(!target.branch.is_empty());
    }

    #[test]
    fn falls_back_to_origin_when_push_remote_missing() {
        let repo = repo_fixture();
        git(
            repo.path(),
            &["remote", "add", "origin", "https://github.com/o/r.git"],
        );
        let target = resolve_pr_target(repo.path(), "upstream").unwrap();
        assert!(target.is_some());
    }

    #[test]
    fn non_github_remote_is_none() {
        let repo = repo_fixture();
        git(
            repo.path(),
            &["remote", "add", "origin", "git@gitlab.com:o/r.git"],
        );
        assert!(resolve_pr_target(repo.path(), "origin").unwrap().is_none());
    }

    #[test]
    fn no_remote_is_none() {
        let repo = repo_fixture();
        assert!(resolve_pr_target(repo.path(), "origin").unwrap().is_none());
    }

    #[test]
    fn detached_head_is_none() {
        let repo = repo_fixture();
        git(
            repo.path(),
            &["remote", "add", "origin", "git@github.com:o/r.git"],
        );
        let head = String::from_utf8_lossy(
            &std::process::Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .to_string();
        git(repo.path(), &["checkout", "--detach", &head]);
        assert!(resolve_pr_target(repo.path(), "origin").unwrap().is_none());
    }
}
