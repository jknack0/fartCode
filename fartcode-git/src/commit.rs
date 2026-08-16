//! Commit-card backend (E4-06, #46): commit/push mutations + the repo-state
//! DTO that drives the card's disabled-state matrix and the PR-open guard.
//!
//! Free functions over a worktree path, same shape as `stage.rs` (E4-03).
//! The PR lookup is behind the [`PrLookup`] trait: the app wires
//! `fartcode_git::pr_sync::CachedPrLookup` (the E4-09 sync cache — local,
//! offline-safe); tests use fakes. The guard logic and its UI wiring are
//! testable without a network.

use std::path::Path;

use fartcode_core::git::GitOps;
use fartcode_core::Error;
use serde::Serialize;

use crate::{git_cmd, git_stdout_ok, output_bounded, CliGit, GitTimeout};

/// Open-PR lookup for one branch (the commit card's PR-open guard).
///
/// `None` = no open PR for the branch. Implementations must not hang
/// (local cache reads only; failures surface as `Err`, which the card
/// treats as "guard unknown → offer push only").
pub trait PrLookup: Send + Sync {
    fn pr_url(&self, repo: &Path, remote: &str, branch: &str) -> Result<Option<String>, Error>;
}

/// Phase 0 stand-in: no PR engine yet (E4-07/E8), so no branch ever has an
/// open PR. Kept as a named type (not a closure) so the Tauri command reads
/// like the reference's injection point.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubPrLookup;

impl PrLookup for StubPrLookup {
    fn pr_url(&self, _repo: &Path, _remote: &str, _branch: &str) -> Result<Option<String>, Error> {
        Ok(None)
    }
}

/// Repo-state snapshot feeding the commit card (camelCase → frontend).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitState {
    /// Checked-out branch (`None` = detached HEAD — commits disabled).
    pub branch: Option<String>,
    /// The push remote name, when it exists in `git remote`.
    pub remote: Option<String>,
    /// The push remote actually exists in `git remote`.
    pub has_remote: bool,
    /// `remote/branch` exists locally (branch was pushed at least once).
    pub published: bool,
    /// An open PR exists for the branch (per [`PrLookup`]).
    pub pr_open: bool,
    /// "Commit & Create PR" is offered: branch + configured remote + no
    /// open PR. (E8 supplies the repository URL; Phase 0 assumes it.)
    pub can_create_pr: bool,
    /// Upstream shorthand (`origin/feat-x`) when the branch tracks one —
    /// rendered with `ahead`/`behind` as the footer hint line's `↑n ↓n
    /// <upstream>` sync segment (#133); the pull/push verbs themselves
    /// moved to the ⌘K palette.
    pub upstream: Option<String>,
    /// Commits ahead of / behind the upstream (0/0 without one).
    pub ahead: u32,
    pub behind: u32,
    /// All configured remotes (origin first — `CliGit::remotes`).
    pub remotes: Vec<String>,
}

/// Collects [`CommitState`] for `worktree`. `push_remote` is the project
/// settings' effective pushRemote; `lookup` answers the PR-open guard.
pub fn state(
    worktree: &Path,
    push_remote: &str,
    lookup: &dyn PrLookup,
) -> Result<CommitState, Error> {
    let cli = CliGit;
    let branch = cli.current_branch(worktree)?;
    let remotes = cli.remotes(worktree)?;
    let has_remote = remotes.iter().any(|r| r == push_remote);

    let published = branch.as_deref().is_some_and(|b| {
        git_stdout_ok(
            worktree,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{push_remote}/{b}"),
            ],
        )
        .is_some()
    });

    // Guard failures degrade to "no open PR info" rather than failing the
    // whole state read — the card must still render commit/push affordances.
    let pr_url = branch
        .as_deref()
        .map(|b| lookup.pr_url(worktree, push_remote, b))
        .transpose()
        .ok()
        .flatten()
        .flatten();
    let pr_open = pr_url.is_some();
    let can_create_pr = branch.is_some() && has_remote && !pr_open;

    let upstream = crate::remote::upstream_of(worktree);
    let (ahead, behind) = crate::remote::ahead_behind(worktree);

    Ok(CommitState {
        branch,
        remote: has_remote.then(|| push_remote.to_string()),
        has_remote,
        published,
        pr_open,
        can_create_pr,
        upstream,
        ahead,
        behind,
        remotes,
    })
}

/// `git commit -m <message>` over the staged set, then `rev-parse HEAD` —
/// returns the new commit hash. Empty/whitespace messages are rejected
/// before spawning git (reference commit-card guard).
pub fn commit(worktree: &Path, message: &str) -> Result<String, Error> {
    if message.trim().is_empty() {
        return Err(Error::Git("commit message is empty".into()));
    }
    // Extended: `git commit` runs the repo's pre-commit/commit-msg hooks,
    // whose runtime belongs to the user, not to us.
    run_timed(worktree, &["commit", "-m", message], GitTimeout::Extended)?;
    let hash = run(worktree, &["rev-parse", "HEAD"])?;
    Ok(hash.trim().to_string())
}

/// Result of [`push`] — tells the UI which path ran and any PR URL the
/// remote advertised (GitHub prints a "Create a pull request" link on first
/// push).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushOutcome {
    pub branch: String,
    pub remote: String,
    /// First push (no upstream yet) used `-u`; subsequent pushes used the
    /// configured upstream via plain `git push`.
    pub set_upstream: bool,
    /// First `https://` URL found in the push output, if any.
    pub pr_url: Option<String>,
}

/// Pushes the current branch to `push_remote`, setting upstream on first
/// push (ticket: "set-upstream on first push"). Published branches push
/// with plain `git push` (uses the configured upstream), new branches with
/// `git push -u <remote> <branch>` (reference publishBranch).
pub fn push(worktree: &Path, push_remote: &str) -> Result<PushOutcome, Error> {
    let branch = CliGit
        .current_branch(worktree)?
        .ok_or_else(|| Error::Git("nothing to push: HEAD is detached".into()))?;

    let has_upstream = git_stdout_ok(worktree, &["rev-parse", "--abbrev-ref", "@{upstream}"])
        .is_some_and(|raw| !String::from_utf8_lossy(&raw).trim().is_empty());

    let output = if has_upstream {
        run_timed(worktree, &["push"], GitTimeout::Network)?
    } else {
        run_timed(
            worktree,
            &["push", "-u", push_remote, &branch],
            GitTimeout::Network,
        )?
    };

    Ok(PushOutcome {
        branch,
        remote: push_remote.to_string(),
        set_upstream: !has_upstream,
        pr_url: extract_url(&output),
    })
}

/// Result of the create-PR flow — the URL the UI opens in the browser,
/// whether the branch had to be published first, and whether the PR was
/// actually created via the GitHub API (#132) or the URL is only the
/// browser compare form (no token — the pre-#132 fallback).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePrOutcome {
    pub url: String,
    pub pushed: bool,
    pub created: bool,
}

/// Everything the create-PR flow needs from git before it may talk to the
/// GitHub API (#132): PR-open guard applied, branch published, remote URL,
/// and the HEAD commit's message for the PR title/body. Blocking (git
/// subprocesses) — callers keep it off the main thread.
#[derive(Debug, Clone)]
pub struct PrPrep {
    pub branch: String,
    pub pushed: bool,
    pub remote_url: String,
    /// HEAD commit subject — the PR title (empty falls back to the branch).
    pub title: String,
    /// HEAD commit body — the PR description.
    pub body: String,
}

/// Git half of "Commit, push & open PR" (E4-06 → #132): ensures the
/// branch is published (pushes first when it isn't — ticket: "pushes
/// first when the branch isn't on the remote"), refuses when the PR-open
/// guard says a PR already exists, and collects what the GitHub API call
/// needs (remote URL, HEAD message).
pub fn prepare_pr(
    worktree: &Path,
    push_remote: &str,
    lookup: &dyn PrLookup,
) -> Result<PrPrep, Error> {
    let branch = CliGit
        .current_branch(worktree)?
        .ok_or_else(|| Error::Git("cannot create a PR from a detached HEAD".into()))?;

    // PR-open guard (reference: the action degrades to push when a PR is
    // already open). Guard failures degrade to "proceed" — same policy as
    // the state read.
    if let Some(existing) = lookup.pr_url(worktree, push_remote, &branch).ok().flatten() {
        return Err(Error::Git(format!(
            "a pull request is already open: {existing}"
        )));
    }

    let published = git_stdout_ok(
        worktree,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{push_remote}/{branch}"),
        ],
    )
    .is_some();
    let pushed = if published {
        false
    } else {
        push(worktree, push_remote)?;
        true
    };

    let remote_url = run(worktree, &["remote", "get-url", push_remote])?
        .trim()
        .to_string();
    // HEAD commit message → PR title/body (what `gh pr create --fill` does).
    let message = run(worktree, &["log", "-1", "--format=%B"])?;
    let (title, body) = split_head_message(&message);

    Ok(PrPrep {
        branch,
        pushed,
        remote_url,
        title,
        body,
    })
}

/// `%B` head message → (subject, body).
fn split_head_message(message: &str) -> (String, String) {
    let mut parts = message.trim().splitn(2, '\n');
    let title = parts.next().unwrap_or("").trim().to_string();
    let body = parts.next().unwrap_or("").trim().to_string();
    (title, body)
}

/// The no-token fallback (and the whole pre-#132 story): maps the prep to
/// the GitHub compare form URL — the browser does the last step.
pub fn compare_outcome(prep: &PrPrep) -> Result<CreatePrOutcome, Error> {
    let url = github_compare_url(&prep.remote_url, &prep.branch).ok_or_else(|| {
        Error::Git(
            "PR creation supports GitHub remotes only in Phase 0 (E8 wires other hosts)".into(),
        )
    })?;
    Ok(CreatePrOutcome {
        url,
        pushed: prep.pushed,
        created: false,
    })
}

/// [`prepare_pr`] + [`compare_outcome`]: the create-PR flow without a
/// GitHub token. The API path is `crate::pr_sync::create_pr_on_github`
/// (#132); it shares the same prep.
pub fn create_pr(
    worktree: &Path,
    push_remote: &str,
    lookup: &dyn PrLookup,
) -> Result<CreatePrOutcome, Error> {
    let prep = prepare_pr(worktree, push_remote, lookup)?;
    compare_outcome(&prep)
}

/// Maps a GitHub remote URL (ssh or https) to
/// `https://github.com/<owner>/<repo>/compare/<branch>?expand=1`.
fn github_compare_url(remote_url: &str, branch: &str) -> Option<String> {
    let slug = if let Some(rest) = remote_url.strip_prefix("git@github.com:") {
        rest
    } else {
        remote_url
            .strip_prefix("https://github.com/")
            .or_else(|| remote_url.strip_prefix("http://github.com/"))?
    };
    let slug = slug.strip_suffix(".git").unwrap_or(slug);
    let mut parts = slug.splitn(3, '/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!(
        "https://github.com/{owner}/{repo}/compare/{branch}?expand=1"
    ))
}

/// Runs git in `worktree` capturing BOTH streams: remotes print PR URLs to
/// stderr on push, and commit hints land there too. Non-zero exit is
/// `Error::Git` carrying stderr. Local class — see [`run_timed`] for the
/// calls that are not local.
fn run(worktree: &Path, args: &[&str]) -> Result<String, Error> {
    run_timed(worktree, args, GitTimeout::Local)
}

/// [`run`] with an explicit timeout class (#80): `push` talks to a remote,
/// `commit` runs the repo's own hooks, `rev-parse` is pure disk.
fn run_timed(worktree: &Path, args: &[&str], class: GitTimeout) -> Result<String, Error> {
    let mut cmd = git_cmd(Some(worktree));
    cmd.args(args);
    let label = args.first().copied().unwrap_or("command");
    let output = output_bounded(cmd, class, label)?;
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

/// First `https://` URL in `text` (remote push hints), token-split so
/// trailing punctuation doesn't leak in.
fn extract_url(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|tok| {
        let tok = tok.trim_matches(|c: char| matches!(c, '(' | ')' | '\'' | '"' | ':' | ','));
        tok.starts_with("https://").then(|| tok.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status;
    use crate::status::tests::{git, repo_fixture};

    /// Worktree + bare sibling remote wired as `origin`.
    fn remote_fixture() -> (tempfile::TempDir, tempfile::TempDir) {
        let repo = repo_fixture();
        let bare = tempfile::tempdir().unwrap();
        git(bare.path(), &["init", "--bare"]);
        let bare_url = bare.path().to_string_lossy().into_owned();
        git(repo.path(), &["remote", "add", "origin", bare_url.as_str()]);
        (repo, bare)
    }

    struct FakeLookup(Option<String>);
    impl PrLookup for FakeLookup {
        fn pr_url(
            &self,
            _repo: &Path,
            _remote: &str,
            _branch: &str,
        ) -> Result<Option<String>, Error> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn commit_commits_exactly_the_staged_set() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("tracked.txt"), "staged change\n").unwrap();
        std::fs::write(repo.path().join("untracked.txt"), "x\n").unwrap();
        // Stage only tracked.txt; the untracked file must stay out.
        git(repo.path(), &["add", "tracked.txt"]);

        let hash = commit(repo.path(), "wip").unwrap();
        let head = run(repo.path(), &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(hash, head.trim());

        let snap = status::status(repo.path()).unwrap();
        assert!(snap.staged.is_empty());
        assert_eq!(snap.unstaged.len(), 1);
        assert_eq!(snap.unstaged[0].path, "untracked.txt");
    }

    #[test]
    fn commit_empty_message_rejected_before_spawn() {
        let repo = repo_fixture();
        let err = commit(repo.path(), "   ").unwrap_err();
        assert!(err.to_string().contains("commit message is empty"));
    }

    #[test]
    fn commit_nothing_staged_surfaces_git_error() {
        let repo = repo_fixture();
        assert!(commit(repo.path(), "no-op").is_err());
    }

    #[test]
    fn state_reports_unpublished_branch_and_remote() {
        let (repo, _bare) = remote_fixture();
        let st = state(repo.path(), "origin", &StubPrLookup).unwrap();
        assert!(st.branch.is_some());
        assert!(st.has_remote);
        assert_eq!(st.remote.as_deref(), Some("origin"));
        assert!(!st.published);
        assert!(!st.pr_open);
        assert!(st.can_create_pr);
    }

    #[test]
    fn state_no_remote_configured() {
        let repo = repo_fixture();
        let st = state(repo.path(), "origin", &StubPrLookup).unwrap();
        assert!(!st.has_remote);
        assert!(st.remote.is_none());
        assert!(!st.published);
        assert!(!st.can_create_pr);
    }

    #[test]
    fn push_first_time_sets_upstream_second_uses_default() {
        let (repo, _bare) = remote_fixture();
        let branch = state(repo.path(), "origin", &StubPrLookup)
            .unwrap()
            .branch
            .unwrap();

        let out1 = push(repo.path(), "origin").unwrap();
        assert!(out1.set_upstream);
        assert_eq!(out1.branch, branch);
        // Upstream now configured on the branch.
        assert!(
            git_stdout_ok(repo.path(), &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_some()
        );
        // Remote actually received it.
        assert!(git_stdout_ok(
            repo.path(),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("origin/{branch}")
            ]
        )
        .is_some());

        // Second push: plain-push path (upstream configured).
        std::fs::write(repo.path().join("tracked.txt"), "second\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "second"]);
        let out2 = push(repo.path(), "origin").unwrap();
        assert!(!out2.set_upstream);
    }

    #[test]
    fn push_without_remote_errors() {
        let repo = repo_fixture();
        assert!(push(repo.path(), "origin").is_err());
    }

    #[test]
    fn pr_open_guard_via_mocked_lookup() {
        let (repo, _bare) = remote_fixture();
        // Open PR → guard flips: no create-PR affordance.
        let st = state(
            repo.path(),
            "origin",
            &FakeLookup(Some("https://github.com/x/pull/1".into())),
        )
        .unwrap();
        assert!(st.pr_open);
        assert!(!st.can_create_pr);
        // No PR → affordance back.
        let st = state(repo.path(), "origin", &FakeLookup(None)).unwrap();
        assert!(!st.pr_open);
        assert!(st.can_create_pr);
    }

    #[test]
    fn extract_url_finds_remote_pr_hint() {
        let text = "remote:\nremote: Create a pull request for 'x' on GitHub by visiting:\nremote:      https://github.com/o/r/pull/new/b\nremote:\nTo github.com:o/r.git";
        assert_eq!(
            extract_url(text).as_deref(),
            Some("https://github.com/o/r/pull/new/b")
        );
        assert_eq!(extract_url("nothing here"), None);
    }

    #[test]
    fn github_compare_url_maps_ssh_and_https() {
        assert_eq!(
            github_compare_url("git@github.com:o/r.git", "feat/x").as_deref(),
            Some("https://github.com/o/r/compare/feat/x?expand=1")
        );
        assert_eq!(
            github_compare_url("https://github.com/o/r", "b").as_deref(),
            Some("https://github.com/o/r/compare/b?expand=1")
        );
        assert_eq!(github_compare_url("https://gitlab.com/o/r.git", "b"), None);
        assert_eq!(github_compare_url("/local/bare/path", "b"), None);
    }

    #[test]
    fn create_pr_pushes_unpublished_branch_first() {
        // Unpublished + local remote: the push happens first (ticket:
        // "pushes first when the branch isn't on the remote"), then URL
        // mapping fails cleanly because the remote isn't GitHub.
        let (repo, _bare) = remote_fixture();
        let branch = state(repo.path(), "origin", &StubPrLookup)
            .unwrap()
            .branch
            .unwrap();
        let err = create_pr(repo.path(), "origin", &StubPrLookup).unwrap_err();
        assert!(err.to_string().contains("GitHub"));
        // Push-first ordering proven: the branch landed on the remote.
        assert!(git_stdout_ok(
            repo.path(),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("origin/{branch}")
            ]
        )
        .is_some());
    }

    #[test]

    fn create_pr_published_branch_returns_url_without_pushing() {
        let (repo, _bare) = remote_fixture();
        push(repo.path(), "origin").unwrap(); // published via local remote
        git(
            repo.path(),
            &["remote", "set-url", "origin", "git@github.com:o/r.git"],
        );

        let out = create_pr(repo.path(), "origin", &StubPrLookup).unwrap();
        assert!(!out.pushed);
        assert!(!out.created);
        let branch = state(repo.path(), "origin", &StubPrLookup)
            .unwrap()
            .branch
            .unwrap();
        assert_eq!(
            out.url,
            format!("https://github.com/o/r/compare/{branch}?expand=1")
        );
    }

    #[test]
    fn prepare_pr_carries_head_message_as_title_and_body() {
        let (repo, _bare) = remote_fixture();
        std::fs::write(repo.path().join("tracked.txt"), "widget\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "Add widget\n\nLong body here"]);

        let prep = prepare_pr(repo.path(), "origin", &StubPrLookup).unwrap();
        assert!(prep.pushed); // unpublished branch → pushed first
        assert_eq!(prep.title, "Add widget");
        assert_eq!(prep.body, "Long body here");
        assert!(prep.remote_url.contains("/"));

        assert_eq!(
            split_head_message("only subject\n"),
            ("only subject".to_string(), String::new())
        );
    }

    #[test]
    fn create_pr_guard_blocks_when_pr_open() {
        let (repo, _bare) = remote_fixture();
        git(
            repo.path(),
            &["remote", "set-url", "origin", "git@github.com:o/r.git"],
        );
        let err = create_pr(
            repo.path(),
            "origin",
            &FakeLookup(Some("https://github.com/o/r/pull/9".into())),
        )
        .unwrap_err();
        assert!(err.to_string().contains("already open"));
        assert!(err.to_string().contains("pull/9"));
    }
}
