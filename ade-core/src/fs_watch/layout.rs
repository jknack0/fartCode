//! Git layout resolution for watch scoping (E4-01).
//!
//! Pure filesystem logic — no `git` binary, no git2 (this runs at watch
//! registration time and must never serialize behind GitOps): `.git` is
//! either a directory (main worktree) or a `gitdir: <path>` file (linked
//! worktree); `<git_dir>/commondir` points at the shared git dir when
//! present. All returned paths are canonicalized so classification matches
//! the realpaths reported by the platform watch backend (macOS FSEvents
//! reports `/private/…` for `/tmp` symlinks).

use std::fs;
use std::path::{Path, PathBuf};

use crate::Error;

/// Resolved git layout for one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitLayout {
    /// Canonical worktree root.
    pub worktree: PathBuf,
    /// This worktree's own git dir (holds its `HEAD` and `index`). Equals
    /// `common_dir` for the main worktree; `<common_dir>/worktrees/<name>`
    /// for linked worktrees.
    pub git_dir: PathBuf,
    /// The shared git dir (holds `refs/`, `packed-refs`, `config`).
    pub common_dir: PathBuf,
}

/// Resolves the git layout for `worktree`. Errors if the path (or its
/// `.git`) does not exist or the `.git` file is malformed.
pub fn resolve_git_layout(worktree: &Path) -> Result<GitLayout, Error> {
    let worktree = worktree.canonicalize()?;
    let dot_git = worktree.join(".git");
    let meta = fs::metadata(&dot_git)?;

    let git_dir = if meta.is_dir() {
        dot_git
    } else {
        // Linked worktree: `.git` is a file `gitdir: <path>`.
        let contents = fs::read_to_string(&dot_git)?;
        let target = contents
            .strip_prefix("gitdir:")
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Error::Git(format!("malformed .git file: {}", dot_git.display())))?;
        resolve_relative(&worktree, Path::new(target)).canonicalize()?
    };

    let common_dir = match fs::read_to_string(git_dir.join("commondir")) {
        Ok(contents) => resolve_relative(&git_dir, Path::new(contents.trim())).canonicalize()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => git_dir.clone(),
        Err(e) => return Err(e.into()),
    };

    Ok(GitLayout {
        worktree,
        git_dir,
        common_dir,
    })
}

fn resolve_relative(base: &Path, p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git spawns");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn main_worktree_layout() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init"]);

        let layout = resolve_git_layout(tmp.path()).unwrap();
        let root = tmp.path().canonicalize().unwrap();
        assert_eq!(layout.worktree, root);
        assert_eq!(layout.git_dir, root.join(".git"));
        assert_eq!(layout.common_dir, root.join(".git"));
    }

    #[test]
    fn linked_worktree_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir(&main).unwrap();
        git(&main, &["init"]);
        std::fs::write(main.join("f.txt"), "x").unwrap();
        git(&main, &["add", "."]);
        git(
            &main,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
            ],
        );
        let linked = tmp.path().join("linked");
        git(
            &main,
            &["worktree", "add", "-b", "wt", linked.to_str().unwrap()],
        );

        let layout = resolve_git_layout(&linked).unwrap();
        let main_root = main.canonicalize().unwrap();
        assert_eq!(layout.worktree, linked.canonicalize().unwrap());
        assert_eq!(layout.common_dir, main_root.join(".git"));
        assert!(
            layout.git_dir.starts_with(main_root.join(".git/worktrees")),
            "git_dir {:?} should live under the shared .git/worktrees",
            layout.git_dir
        );
        assert!(layout.git_dir.join("index").exists() || layout.git_dir.join("HEAD").exists());
    }

    #[test]
    fn non_repo_errors() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(resolve_git_layout(tmp.path()).is_err());
    }
}
