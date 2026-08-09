//! Footer git actions (E4-08, #48): fetch / pull / publish / add-remote —
//! the everyday verbs on the Changes-sidebar footer. Same shape as
//! `stage.rs`/`commit.rs`: free functions over a worktree path, git CLI
//! with the non-interactive env (`git_cmd`), failures surface, never hang
//! on credential prompts.
//!
//! Deliberate deviation from the reference (ticket): `pull` is `--ff-only`
//! — diverged history fails with git's own message instead of spawning a
//! merge.

use std::path::Path;

use fartcode_core::git::GitOps;
use fartcode_core::Error;
use serde::Serialize;

use crate::commit::PushOutcome;
use crate::{git_cmd, git_stdout_ok, output_bounded, CliGit, GitTimeout};

/// `git fetch <remote>` — refresh remote-tracking refs. Output is
/// suppressed (`-q`): fetch progress noise belongs to a terminal, not the
/// footer's error slot.
pub fn fetch(worktree: &Path, remote: &str) -> Result<(), Error> {
    run(worktree, &["fetch", "-q", remote], GitTimeout::Network).map(|_| ())
}

/// `git remote get-url <remote>` — `Ok(None)` when the remote doesn't
/// exist (nonzero exit), never an error for a missing remote.
pub fn remote_url(worktree: &Path, remote: &str) -> Option<String> {
    let out = git_stdout_ok(worktree, &["remote", "get-url", remote])?;
    let url = String::from_utf8_lossy(&out).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// Normalizes a git remote URL to a browsable GitHub URL — scp-style
/// (`git@github.com:owner/repo.git`), ssh://, and https:// inputs all map
/// to `https://github.com/owner/repo`. Non-GitHub hosts yield `None`
/// (the UI hides the affordance entirely).
pub fn github_https_url(remote_url: &str) -> Option<String> {
    let path = remote_url
        .strip_prefix("git@github.com:")
        .or_else(|| remote_url.strip_prefix("https://github.com/"))
        .or_else(|| remote_url.strip_prefix("http://github.com/"))
        .or_else(|| remote_url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    if path.is_empty() {
        return None;
    }
    Some(format!("https://github.com/{path}"))
}

/// `git pull --ff-only` (ticket: "ff-only default; surface non-ff as
/// error"). Refuses to merge diverged history — the footer shows git's
/// stderr verbatim ("Not possible to fast-forward, ...").
pub fn pull(worktree: &Path) -> Result<(), Error> {
    run(worktree, &["pull", "--ff-only"], GitTimeout::Network).map(|_| ())
}

/// Result of [`publish`] — the branch just published and its new upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishOutcome {
    pub branch: String,
    pub remote: String,
}

/// Publishes an unpublished branch: `push -u <remote> <branch>` (reference
/// `publishBranch`). Errors when HEAD is detached or the branch already
/// tracks an upstream — pushing in that case is `commit.rs::push`'s job,
/// and the footer hides Publish entirely once an upstream exists.
pub fn publish(worktree: &Path, remote: &str) -> Result<PublishOutcome, Error> {
    let branch = CliGit
        .current_branch(worktree)?
        .ok_or_else(|| Error::Git("cannot publish: HEAD is detached".into()))?;
    if upstream_of(worktree).is_some() {
        return Err(Error::Git(format!(
            "branch {branch} already has an upstream — use push"
        )));
    }
    crate::commit::push(worktree, remote)
        .map_err(|e| Error::Git(format!("publish failed: {}", e)))?;
    Ok(PublishOutcome {
        branch,
        remote: remote.to_string(),
    })
}

/// Republishes via [`publish`] or pushes via the existing upstream — the
/// footer's single "Push" verb routes here: unpublished → `-u` publish
/// path, published → plain `git push`. Returns which path ran.
pub fn push_or_publish(worktree: &Path, remote: &str) -> Result<PushOutcome, Error> {
    crate::commit::push(worktree, remote)
}

/// `git remote add <name> <url>` with validation: non-empty name matching
/// `[A-Za-z0-9._-]+` (git rejects most punctuation itself; we reject the
/// ambiguous set up front), no duplicate remote names, non-empty URL.
pub fn add_remote(worktree: &Path, name: &str, url: &str) -> Result<(), Error> {
    let name = name.trim();
    let url = url.trim();
    if name.is_empty() {
        return Err(Error::Git("remote name is empty".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(Error::Git(format!(
            "invalid remote name {name:?} (letters, digits, '.', '_', '-' only)"
        )));
    }
    if url.is_empty() {
        return Err(Error::Git("remote URL is empty".into()));
    }
    let existing = CliGit.remotes(worktree)?;
    if existing.iter().any(|r| r == name) {
        return Err(Error::Git(format!("remote {name:?} already exists")));
    }
    // Local: `remote add` only writes .git/config — no wire traffic.
    run(worktree, &["remote", "add", name, url], GitTimeout::Local).map(|_| ())
}

/// Current upstream shorthand (`git rev-parse --abbrev-ref @{upstream}`),
/// e.g. `origin/feat-x`. `None` when the branch tracks nothing (or HEAD is
/// detached — the rev-parse failure encodes that).
pub fn upstream_of(worktree: &Path) -> Option<String> {
    git_stdout_ok(worktree, &["rev-parse", "--abbrev-ref", "@{upstream}"]).and_then(|raw| {
        let s = String::from_utf8_lossy(&raw);
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_string())
    })
}

/// `(ahead, behind)` commit counts vs the upstream. `(0, 0)` when there is
/// no upstream (unpublished branch) — the footer renders counts only with
/// one. Reference: `rev-list --count @{upstream}..HEAD` and its mirror.
pub fn ahead_behind(worktree: &Path) -> (u32, u32) {
    let ahead = rev_list_count(worktree, "@{upstream}..HEAD");
    let behind = rev_list_count(worktree, "HEAD..@{upstream}");
    (ahead, behind)
}

fn rev_list_count(worktree: &Path, range: &str) -> u32 {
    git_stdout_ok(worktree, &["rev-list", "--count", range])
        .and_then(|raw| String::from_utf8_lossy(&raw).trim().parse::<u32>().ok())
        .unwrap_or(0)
}

/// Runs git capturing both streams (fetch/pull print progress to stderr);
/// non-zero exit → `Error::Git` with the trimmed stderr.
///
/// `class` bounds the wait (#80). This was a bare `Command::output()`: an
/// unreachable remote — a VPN that dropped, a host that black-holes SYNs —
/// wedged the call forever, and with it the command that called it.
fn run(worktree: &Path, args: &[&str], class: GitTimeout) -> Result<String, Error> {
    let mut cmd = git_cmd(Some(worktree));
    cmd.args(args);
    let output = output_bounded(cmd, class, args.first().copied().unwrap_or("command"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        Ok(format!("{stdout}{stderr}"))
    } else {
        Err(Error::Git(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or("?"),
            stderr.trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::tests::{git, repo_fixture};

    /// Worktree + bare sibling remote wired as `origin`, branch pushed.
    fn remote_fixture() -> (tempfile::TempDir, tempfile::TempDir) {
        let repo = repo_fixture();
        let bare = tempfile::tempdir().unwrap();
        git(bare.path(), &["init", "--bare"]);
        let bare_url = bare.path().to_string_lossy().into_owned();
        git(repo.path(), &["remote", "add", "origin", bare_url.as_str()]);
        (repo, bare)
    }

    fn branch_name(repo: &tempfile::TempDir) -> String {
        CliGit.current_branch(repo.path()).unwrap().unwrap()
    }

    #[test]
    fn publish_sets_upstream_and_second_publish_errors() {
        let (repo, _bare) = remote_fixture();
        let branch = branch_name(&repo);
        assert!(upstream_of(repo.path()).is_none());

        let out = publish(repo.path(), "origin").unwrap();
        assert_eq!(out.branch, branch);
        assert_eq!(
            upstream_of(repo.path()).as_deref(),
            Some(&*format!("origin/{branch}"))
        );

        // Already published → publish refuses (footer hides the button).
        let err = publish(repo.path(), "origin").unwrap_err();
        assert!(err.to_string().contains("already has an upstream"));
    }

    #[test]
    fn ahead_behind_counts_and_ff_pull() {
        let (repo, bare) = remote_fixture();
        publish(repo.path(), "origin").unwrap();
        let branch = branch_name(&repo);

        // Local-only commit → ahead 1, behind 0.
        std::fs::write(repo.path().join("tracked.txt"), "local\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "local ahead"]);
        assert_eq!(ahead_behind(repo.path()), (1, 0));

        // Advance the remote via a clone, then fetch → behind 1.
        let clone = tempfile::tempdir().unwrap();
        git(
            clone.path(),
            &["clone", bare.path().to_string_lossy().as_ref(), "c"],
        );
        let cl = clone.path().join("c");
        std::fs::write(cl.join("remote-only.txt"), "r\n").unwrap();
        git(&cl, &["add", "."]);
        git(&cl, &["commit", "-m", "remote ahead"]);
        git(&cl, &["push"]);

        fetch(repo.path(), "origin").unwrap();
        assert_eq!(ahead_behind(repo.path()), (1, 1));

        // Diverged pull fails cleanly with the ff-only message.
        let err = pull(repo.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("fast-forward") || msg.contains("divergent"),
            "unexpected pull error: {msg}"
        );

        // Rebase local on top → pull succeeds (ff).
        git(repo.path(), &["rebase", &format!("origin/{branch}")]);
        pull(repo.path()).unwrap();
        assert_eq!(ahead_behind(repo.path()), (1, 0));

        // Push the remaining commit and land at zero/zero.
        push_or_publish(repo.path(), "origin").unwrap();
        assert_eq!(ahead_behind(repo.path()), (0, 0));
    }

    #[test]
    fn pull_without_upstream_errors() {
        let repo = repo_fixture();
        let err = pull(repo.path()).unwrap_err();
        assert!(err.to_string().contains("pull"));
    }

    #[test]
    fn fetch_without_remote_errors() {
        let repo = repo_fixture();
        assert!(fetch(repo.path(), "origin").is_err());
    }

    #[test]
    fn add_remote_validates_and_adds() {
        let repo = repo_fixture();
        // Validation rejections.
        assert!(add_remote(repo.path(), "", "u").is_err());
        assert!(add_remote(repo.path(), "bad name", "u").is_err());
        assert!(add_remote(repo.path(), "ok", " ").is_err());

        add_remote(repo.path(), "upstream", "/tmp/elsewhere.git").unwrap();
        let remotes = CliGit.remotes(repo.path()).unwrap();
        assert!(remotes.contains(&"upstream".to_string()));

        // Duplicate rejected.
        assert!(add_remote(repo.path(), "upstream", "/x").is_err());
    }

    #[test]
    fn publish_detached_head_errors() {
        let (repo, _bare) = remote_fixture();
        let head = git_stdout_ok(repo.path(), &["rev-parse", "HEAD"])
            .map(|r| String::from_utf8_lossy(&r).trim().to_string())
            .unwrap();
        git(repo.path(), &["checkout", "--detach", head.as_str()]);
        let err = publish(repo.path(), "origin").unwrap_err();
        assert!(err.to_string().contains("detached"));
    }

    #[test]
    fn github_https_url_normalizes_remote_shapes() {
        assert_eq!(
            github_https_url("git@github.com:jknack0/fartCode.git"),
            Some("https://github.com/jknack0/fartCode".to_string())
        );
        assert_eq!(
            github_https_url("https://github.com/jknack0/fartCode.git"),
            Some("https://github.com/jknack0/fartCode".to_string())
        );
        assert_eq!(
            github_https_url("ssh://git@github.com/jknack0/fartCode"),
            Some("https://github.com/jknack0/fartCode".to_string())
        );
        assert_eq!(
            github_https_url("https://github.com/jknack0/fartCode/"),
            Some("https://github.com/jknack0/fartCode".to_string())
        );
        // Non-GitHub hosts hide the affordance.
        assert_eq!(
            github_https_url("git@gitlab.com:jknack0/fartCode.git"),
            None
        );
        assert_eq!(github_https_url("https://example.com/x/y"), None);
    }

    #[test]
    fn remote_url_missing_remote_is_none() {
        let (repo, _bare) = remote_fixture();
        assert!(remote_url(repo.path(), "no-such-remote").is_none());
        assert!(remote_url(repo.path(), "origin").is_some());
    }
}
