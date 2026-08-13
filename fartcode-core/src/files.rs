//! Workspace file writes (E4-05, #45; E5 will add reads/tree listing here).
//!
//! The diff editor's ⌘S path writes the worktree side of an unstaged diff
//! back to disk. Containment (AGENTS.md realpath rule) has one owner:
//! [`resolve_contained`] — lexical + canonical, documented there — which
//! the line-comment anchor guardrail also calls through.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::Error;

/// How [`resolve_contained`] treats a target that doesn't exist yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveMode {
    /// The target must canonicalize — a missing file surfaces the
    /// underlying io error.
    MustExist,
    /// The target may not exist yet (write path): the nearest existing
    /// ancestor is canonicalized and the missing tail re-appended, so a
    /// symlinked ancestor is still caught.
    ForWrite,
}

/// Resolves `<worktree>/<rel_path>` and proves the result stays inside the
/// worktree — the single owner of the containment rule (AGENTS.md realpath
/// rule) for every worktree-relative path the frontend or an agent hands
/// us. Two checks: lexical — non-empty, not absolute, no `..`, which also
/// covers targets that can't be canonicalized yet — and canonical — the
/// filesystem-resolved target must stay under the canonical worktree root,
/// which covers symlink escapes. `.` components (`./x`) are accepted:
/// they can't escape, and user-visible paths keep working however the
/// caller spells them.
pub fn resolve_contained(
    worktree: &Path,
    rel_path: &str,
    mode: ResolveMode,
) -> Result<PathBuf, Error> {
    let rel = Path::new(rel_path);
    let lexical = !rel_path.is_empty()
        && rel
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !lexical {
        return Err(Error::PathEscape(rel_path.into()));
    }

    let canonical_worktree = worktree.canonicalize()?;
    let target = canonical_worktree.join(rel);
    let resolved = match mode {
        ResolveMode::MustExist => target.canonicalize()?,
        ResolveMode::ForWrite => resolve_for_write(&target)?,
    };
    if !resolved.starts_with(&canonical_worktree) {
        return Err(Error::PathEscape(rel_path.into()));
    }
    Ok(resolved)
}

/// Writes `content` to `<worktree>/<rel_path>`, creating the file when it
/// doesn't exist (parent must). Fails with [`Error::PathEscape`] for any
/// path that would land outside the worktree.
pub fn write_file(worktree: &Path, rel_path: &str, content: &str) -> Result<(), Error> {
    let resolved = resolve_contained(worktree, rel_path, ResolveMode::ForWrite)?;
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

/// Reads `<worktree>/<rel_path>` as UTF-8 (E5-02 editor open path).
/// Same containment as [`write_file`] via [`resolve_contained`]; the
/// target must exist (symlinked files pointing outside are rejected).
pub fn read_file(worktree: &Path, rel_path: &str) -> Result<String, Error> {
    let resolved = resolve_contained(worktree, rel_path, ResolveMode::MustExist)?;
    Ok(std::fs::read_to_string(&resolved)?)
}

/// One row of a directory listing (E5-01 file tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Directories hidden from the file tree by default (PRD E5:
/// node_modules / .git / build output).
const HIDDEN_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "dist",
    "target",
    "build",
    "out",
    ".next",
    ".venv",
    "__pycache__",
];

/// Lists `<worktree>/<rel_path>` (empty `rel_path` = the worktree root).
/// Same containment as [`write_file`] via [`resolve_contained`] — the
/// resolved dir must exist (symlinked dirs pointing outside are rejected).
/// Hidden dirs are filtered; entries sort dirs-first, then
/// case-insensitive by name. Symlink entries are listed as files (never
/// followed).
pub fn list_dir(worktree: &Path, rel_path: &str) -> Result<Vec<DirEntry>, Error> {
    // The empty-string root spelling is this API's alone — the helper
    // rejects empty paths, so map it to `.` before the shared check.
    let rel = if rel_path.is_empty() { "." } else { rel_path };
    let dir = resolve_contained(worktree, rel, ResolveMode::MustExist)?;

    let mut entries: Vec<DirEntry> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // file_type() never follows symlinks — a link to a dir lists as a
        // file, so the tree can't recurse through link cycles or escapes.
        let is_dir = entry.file_type()?.is_dir();
        if is_dir && HIDDEN_DIRS.contains(&name.as_str()) {
            continue;
        }
        entries.push(DirEntry { name, is_dir });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_contained_rejects_traversal_in_both_modes() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["..", "../x", "/etc/passwd", "a/../../b.txt", ""] {
            for mode in [ResolveMode::MustExist, ResolveMode::ForWrite] {
                assert!(
                    matches!(
                        resolve_contained(tmp.path(), bad, mode),
                        Err(Error::PathEscape(_))
                    ),
                    "{bad} must be rejected in {mode:?}"
                );
            }
        }
    }

    #[test]
    fn resolve_contained_accepts_curdir_components() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/a.txt"), "").unwrap();

        let plain = resolve_contained(tmp.path(), "sub/a.txt", ResolveMode::MustExist).unwrap();
        for spelled in ["./sub/a.txt", "sub/./a.txt"] {
            assert_eq!(
                resolve_contained(tmp.path(), spelled, ResolveMode::MustExist).unwrap(),
                plain,
                "{spelled} must resolve like the plain spelling"
            );
        }
        assert_eq!(
            resolve_contained(tmp.path(), "./sub/new.txt", ResolveMode::ForWrite).unwrap(),
            plain.parent().unwrap().join("new.txt")
        );
    }

    #[test]
    fn resolve_contained_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp.path().join("wt");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&worktree).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), worktree.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, worktree.join("dir")).unwrap();

        // A symlinked file (MustExist) and a missing file under a
        // symlinked dir (ForWrite — nothing to canonicalize at the tail)
        // both resolve outside and are rejected.
        assert!(matches!(
            resolve_contained(&worktree, "link.txt", ResolveMode::MustExist),
            Err(Error::PathEscape(_))
        ));
        assert!(matches!(
            resolve_contained(&worktree, "dir/new.txt", ResolveMode::ForWrite),
            Err(Error::PathEscape(_))
        ));
    }

    #[test]
    fn resolve_contained_for_write_allows_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical = tmp.path().canonicalize().unwrap();

        // ForWrite resolves a not-yet-existing target; MustExist surfaces
        // the io error (not a containment failure).
        assert_eq!(
            resolve_contained(tmp.path(), "new.txt", ResolveMode::ForWrite).unwrap(),
            canonical.join("new.txt")
        );
        assert!(matches!(
            resolve_contained(tmp.path(), "new.txt", ResolveMode::MustExist),
            Err(Error::Io(_))
        ));
    }

    #[test]
    fn reads_file_and_rejects_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello\n").unwrap();
        assert_eq!(read_file(tmp.path(), "a.txt").unwrap(), "hello\n");
        for bad in ["../a.txt", "/etc/passwd", ""] {
            assert!(matches!(
                read_file(tmp.path(), bad),
                Err(Error::PathEscape(_))
            ));
        }
    }

    #[test]
    fn read_rejects_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp.path().join("wt");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&worktree).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "s").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), worktree.join("link.txt")).unwrap();
        assert!(matches!(
            read_file(&worktree, "link.txt"),
            Err(Error::PathEscape(_))
        ));
    }

    #[test]
    fn lists_root_dirs_first_hidden_filtered() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::create_dir(tmp.path().join("node_modules")).unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        std::fs::write(tmp.path().join("a.txt"), "").unwrap();
        std::fs::write(tmp.path().join("Zed.md"), "").unwrap();

        let names: Vec<(String, bool)> = list_dir(tmp.path(), "")
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.is_dir))
            .collect();
        assert_eq!(
            names,
            vec![
                ("src".into(), true),
                ("a.txt".into(), false),
                ("Zed.md".into(), false),
            ]
        );
    }

    #[test]
    fn lists_subdir_and_rejects_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/f.txt"), "").unwrap();

        let entries = list_dir(tmp.path(), "sub").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "f.txt");

        for bad in ["..", "../x", "/etc"] {
            assert!(
                matches!(list_dir(tmp.path(), bad), Err(Error::PathEscape(_))),
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_symlinked_dir_escape_and_lists_link_as_file() {
        let tmp = tempfile::tempdir().unwrap();
        let worktree = tmp.path().join("wt");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&worktree).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, worktree.join("link")).unwrap();

        // Listing THROUGH the link is a containment failure…
        assert!(matches!(
            list_dir(&worktree, "link"),
            Err(Error::PathEscape(_))
        ));
        // …and the link itself lists as a plain file (never a dir).
        let entries = list_dir(&worktree, "").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].is_dir);
    }

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
