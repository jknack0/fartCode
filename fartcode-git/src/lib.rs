//! fartcode-git — git operations (ARCHITECTURE.md §6.4).
//!
//! `CliGit` shells out to the `git` CLI via `std::process::Command` with
//! argument arrays — no shell, no quoting (AGENTS.md: "no ad-hoc shell
//! quoting"). `Git2Ops` provides git2-backed worktree operations (E2-02
//! git strategy) and delegates everything else to `CliGit`. The `GitOps`
//! trait itself lives in `fartcode-core` so domain crates can use it without
//! violating the leaf rule; this crate provides the implementations and
//! re-exports the trait.

pub mod commit;
pub mod diff;
pub mod git2ops;
pub mod issues;
pub mod pr_sync;
pub mod pr_target;
pub mod remote;
pub mod stage;
pub mod status;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub use fartcode_core::git::{BranchRef, GitOps, WorktreeEntry};
use fartcode_core::Error;
pub use git2ops::Git2Ops;

/// git CLI-backed implementation of `GitOps`.
#[derive(Debug)]
pub struct CliGit;

/// Reference `NON_INTERACTIVE_GIT_ENV`: suppress all git prompts so cleanup /
/// background ops can never hang on credential dialogs.
const NON_INTERACTIVE_ENV: &[(&str, &str)] = &[
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_ASKPASS", ""),
    ("GCM_INTERACTIVE", "never"),
    ("SSH_ASKPASS", ""),
];

pub(crate) fn git_cmd(path: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    for (k, v) in NON_INTERACTIVE_ENV {
        cmd.env(k, v);
    }
    if let Some(path) = path {
        cmd.arg("-C").arg(path);
    }
    cmd
}

/// How long a git subprocess may run before it is killed (#80).
///
/// `Command::output()` has no upper bound: `remote::run` could wait forever
/// on an unreachable remote, and the caller — a Tauri command — never
/// returned. Every git invocation in this crate now goes through
/// [`output_bounded`], which kills the child at its deadline and returns
/// [`Error::GitTimeout`], a typed error the UI renders like any other git
/// failure.
///
/// Defaults and their overrides (whole seconds; `0` disables the bound
/// entirely, restoring the old unbounded wait):
///
/// | class      | default | env var                               |
/// |------------|---------|---------------------------------------|
/// | `Local`    | 30s     | `FARTCODE_GIT_TIMEOUT_LOCAL_SECS`     |
/// | `Network`  | 60s     | `FARTCODE_GIT_TIMEOUT_NETWORK_SECS`   |
/// | `Extended` | 600s    | `FARTCODE_GIT_TIMEOUT_EXTENDED_SECS`  |
///
/// The three classes exist because one number cannot be both honest and
/// safe: a `status` that takes 30s is broken, a `push` that takes 30s is
/// merely on hotel wifi, and a `commit` that takes 30s is running the
/// repo's own pre-commit hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitTimeout {
    /// Disk-only work — status, diff, add, restore, rev-parse. Finishes in
    /// milliseconds; the bound only fires on a wedged filesystem or an
    /// index lock held by something else.
    Local,
    /// The wire — fetch/pull/push/publish. Generous enough for a slow link,
    /// short enough that a black-holed remote becomes an error rather than
    /// a spinner that never stops.
    Network,
    /// Work whose honest duration is defined by the user's repo: `commit`
    /// (runs pre-commit/commit-msg hooks) and `clone` (arbitrarily large
    /// transfers). The bound is a backstop, not a policy.
    Extended,
}

impl GitTimeout {
    const fn env_var(self) -> &'static str {
        match self {
            Self::Local => "FARTCODE_GIT_TIMEOUT_LOCAL_SECS",
            Self::Network => "FARTCODE_GIT_TIMEOUT_NETWORK_SECS",
            Self::Extended => "FARTCODE_GIT_TIMEOUT_EXTENDED_SECS",
        }
    }

    const fn default_duration(self) -> Duration {
        match self {
            Self::Local => Duration::from_secs(30),
            Self::Network => Duration::from_secs(60),
            Self::Extended => Duration::from_secs(600),
        }
    }

    /// Resolved bound. `None` means unbounded (an explicit `0` override).
    pub fn duration(self) -> Option<Duration> {
        parse_timeout_secs(
            std::env::var(self.env_var()).ok().as_deref(),
            self.default_duration(),
        )
    }
}

/// Parses a timeout override. Unset/empty/garbage keeps `default` — a
/// typo must never silently restore the unbounded wait; only a literal `0`
/// does, and that is the documented escape hatch.
fn parse_timeout_secs(raw: Option<&str>, default: Duration) -> Option<Duration> {
    match raw.map(str::trim) {
        None | Some("") => Some(default),
        Some(v) => match v.parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(_) => Some(default),
        },
    }
}

/// Longest gap between `try_wait` polls. Starts far shorter (see
/// [`output_within`]) so a 5ms `rev-parse` is not rounded up to a poll tick.
const MAX_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Runs `cmd` capturing both streams, killing it if it outlives its class's
/// bound. Same result shape as `Command::output()`; the only new outcome is
/// [`Error::GitTimeout`].
pub(crate) fn output_bounded(
    cmd: Command,
    class: GitTimeout,
    label: &str,
) -> Result<Output, Error> {
    output_within(cmd, class.duration(), label)
}

/// [`output_bounded`] with an explicit bound (`None` = wait forever).
pub(crate) fn output_within(
    mut cmd: Command,
    timeout: Option<Duration>,
    label: &str,
) -> Result<Output, Error> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Git(format!("failed to spawn git: {e}")))?;

    // Drain both pipes on their own threads. `output()` drains while it
    // waits; a `try_wait` loop does not, so without these a chatty git
    // (push progress, a long diff) would fill a pipe buffer and block
    // forever — a deadlock wearing a timeout's clothes.
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let out_reader = std::thread::spawn(move || drain(out_pipe.as_mut()));
    let err_reader = std::thread::spawn(move || drain(err_pipe.as_mut()));

    let Some(timeout) = timeout else {
        let status = child
            .wait()
            .map_err(|e| Error::Git(format!("waiting on git {label}: {e}")))?;
        return Ok(collect(status, out_reader, err_reader));
    };

    let deadline = Instant::now() + timeout;
    let mut interval = Duration::from_micros(250);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(collect(status, out_reader, err_reader)),
            Ok(None) => {
                let now = Instant::now();
                if now >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Deliberately NOT joining the readers: a killed git may
                    // leave a grandchild (ssh, credential helper) holding the
                    // write end, and joining would hang the very call this
                    // bound exists to end. The threads exit at EOF.
                    return Err(Error::GitTimeout(format!(
                        "git {label} exceeded {}s",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(interval.min(deadline - now));
                interval = (interval * 2).min(MAX_POLL_INTERVAL);
            }
            Err(e) => return Err(Error::Git(format!("waiting on git {label}: {e}"))),
        }
    }
}

fn drain(pipe: Option<&mut impl Read>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(pipe) = pipe {
        let _ = pipe.read_to_end(&mut buf);
    }
    buf
}

fn collect(
    status: std::process::ExitStatus,
    out: std::thread::JoinHandle<Vec<u8>>,
    err: std::thread::JoinHandle<Vec<u8>>,
) -> Output {
    Output {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    }
}

/// First arg of a git invocation — the subcommand, which is all a timeout
/// message needs ("git fetch exceeded 60s").
fn label_of<'a>(args: &[&'a str]) -> &'a str {
    match args.first().copied() {
        Some(a) if !a.starts_with('-') => a,
        _ => "command",
    }
}

/// Runs git in `worktree`, returning raw stdout bytes; non-zero exit is an
/// `Error::Git` carrying stderr.
pub(crate) fn git_stdout(worktree: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let mut cmd = git_cmd(Some(worktree));
    cmd.args(args);
    let output = output_bounded(cmd, GitTimeout::Local, label_of(args))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::Git(format!(
            "git {args:?} failed: {}",
            stderr.trim()
        )))
    }
}

/// Runs git in `worktree`, returning stdout bytes on success and `None` on
/// any failure (missing blob spec, spawn error). Probing helper for callers
/// that treat absence as data (`diff::read_blob_chain`, tolerated numstat).
pub(crate) fn git_stdout_ok(worktree: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut cmd = git_cmd(Some(worktree));
    cmd.args(args);
    // A probe's contract is "absence is data", so a timeout still reads as
    // `None` — but it is an anomaly, not an answer, so it gets logged.
    let output = match output_bounded(cmd, GitTimeout::Local, label_of(args)) {
        Ok(output) => output,
        Err(e @ Error::GitTimeout(_)) => {
            tracing::warn!(error = %e, ?args, "git probe timed out");
            return None;
        }
        Err(_) => return None,
    };
    output.status.success().then_some(output.stdout)
}

impl CliGit {
    fn run<I, S>(&self, path: Option<&Path>, args: I) -> Result<String, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.run_timed(path, args, GitTimeout::Local)
    }

    fn run_timed<I, S>(
        &self,
        path: Option<&Path>,
        args: I,
        class: GitTimeout,
    ) -> Result<String, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let args: Vec<std::ffi::OsString> = args
            .into_iter()
            .map(|a| a.as_ref().to_os_string())
            .collect();
        let label = args
            .first()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_else(|| "command".to_string());
        let mut cmd = git_cmd(path);
        cmd.args(&args);
        let output = output_bounded(cmd, class, &label)?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(Error::Git(format!(
                "git {args:?} failed: {}",
                stderr.trim()
            )))
        }
    }

    fn run_ok<I, S>(&self, path: Option<&Path>, args: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.run(path, args).map(|_| ())
    }

    fn run_ok_timed<I, S>(
        &self,
        path: Option<&Path>,
        args: I,
        class: GitTimeout,
    ) -> Result<(), Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.run_timed(path, args, class).map(|_| ())
    }

    fn run_quiet_ok<I, S>(&self, path: Option<&Path>, args: I) -> Result<bool, Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let args: Vec<std::ffi::OsString> = args
            .into_iter()
            .map(|a| a.as_ref().to_os_string())
            .collect();
        let label = args
            .first()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_else(|| "command".to_string());
        let mut cmd = git_cmd(path);
        cmd.args(&args);
        // Pipe (don't inherit) stdout/stderr: "quiet" must not print — e.g.
        // `rev-parse --verify` would otherwise leak the oid, and
        // `--is-inside-work-tree` would print "true" into the host terminal.
        let output = output_bounded(cmd, GitTimeout::Local, &label)?;
        Ok(output.status.success())
    }
}

impl GitOps for CliGit {
    fn is_git_repo(&self, path: &Path) -> Result<bool, Error> {
        self.run_quiet_ok(Some(path), ["rev-parse", "--is-inside-work-tree"])
    }

    fn init(&self, path: &Path) -> Result<(), Error> {
        let mut cmd = git_cmd(Some(path));
        cmd.arg("init");
        let output = output_bounded(cmd, GitTimeout::Local, "init")?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Git(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn clone(&self, url: &str, target: &Path) -> Result<(), Error> {
        let mut cmd = git_cmd(None);
        cmd.arg("clone").arg(url).arg(target);
        // Extended, not Network: a first clone is an unbounded-by-nature
        // transfer, and killing it at 60s would break big repos on purpose.
        let output = output_bounded(cmd, GitTimeout::Extended, "clone")?;
        if output.status.success() {
            Ok(())
        } else {
            Err(Error::Git(format!(
                "git clone {url} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn show_toplevel(&self, path: &Path) -> Result<PathBuf, Error> {
        let out = self.run(Some(path), ["rev-parse", "--show-toplevel"])?;
        Ok(PathBuf::from(out.trim()))
    }

    fn git_dir(&self, path: &Path) -> Result<PathBuf, Error> {
        let out = self.run(Some(path), ["rev-parse", "--git-dir"])?;
        let dir = PathBuf::from(out.trim());
        if dir.is_absolute() {
            Ok(dir)
        } else {
            // `--git-dir` returns a relative path for some layouts.
            Ok(path.join(dir))
        }
    }

    fn remotes(&self, path: &Path) -> Result<Vec<String>, Error> {
        let out = self.run(Some(path), ["remote"])?;
        let mut remotes: Vec<String> = out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        // Origin first if present (reference: origin preferred).
        if let Some(pos) = remotes.iter().position(|r| r == "origin") {
            if pos != 0 {
                remotes.swap(0, pos);
            }
        }
        Ok(remotes)
    }

    fn current_branch(&self, path: &Path) -> Result<Option<String>, Error> {
        // `git branch --show-current` exits non-zero on unborn/detached HEAD;
        // that's not an error, just "no branch".
        let mut cmd = git_cmd(Some(path));
        cmd.args(["branch", "--show-current"]);
        let output = output_bounded(cmd, GitTimeout::Local, "branch")?;
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(if branch.is_empty() {
                None
            } else {
                Some(branch)
            })
        } else {
            Ok(None)
        }
    }

    fn remote_head(&self, path: &Path, remote: &str) -> Result<Option<String>, Error> {
        // `git symbolic-ref --short refs/remotes/<remote>/HEAD`. The reference
        // falls back to `git remote show <remote>` (a network call that can
        // hang); Phase 0 drops that fallback — clones set origin/HEAD, and the
        // base-ref resolver has local candidates as a further fallback.
        let out = match self.run(
            Some(path),
            [
                "symbolic-ref",
                "--short",
                &format!("refs/remotes/{remote}/HEAD"),
            ],
        ) {
            Ok(out) => out,
            Err(_) => return Ok(None),
        };
        let head = out.trim().to_string();
        if head.is_empty() || head == remote {
            return Ok(None);
        }
        // `--short` prints "origin/main"; strip the remote prefix.
        let name = head
            .strip_prefix(&format!("{remote}/"))
            .unwrap_or(&head)
            .to_string();
        Ok(Some(name))
    }

    fn verify_ref(&self, path: &Path, refname: &str) -> Result<bool, Error> {
        self.run_quiet_ok(Some(path), ["rev-parse", "--verify", "--quiet", refname])
    }

    fn branches(&self, path: &Path) -> Result<Vec<BranchRef>, Error> {
        let out = self.run(
            Some(path),
            [
                "branch",
                "-a",
                "--format=%(refname)|%(refname:short)|%(upstream:short)|%(upstream:track)|%(objectname)",
            ],
        )?;
        let mut branches = Vec::new();
        for line in out.lines() {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() < 5 || parts[0].is_empty() {
                continue;
            }
            let refname = parts[0];
            if refname.ends_with("/HEAD") {
                continue;
            }
            branches.push(BranchRef {
                name: parts[1].to_string(),
                upstream: {
                    let u = parts[2].trim();
                    if u.is_empty() {
                        None
                    } else {
                        Some(u.to_string())
                    }
                },
                is_remote: refname.starts_with("refs/remotes/"),
                object: parts[4].to_string(),
            });
        }
        Ok(branches)
    }

    fn default_branch(&self, path: &Path) -> Result<String, Error> {
        let out = self.run(Some(path), ["rev-parse", "--abbrev-ref", "HEAD"])?;
        let branch = out.trim();
        if !branch.is_empty() && branch != "HEAD" {
            return Ok(branch.to_string());
        }
        for candidate in ["main", "master"] {
            if self.verify_ref(path, &format!("refs/heads/{candidate}"))? {
                return Ok(candidate.to_string());
            }
        }
        Ok("main".to_string())
    }

    fn worktree_list(&self, repo_path: &Path) -> Result<Vec<WorktreeEntry>, Error> {
        let out = self.run(Some(repo_path), ["worktree", "list", "--porcelain"])?;
        let mut entries = Vec::new();
        let mut current: Option<WorktreeEntry> = None;
        for line in out.lines() {
            if line.is_empty() {
                if let Some(entry) = current.take() {
                    entries.push(entry);
                }
                continue;
            }
            let entry = current.get_or_insert_with(|| WorktreeEntry {
                path: PathBuf::new(),
                head: String::new(),
                branch: None,
                bare: false,
                locked: false,
            });
            if let Some(value) = line.strip_prefix("worktree ") {
                entry.path = PathBuf::from(value.trim());
            } else if let Some(value) = line.strip_prefix("HEAD ") {
                entry.head = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("branch ") {
                entry.branch = Some(value.trim().to_string());
            } else if line == "bare" {
                entry.bare = true;
            } else if line == "locked" {
                entry.locked = true;
            }
        }
        if let Some(entry) = current {
            entries.push(entry);
        }
        Ok(entries)
    }

    fn is_worktree(&self, path: &Path) -> Result<bool, Error> {
        self.is_git_repo(path)
    }

    fn fetch(&self, repo_path: &Path, remote: &str) -> Result<(), Error> {
        self.run_ok_timed(Some(repo_path), ["fetch", remote], GitTimeout::Network)
    }

    fn worktree_add(
        &self,
        repo_path: &Path,
        target_path: &Path,
        branch: &str,
    ) -> Result<(), Error> {
        self.run_ok(
            Some(repo_path),
            [
                "worktree",
                "add",
                target_path.to_string_lossy().as_ref(),
                branch,
            ],
        )
    }

    fn worktree_prune(&self, repo_path: &Path) -> Result<(), Error> {
        self.run_ok(Some(repo_path), ["worktree", "prune"])
    }

    fn worktree_prune_timed(&self, repo_path: &Path, timeout: Duration) -> Result<(), Error> {
        // Bounded prune (reference pruneGitWorktrees: 5s timeout + the
        // NON_INTERACTIVE_ENV already applied by git_cmd). The caller names
        // the bound here, so it goes to `output_within` rather than a class.
        let mut cmd = git_cmd(Some(repo_path));
        cmd.args(["worktree", "prune"]);
        let output = output_within(cmd, Some(timeout), "worktree prune")?;
        if output.status.success() {
            return Ok(());
        }
        Err(Error::Git(format!(
            "git worktree prune failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }

    fn is_worktree_clean(&self, _repo: &Path, worktree: &Path) -> Result<bool, Error> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(worktree)
            .args(["status", "--porcelain"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .map_err(|e| Error::Git(format!("status porcelain: {e}")))?;
        // The exit code is always 0.
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().is_empty())
    }

    fn worktree_remove(&self, repo_path: &Path, worktree_path: &Path) -> Result<(), Error> {
        // Phase 0: rm -rf + prune (matches §6.4). E2-02 should switch to
        // `git worktree remove` (safer for dirty/detached worktrees).
        if worktree_path.exists() {
            std::fs::remove_dir_all(worktree_path)?;
        }
        self.worktree_prune(repo_path)
    }

    fn branch_create(&self, repo_path: &Path, name: &str, start_point: &str) -> Result<(), Error> {
        // `--` so dash-leading names are not parsed as options (reference parity).
        self.run_ok(
            Some(repo_path),
            ["branch", "--no-track", "--", name, start_point],
        )
    }

    fn branch_delete(&self, repo_path: &Path, name: &str, force: bool) -> Result<(), Error> {
        // `--` so dash-leading names are not parsed as options (reference
        // `deleteBranch` parity: `-d` safe delete, `-D` forced).
        let flag = if force { "-D" } else { "-d" };
        self.run_ok(Some(repo_path), ["branch", flag, "--", name])
    }

    fn config_get(&self, repo_path: &Path, key: &str) -> Result<Option<String>, Error> {
        // `git config --get` exits 1 (no output) when the key is unset — that
        // is Ok(None). Any OTHER failure (corrupt config, spawn error) is an
        // error, not a silent None.
        let mut cmd = git_cmd(Some(repo_path));
        cmd.args(["config", "--get", key]);
        let output = cmd
            .output()
            .map_err(|e| Error::Git(format!("failed to spawn git config: {e}")))?;
        if !output.status.success() {
            if output.status.code() == Some(1) {
                return Ok(None);
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!(
                "git config --get {key} failed: {}",
                stderr.trim()
            )));
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(if value.is_empty() { None } else { Some(value) })
    }

    fn config_set(&self, repo_path: &Path, key: &str, value: &str) -> Result<(), Error> {
        self.run_ok(Some(repo_path), ["config", key, value])
    }

    fn push(
        &self,
        repo_path: &Path,
        remote: &str,
        branch: &str,
        set_upstream: bool,
    ) -> Result<(), Error> {
        let mut args = vec!["push".to_string()];
        if set_upstream {
            args.push("-u".into());
        }
        args.push(remote.into());
        args.push(branch.into());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run_ok_timed(Some(repo_path), args, GitTimeout::Network)
    }

    fn is_tracked(&self, repo_path: &Path, rel_path: &str) -> Result<bool, Error> {
        // `git ls-files --error-unmatch`: exit 0 = tracked, 1 = untracked
        // (or absent) — both are meaningful answers, so surface as bool.
        self.run_quiet_ok(
            Some(repo_path),
            ["ls-files", "--error-unmatch", "--", rel_path],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo_with_commit(tmp: &tempfile::TempDir) -> PathBuf {
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = CliGit;
        git.init(&repo).unwrap();
        std::fs::write(repo.join("README.md"), "# test\n").unwrap();
        run_quiet(&repo, ["add", "."]).unwrap();
        run_quiet(
            &repo,
            [
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@fartCode.dev",
                "commit",
                "-m",
                "init",
            ],
        )
        .unwrap();
        repo
    }

    fn run_quiet<I, S>(path: &Path, args: I) -> Result<(), Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
        Ok(())
    }

    #[test]
    fn timeout_overrides_parse_conservatively() {
        let default = Duration::from_secs(30);
        // Unset / empty / garbage all keep the default: a typo in the env
        // must never silently restore the unbounded wait.
        assert_eq!(parse_timeout_secs(None, default), Some(default));
        assert_eq!(parse_timeout_secs(Some("  "), default), Some(default));
        assert_eq!(parse_timeout_secs(Some("soon"), default), Some(default));
        assert_eq!(parse_timeout_secs(Some("-5"), default), Some(default));
        // A number is the bound; a literal 0 is the documented opt-out.
        assert_eq!(
            parse_timeout_secs(Some("90"), default),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            parse_timeout_secs(Some(" 90 "), default),
            Some(Duration::from_secs(90))
        );
        assert_eq!(parse_timeout_secs(Some("0"), default), None);
    }

    #[test]
    fn timeout_classes_keep_their_documented_defaults() {
        // Read the constants, not `duration()`: the latter honours env
        // overrides, and a developer's shell must not fail the suite.
        assert_eq!(
            GitTimeout::Local.default_duration(),
            Duration::from_secs(30)
        );
        assert_eq!(
            GitTimeout::Network.default_duration(),
            Duration::from_secs(60)
        );
        assert_eq!(
            GitTimeout::Extended.default_duration(),
            Duration::from_secs(600)
        );
        assert_eq!(
            GitTimeout::Network.env_var(),
            "FARTCODE_GIT_TIMEOUT_NETWORK_SECS"
        );
    }

    #[test]
    fn bounded_run_returns_output_under_budget() {
        let mut cmd = git_cmd(None);
        cmd.arg("--version");
        let out = output_within(cmd, Some(Duration::from_secs(30)), "--version").unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("git version"));
    }

    /// The bound this whole change exists for: a child that outlives it is
    /// killed and reported, instead of wedging the caller forever.
    #[cfg(unix)]
    #[test]
    fn bounded_run_kills_an_over_budget_child() {
        let started = std::time::Instant::now();
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let err = output_within(cmd, Some(Duration::from_millis(150)), "sleep").unwrap_err();
        // Bounded assertion: if the kill path were broken this test would
        // fail on the elapsed check rather than hang the suite.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timeout did not fire promptly: {:?}",
            started.elapsed()
        );
        assert!(
            matches!(err, Error::GitTimeout(_)),
            "expected a typed timeout, got {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "git operation timed out: git sleep exceeded 0s"
        );
    }

    /// Output larger than a pipe buffer must not deadlock: `try_wait` never
    /// drains, so the readers have to run on their own threads.
    #[cfg(unix)]
    #[test]
    fn bounded_run_drains_more_than_a_pipe_buffer() {
        let mut cmd = Command::new("head");
        cmd.args(["-c", "300000", "/dev/zero"]);
        let out = output_within(cmd, Some(Duration::from_secs(20)), "head").unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 300_000);
    }

    #[test]
    fn detects_and_inits_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("empty");
        std::fs::create_dir_all(&dir).unwrap();
        let git = CliGit;

        assert!(!git.is_git_repo(&dir).unwrap());
        git.init(&dir).unwrap();
        assert!(git.is_git_repo(&dir).unwrap());
        assert!(git.show_toplevel(&dir).unwrap().ends_with("empty"));
    }

    #[test]
    fn resolves_branches_and_refs() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo_with_commit(&tmp);
        let git = CliGit;

        assert_eq!(
            git.current_branch(&repo).unwrap().as_deref(),
            Some("master")
        );
        assert_eq!(git.default_branch(&repo).unwrap(), "master");
        assert!(git.verify_ref(&repo, "refs/heads/master").unwrap());
        assert!(!git.verify_ref(&repo, "refs/heads/nope").unwrap());

        let branches = git.branches(&repo).unwrap();
        assert!(branches.iter().any(|b| b.name == "master" && !b.is_remote));
    }

    #[test]
    fn remote_head_and_remotes() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("bare.git");
        Command::new("git")
            .args(["init", "--bare", bare.to_str().unwrap()])
            .status()
            .unwrap();

        // Seed the remote with a main branch BEFORE cloning so the clone
        // sets refs/remotes/origin/HEAD (a clone of an empty repo does not).
        let seed = tmp.path().join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        let git = CliGit;
        git.init(&seed).unwrap();
        std::fs::write(seed.join("README.md"), "# test\n").unwrap();
        run_quiet(&seed, ["add", "."]).unwrap();
        run_quiet(
            &seed,
            [
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@t",
                "commit",
                "-m",
                "init",
            ],
        )
        .unwrap();
        run_quiet(&seed, ["branch", "-M", "main"]).unwrap();
        run_quiet(&seed, ["remote", "add", "origin", bare.to_str().unwrap()]).unwrap();
        run_quiet(&seed, ["push", "-u", "origin", "main"]).unwrap();
        // Point the bare repo's HEAD at main so clones resolve origin/HEAD.
        Command::new("git")
            .args([
                "--git-dir",
                bare.to_str().unwrap(),
                "symbolic-ref",
                "HEAD",
                "refs/heads/main",
            ])
            .status()
            .unwrap();

        let work = tmp.path().join("work");
        git.clone(bare.to_str().unwrap(), &work).unwrap();

        let remotes = git.remotes(&work).unwrap();
        assert_eq!(remotes, vec!["origin".to_string()]);
        assert_eq!(
            git.remote_head(&work, "origin").unwrap().as_deref(),
            Some("main")
        );
        assert_eq!(git.current_branch(&work).unwrap().as_deref(), Some("main"));
    }

    #[test]
    fn worktree_list_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo_with_commit(&tmp);
        let git = CliGit;

        git.branch_create(&repo, "feature", "master").unwrap();
        let wt_raw = tmp.path().join("wt-feature");
        git.worktree_add(&repo, &wt_raw, "feature").unwrap();
        let wt = std::fs::canonicalize(&wt_raw).unwrap(); // git reports realpaths

        let entries = git.worktree_list(&repo).unwrap();
        assert!(entries
            .iter()
            .any(|e| e.path == wt && e.branch.as_deref() == Some("refs/heads/feature")));

        git.worktree_remove(&repo, &wt_raw).unwrap();
        assert!(!wt.exists());
        let entries = git.worktree_list(&repo).unwrap();
        assert!(!entries.iter().any(|e| e.path == wt));
    }

    #[test]
    fn clone_creates_toplevel() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("bare.git");
        Command::new("git")
            .args(["init", "--bare", bare.to_str().unwrap()])
            .status()
            .unwrap();
        let target = tmp.path().join("clone");
        let git = CliGit;
        git.clone(bare.to_str().unwrap(), &target).unwrap();
        assert!(target.join(".git").exists());
        assert!(git.is_git_repo(&target).unwrap());
    }
}
