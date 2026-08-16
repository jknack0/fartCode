//! Stage / unstage / discard write operations (E4-03, #43).
//!
//! Worktree-scoped mutations behind the Changes sidebar row actions. All
//! paths are worktree-relative and validated lexically (same rule as
//! `diff::file_diff` — command inputs must stay inside the worktree). These
//! are the only git mutations the UI performs; everything funnels through
//! here so path validation and the unborn-HEAD edge live in exactly one
//! place.

use std::path::Path;

use fartcode_core::Error;

use crate::diff::validate_rel_path;
use crate::{git_stdout, GitOps};

/// `git add -- <paths>` (stage selected paths).
pub fn stage(worktree: &Path, paths: &[String]) -> Result<(), Error> {
    validate_paths(paths)?;
    git_stdout(worktree, &with_paths("add", paths)).map(|_| ())
}

/// `git add -A` (stage everything, including deletions and untracked).
pub fn stage_all(worktree: &Path) -> Result<(), Error> {
    git_stdout(worktree, &["add", "-A"]).map(|_| ())
}

/// `git restore --staged -- <paths>` (unstage). On an unborn HEAD (fresh
/// repo, no commits) `--staged` cannot resolve HEAD, so fall back to
/// `git rm --cached -r -- <paths>` — the index-emptying equivalent.
pub fn unstage(worktree: &Path, paths: &[String]) -> Result<(), Error> {
    validate_paths(paths)?;
    let head_born = crate::CliGit.verify_ref(worktree, "HEAD")?;
    if head_born {
        let mut args = vec!["restore", "--staged", "--"];
        args.extend(paths.iter().map(String::as_str));
        git_stdout(worktree, &args).map(|_| ())
    } else {
        let mut args = vec!["rm", "--cached", "-r", "--"];
        args.extend(paths.iter().map(String::as_str));
        git_stdout(worktree, &args).map(|_| ())
    }
}

/// Reverts paths to their index state. Tracked paths go through
/// `git restore --worktree -- <paths>`; untracked paths are moved into the
/// worktree's holding pen (`.fartCode/trash/<epoch-millis>/<path>`) instead
/// of being deleted, so a mistaken discard is recoverable by hand (#129).
/// The pen carries a `*` gitignore so trashed files never resurface as
/// untracked changes. A path that is neither tracked nor present on disk
/// is an error (typo guard).
pub fn discard(worktree: &Path, paths: &[String]) -> Result<(), Error> {
    validate_paths(paths)?;
    let mut tracked: Vec<&str> = Vec::new();
    let mut batch: Option<std::path::PathBuf> = None;
    for path in paths {
        if crate::CliGit.is_tracked(worktree, path)? {
            tracked.push(path);
        } else {
            let full = worktree.join(path);
            if !full.exists() {
                return Err(Error::Git(format!(
                    "path is neither tracked nor on disk: {path}"
                )));
            }
            let batch_dir = match &batch {
                Some(dir) => dir.clone(),
                None => {
                    let dir = new_trash_batch(worktree)?;
                    batch = Some(dir.clone());
                    dir
                }
            };
            let dest = batch_dir.join(path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&full, &dest)?;
        }
    }
    if !tracked.is_empty() {
        let mut args = vec!["restore", "--worktree", "--"];
        args.extend(tracked);
        git_stdout(worktree, &args)?;
    }
    Ok(())
}

/// Creates a fresh per-discard batch directory under the worktree's trash
/// pen (`.fartCode/trash/`). `.fartCode/` is fartCode's gitignored internals
/// dir; the pen additionally writes its own `*` gitignore so it stays
/// invisible to git even in repos that never ignored `.fartCode/`. Batch
/// names are epoch millis, `-<n>` suffixed on collision.
fn new_trash_batch(worktree: &Path) -> Result<std::path::PathBuf, Error> {
    let root = worktree.join(".fartCode").join("trash");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join(".gitignore"), "*\n")?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    for n in 0u32.. {
        let name = if n == 0 {
            millis.to_string()
        } else {
            format!("{millis}-{n}")
        };
        let dir = root.join(name);
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!("u32 batch suffixes exhausted")
}

fn validate_paths(paths: &[String]) -> Result<(), Error> {
    if paths.is_empty() {
        return Err(Error::Git("no paths given".into()));
    }
    for p in paths {
        validate_rel_path(p)?;
    }
    Ok(())
}

fn with_paths<'a>(cmd: &'a str, paths: &'a [String]) -> Vec<&'a str> {
    let mut args = vec![cmd, "--"];
    args.extend(paths.iter().map(String::as_str));
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::status;
    use crate::status::tests::{git, repo_fixture};
    use crate::status::ChangeStatus;

    fn paths(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stage_and_unstage_round_trip() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("tracked.txt"), "changed\n").unwrap();

        stage(repo.path(), &paths(&["tracked.txt"])).unwrap();
        let snap = status(repo.path()).unwrap();
        assert_eq!(snap.staged.len(), 1);
        assert!(snap.unstaged.is_empty());

        unstage(repo.path(), &paths(&["tracked.txt"])).unwrap();
        let snap = status(repo.path()).unwrap();
        assert!(snap.staged.is_empty());
        assert_eq!(snap.unstaged.len(), 1);
        assert_eq!(snap.unstaged[0].status, ChangeStatus::Modified);
    }

    #[test]
    fn stage_all_picks_up_everything() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("tracked.txt"), "changed\n").unwrap();
        std::fs::write(repo.path().join("new.txt"), "new\n").unwrap();

        stage_all(repo.path()).unwrap();
        let snap = status(repo.path()).unwrap();
        assert_eq!(snap.staged.len(), 2);
        assert!(snap.unstaged.is_empty());
    }

    #[test]
    fn unstage_on_unborn_head_uses_rm_cached() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init"]);
        std::fs::write(tmp.path().join("f.txt"), "x\n").unwrap();
        git(tmp.path(), &["add", "."]);
        // Sanity: staged without any commit.
        assert_eq!(status(tmp.path()).unwrap().staged.len(), 1);

        unstage(tmp.path(), &paths(&["f.txt"])).unwrap();
        let snap = status(tmp.path()).unwrap();
        assert!(snap.staged.is_empty());
        assert_eq!(snap.unstaged[0].status, ChangeStatus::Added); // untracked again
    }

    /// Resolves a worktree-relative path inside the (single) trash batch.
    fn trashed(worktree: &Path, rel: &str) -> std::path::PathBuf {
        let root = worktree.join(".fartCode").join("trash");
        let batch = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_type().unwrap().is_dir())
            .expect("trash batch dir");
        batch.path().join(rel)
    }

    #[test]
    fn discard_restores_modified_and_trashes_untracked() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("tracked.txt"), "changed\n").unwrap();
        std::fs::write(repo.path().join("untracked.txt"), "bye\n").unwrap();

        discard(repo.path(), &paths(&["tracked.txt", "untracked.txt"])).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.path().join("tracked.txt")).unwrap(),
            "one\ntwo\nthree\n"
        );
        // Untracked file is moved to the holding pen, not destroyed (#129).
        assert!(!repo.path().join("untracked.txt").exists());
        assert_eq!(
            std::fs::read_to_string(trashed(repo.path(), "untracked.txt")).unwrap(),
            "bye\n"
        );
        // The pen self-ignores: status stays clean despite files in it.
        let snap = status(repo.path()).unwrap();
        assert!(snap.staged.is_empty() && snap.unstaged.is_empty());
    }

    #[test]
    fn discard_trashes_untracked_directory_recoverably() {
        let repo = repo_fixture();
        let dir = repo.path().join("newdir");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("inner.txt"), "keep me\n").unwrap();

        discard(repo.path(), &paths(&["newdir"])).unwrap();
        assert!(!dir.exists());
        assert_eq!(
            std::fs::read_to_string(trashed(repo.path(), "newdir/inner.txt")).unwrap(),
            "keep me\n"
        );
        let snap = status(repo.path()).unwrap();
        assert!(snap.staged.is_empty() && snap.unstaged.is_empty());
    }

    #[test]
    fn discard_missing_path_errors() {
        let repo = repo_fixture();
        assert!(discard(repo.path(), &paths(&["nope.txt"])).is_err());
    }

    #[test]
    fn path_validation_rejects_escape_and_empty() {
        let repo = repo_fixture();
        assert!(stage(repo.path(), &paths(&["../x"])).is_err());
        assert!(unstage(repo.path(), &paths(&["/abs"])).is_err());
        assert!(discard(repo.path(), &[]).is_err());
    }
}
