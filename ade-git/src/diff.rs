//! Per-file diff payloads (E4-02, #42).
//!
//! The diff view (E4-04) renders with `@codemirror/merge`, which computes
//! hunks from two documents — so the payload is the two sides' *contents*,
//! not parsed hunks (the reference does the same via `getFileAtRef` +
//! working-file reads). Sides per `DiffSide`:
//!
//! - `Staged`:   old = `HEAD:<orig_path|path>`, new = index (`:0:<path>`)
//! - `Unstaged`: old = index (`:0:`), falling back to `:2:` (ours, during a
//!   conflict) then `HEAD:`; new = the working file on disk
//!
//! Guards (UI-safe, never the full content): either side larger than
//! [`MAX_DIFF_CONTENT_BYTES`] → `too_large`; NUL in the first 8 KiB of
//! either side → `binary`. Both guards clear the contents and keep sizes so
//! the UI can say why. Absent sides (added/deleted/untracked) report
//! `*_exists: false` with `None` content.

use std::path::{Component, Path};

use ade_core::Error;
use serde::{Deserialize, Serialize};

use crate::git_stdout_ok;

/// Content cap per side (reference `MAX_DIFF_CONTENT_BYTES`).
pub const MAX_DIFF_CONTENT_BYTES: u64 = 512 * 1024;
/// Binary sniff window (git's own heuristic checks the first 8000 bytes).
const BINARY_SNIFF_BYTES: usize = 8000;

/// Which pair of sides to diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffSide {
    /// HEAD ↔ index.
    Staged,
    /// index ↔ worktree.
    Unstaged,
}

/// Two-sided diff payload for one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    /// Worktree-relative path (rename target).
    pub path: String,
    /// Rename source (staged renames), echoed from the status entry.
    pub orig_path: Option<String>,
    /// `None` when the side is absent or a guard tripped.
    pub old_content: Option<String>,
    pub new_content: Option<String>,
    pub old_size: u64,
    pub new_size: u64,
    pub old_exists: bool,
    pub new_exists: bool,
    /// Either side sniffed as binary — contents withheld.
    pub binary: bool,
    /// Either side exceeds [`MAX_DIFF_CONTENT_BYTES`] — contents withheld.
    pub too_large: bool,
}

/// One resolved side.
#[derive(Debug, Default)]
struct Side {
    bytes: Option<Vec<u8>>,
    size: u64,
    exists: bool,
    too_large: bool,
}

/// Computes the diff payload for `rel_path` in `worktree`.
///
/// `orig_path` is the rename source from the status entry (staged renames:
/// the old side reads `HEAD:<orig_path>`).
pub fn file_diff(
    worktree: &Path,
    rel_path: &str,
    side: DiffSide,
    orig_path: Option<&str>,
) -> Result<FileDiff, Error> {
    validate_rel_path(rel_path)?;
    if let Some(orig) = orig_path {
        validate_rel_path(orig)?;
    }

    let (old, new) = match side {
        DiffSide::Staged => {
            let head_path = orig_path.unwrap_or(rel_path);
            let old = read_blob_chain(worktree, &[&format!("HEAD:{head_path}")])?;
            let new = read_blob_chain(worktree, &[&format!(":0:{rel_path}")])?;
            (old, new)
        }
        DiffSide::Unstaged => {
            let old = read_blob_chain(
                worktree,
                &[
                    &format!(":0:{rel_path}"),
                    // Conflicted files have no stage 0 — fall back to "ours",
                    // then HEAD, so the left pane still shows a baseline.
                    &format!(":2:{rel_path}"),
                    &format!("HEAD:{rel_path}"),
                ],
            )?;
            let new = read_worktree_file(&worktree.join(rel_path));
            (old, new)
        }
    };

    let binary = sniff_binary(&old) || sniff_binary(&new);
    let too_large = old.too_large || new.too_large;
    let readable = !binary && !too_large;

    Ok(FileDiff {
        path: rel_path.to_string(),
        orig_path: orig_path.map(str::to_string),
        old_content: old
            .bytes
            .filter(|_| readable)
            .map(|b| String::from_utf8_lossy(&b).into_owned()),
        new_content: new
            .bytes
            .filter(|_| readable)
            .map(|b| String::from_utf8_lossy(&b).into_owned()),
        old_size: old.size,
        new_size: new.size,
        old_exists: old.exists,
        new_exists: new.exists,
        binary,
        too_large,
    })
}

/// Rejects absolute paths and any `..`/root component — command inputs must
/// stay inside the worktree (AGENTS.md realpath-containment rule; lexical
/// here because deleted files cannot be canonicalized).
pub(crate) fn validate_rel_path(rel_path: &str) -> Result<(), Error> {
    let path = Path::new(rel_path);
    let ok = !rel_path.is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if ok {
        Ok(())
    } else {
        Err(Error::Git(format!("path escapes the worktree: {rel_path}")))
    }
}

/// Reads the first spec in `specs` that resolves to a blob. Size-checks
/// before reading content so an oversize blob is never materialized.
fn read_blob_chain(worktree: &Path, specs: &[&str]) -> Result<Side, Error> {
    for spec in specs {
        let Some(size_out) = git_stdout_ok(worktree, &["cat-file", "-s", spec]) else {
            continue;
        };
        let size: u64 = String::from_utf8_lossy(&size_out)
            .trim()
            .parse()
            .map_err(|_| Error::Git(format!("unparseable blob size for {spec}")))?;
        if size > MAX_DIFF_CONTENT_BYTES {
            return Ok(Side {
                bytes: None,
                size,
                exists: true,
                too_large: true,
            });
        }
        let Some(bytes) = git_stdout_ok(worktree, &["cat-file", "blob", spec]) else {
            continue;
        };
        return Ok(Side {
            size: bytes.len() as u64,
            bytes: Some(bytes),
            exists: true,
            too_large: false,
        });
    }
    Ok(Side::default())
}

/// Reads the working-tree side from disk.
fn read_worktree_file(path: &Path) -> Side {
    let Ok(meta) = std::fs::metadata(path) else {
        return Side::default();
    };
    if !meta.is_file() {
        return Side::default();
    }
    if meta.len() > MAX_DIFF_CONTENT_BYTES {
        return Side {
            bytes: None,
            size: meta.len(),
            exists: true,
            too_large: true,
        };
    }
    match std::fs::read(path) {
        Ok(bytes) => Side {
            size: bytes.len() as u64,
            bytes: Some(bytes),
            exists: true,
            too_large: false,
        },
        Err(_) => Side::default(),
    }
}

fn sniff_binary(side: &Side) -> bool {
    side.bytes
        .as_deref()
        .is_some_and(|b| b[..b.len().min(BINARY_SNIFF_BYTES)].contains(&0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::tests::{git, repo_fixture};

    #[test]
    fn unstaged_modification_diffs_index_vs_worktree() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("tracked.txt"), "one\ntwo\nthree\nfour\n").unwrap();

        let d = file_diff(repo.path(), "tracked.txt", DiffSide::Unstaged, None).unwrap();
        assert_eq!(d.old_content.as_deref(), Some("one\ntwo\nthree\n"));
        assert_eq!(d.new_content.as_deref(), Some("one\ntwo\nthree\nfour\n"));
        assert!(d.old_exists && d.new_exists);
        assert!(!d.binary && !d.too_large);
    }

    #[test]
    fn staged_modification_diffs_head_vs_index() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("tracked.txt"), "staged\n").unwrap();
        git(repo.path(), &["add", "."]);
        std::fs::write(repo.path().join("tracked.txt"), "worktree-only\n").unwrap();

        let d = file_diff(repo.path(), "tracked.txt", DiffSide::Staged, None).unwrap();
        assert_eq!(d.old_content.as_deref(), Some("one\ntwo\nthree\n"));
        assert_eq!(d.new_content.as_deref(), Some("staged\n"));
    }

    #[test]
    fn untracked_file_has_no_old_side() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("new.txt"), "fresh\n").unwrap();

        let d = file_diff(repo.path(), "new.txt", DiffSide::Unstaged, None).unwrap();
        assert!(!d.old_exists);
        assert_eq!(d.old_content, None);
        assert_eq!(d.new_content.as_deref(), Some("fresh\n"));
    }

    #[test]
    fn deleted_file_has_no_new_side() {
        let repo = repo_fixture();
        std::fs::remove_file(repo.path().join("tracked.txt")).unwrap();

        let d = file_diff(repo.path(), "tracked.txt", DiffSide::Unstaged, None).unwrap();
        assert_eq!(d.old_content.as_deref(), Some("one\ntwo\nthree\n"));
        assert!(!d.new_exists);
        assert_eq!(d.new_content, None);
    }

    #[test]
    fn staged_rename_reads_head_at_orig_path() {
        let repo = repo_fixture();
        git(repo.path(), &["mv", "tracked.txt", "renamed.txt"]);

        let d = file_diff(
            repo.path(),
            "renamed.txt",
            DiffSide::Staged,
            Some("tracked.txt"),
        )
        .unwrap();
        assert_eq!(d.old_content.as_deref(), Some("one\ntwo\nthree\n"));
        assert_eq!(d.new_content.as_deref(), Some("one\ntwo\nthree\n"));
        assert_eq!(d.orig_path.as_deref(), Some("tracked.txt"));
    }

    #[test]
    fn conflicted_file_falls_back_to_ours_baseline() {
        let repo = repo_fixture();
        git(repo.path(), &["checkout", "-b", "feature"]);
        std::fs::write(repo.path().join("tracked.txt"), "feature\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "feature"]);
        git(repo.path(), &["checkout", "-"]);
        std::fs::write(repo.path().join("tracked.txt"), "main\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "main"]);
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["merge", "feature"])
            .output()
            .unwrap();

        let d = file_diff(repo.path(), "tracked.txt", DiffSide::Unstaged, None).unwrap();
        // Old side = :2: (ours); new side = the conflict-marked working file.
        assert_eq!(d.old_content.as_deref(), Some("main\n"));
        let markers = d.new_content.unwrap();
        assert!(
            markers.contains("<<<<<<<"),
            "worktree side keeps markers: {markers}"
        );
    }

    #[test]
    fn binary_content_is_withheld() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("bin.dat"), [0u8, 159, 146, 150, 0]).unwrap();
        git(repo.path(), &["add", "bin.dat"]);

        let d = file_diff(repo.path(), "bin.dat", DiffSide::Staged, None).unwrap();
        assert!(d.binary);
        assert_eq!(d.old_content, None);
        assert_eq!(d.new_content, None);
        assert_eq!(d.new_size, 5);
    }

    #[test]
    fn oversize_content_is_withheld_with_sizes() {
        let repo = repo_fixture();
        let big = "a".repeat((MAX_DIFF_CONTENT_BYTES + 1) as usize);
        std::fs::write(repo.path().join("big.txt"), &big).unwrap();

        let d = file_diff(repo.path(), "big.txt", DiffSide::Unstaged, None).unwrap();
        assert!(d.too_large);
        assert_eq!(d.new_content, None);
        assert_eq!(d.new_size, MAX_DIFF_CONTENT_BYTES + 1);
        assert!(d.new_exists);
    }

    #[test]
    fn oversize_blob_is_never_materialized() {
        let repo = repo_fixture();
        let big = "b".repeat((MAX_DIFF_CONTENT_BYTES + 1) as usize);
        std::fs::write(repo.path().join("big.txt"), &big).unwrap();
        git(repo.path(), &["add", "big.txt"]);

        let d = file_diff(repo.path(), "big.txt", DiffSide::Staged, None).unwrap();
        assert!(d.too_large);
        assert_eq!(d.old_content, None);
        assert_eq!(d.new_content, None);
    }

    #[test]
    fn path_traversal_is_rejected() {
        let repo = repo_fixture();
        for bad in ["../secrets", "/etc/passwd", "a/../../b", ""] {
            assert!(
                file_diff(repo.path(), bad, DiffSide::Unstaged, None).is_err(),
                "{bad} must be rejected"
            );
        }
        assert!(file_diff(repo.path(), "ok.txt", DiffSide::Unstaged, Some("../x")).is_err());
    }
}
