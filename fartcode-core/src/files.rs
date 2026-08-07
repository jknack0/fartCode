//! Workspace file writes (E4-05, #45; E5 will add reads/tree listing here).
//!
//! The diff editor's ⌘S path writes the worktree side of an unstaged diff
//! back to disk. Containment is enforced two ways (AGENTS.md realpath
//! rule): lexical — no absolute paths, no `..` components, which also
//! covers files that don't exist yet and can't be canonicalized — and
//! canonical — the resolved target must stay under the canonical worktree
//! root, which covers symlink escapes.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::Error;

/// Writes `content` to `<worktree>/<rel_path>`, creating the file when it
/// doesn't exist (parent must). Fails with [`Error::PathEscape`] for any
/// path that would land outside the worktree.
pub fn write_file(worktree: &Path, rel_path: &str, content: &str) -> Result<(), Error> {
    let rel = Path::new(rel_path);
    let lexical = !rel_path.is_empty()
        && rel
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !lexical {
        return Err(Error::PathEscape(rel_path.into()));
    }

    let canonical_worktree = worktree.canonicalize()?;
    let resolved = resolve_for_write(&canonical_worktree.join(rel))?;
    if !resolved.starts_with(&canonical_worktree) {
        return Err(Error::PathEscape(rel_path.into()));
    }
    std::fs::write(&resolved, content)?;
    Ok(())
}

/// Resolves `target` through the filesystem for the containment check: the
/// canonical path when it exists, else the canonical nearest existing
/// ancestor with the missing tail re-appended (so a symlinked ancestor is
/// still caught).
fn resolve_for_write(target: &Path) -> Result<PathBuf, Error> {
    if target.exists() {
        return Ok(target.canonicalize()?);
    }
    let mut missing: Vec<OsString> = target
        .file_name()
        .map(|n| n.to_os_string())
        .into_iter()
        .collect();
    let mut dir = target.parent();
    while let Some(d) = dir {
        if d.exists() {
            let mut resolved = d.canonicalize()?;
            for part in missing.iter().rev() {
                resolved.push(part);
            }
            return Ok(resolved);
        }
        if let Some(name) = d.file_name() {
            missing.push(name.to_os_string());
        }
        dir = d.parent();
    }
    Ok(target.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_content_to_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, "old\n").unwrap();

        write_file(tmp.path(), "a.txt", "new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
    }

    #[test]
    fn creates_missing_file_in_existing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();

        write_file(tmp.path(), "sub/new.txt", "hello\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("sub/new.txt")).unwrap(),
            "hello\n"
        );
    }

    #[test]
    fn rejects_lexical_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["../out.txt", "/etc/passwd", "a/../../b.txt", ""] {
            assert!(
                matches!(write_file(tmp.path(), bad, "x"), Err(Error::PathEscape(_))),
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp.path().join("wt");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&worktree).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let secret = outside.join("target.txt");
        std::fs::write(&secret, "untouched\n").unwrap();
        std::os::unix::fs::symlink(&secret, worktree.join("link.txt")).unwrap();

        assert!(matches!(
            write_file(&worktree, "link.txt", "pwned\n"),
            Err(Error::PathEscape(_))
        ));
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), "untouched\n");
    }

    #[test]
    fn writes_through_safe_symlinked_dir() {
        // A symlink INSIDE the worktree pointing at another dir INSIDE the
        // worktree is fine — containment still holds after resolution.
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp.path().join("wt");
        let real = worktree.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, worktree.join("link")).unwrap();

        write_file(&worktree, "link/f.txt", "ok\n").unwrap();
        assert_eq!(std::fs::read_to_string(real.join("f.txt")).unwrap(), "ok\n");
    }
}
