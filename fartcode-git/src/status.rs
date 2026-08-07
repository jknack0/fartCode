//! Worktree-scoped status engine (E4-02, #42).
//!
//! Port of the reference status path (`git-worktree.ts computeStatus` +
//! `parsers/status-parser.ts`): one `git --no-optional-locks status
//! --porcelain=v2 -z -uall` pass (`--no-optional-locks` so status never
//! writes the index — the E4-01 watcher would see that write and loop),
//! paired with staged/unstaged `diff --numstat -z` for per-file line
//! counts. Entries split into staged (x column) and unstaged (y column /
//! untracked) lists; conflicts (`u` records) appear in both, matching the
//! reference. Snapshots exceeding [`MAX_STATUS_FILES`] return
//! `truncated: true` with empty lists — the UI shows a "too many changes"
//! notice instead of rendering 10k rows.
//!
//! Paths are worktree-relative throughout (the UI is workspace-scoped;
//! `diff::file_diff` takes the same relative paths back).

use std::collections::HashMap;
use std::path::Path;

use fartcode_core::Error;
use serde::Serialize;

use crate::diff::MAX_DIFF_CONTENT_BYTES;
use crate::{git_stdout, git_stdout_ok};

/// Snapshot cap, after which the UI degrades (reference `MAX_STATUS_FILES`).
pub const MAX_STATUS_FILES: usize = 10_000;

/// PRD E4 status icon classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Conflicted,
}

/// One file row in the Changes sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChange {
    /// Worktree-relative path (the rename target for renames).
    pub path: String,
    /// Rename source, present on renamed entries.
    pub orig_path: Option<String>,
    pub status: ChangeStatus,
    pub additions: u32,
    pub deletions: u32,
}

/// Status snapshot for one worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    pub staged: Vec<GitChange>,
    pub unstaged: Vec<GitChange>,
    /// Totals across `staged` (commit-card counters).
    pub staged_additions: u32,
    pub staged_deletions: u32,
    /// True when the worktree has more than [`MAX_STATUS_FILES`] entries;
    /// the lists are empty then.
    pub truncated: bool,
}

/// One parsed `--porcelain=v2 -z` record (crate-visible for the diff side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    /// Staged column (X), `' '` when unmodified ('.' normalized).
    pub x: char,
    /// Unstaged column (Y), `' '` when unmodified.
    pub y: char,
    /// Worktree-relative path (rename target for `2` records).
    pub path: String,
    /// Rename source (`2` records only).
    pub orig_path: Option<String>,
}

/// Computes the status snapshot for `worktree`.
pub fn status(worktree: &Path) -> Result<StatusSnapshot, Error> {
    let raw = git_stdout(
        worktree,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "-z",
            "-uall",
        ],
    )?;
    let entries = parse_porcelain_v2(&raw);
    if entries.len() > MAX_STATUS_FILES {
        return Ok(StatusSnapshot {
            staged: Vec::new(),
            unstaged: Vec::new(),
            staged_additions: 0,
            staged_deletions: 0,
            truncated: true,
        });
    }

    // Numstat failures are tolerated (reference `.catch(() => '')`): rows
    // render with 0/0 counts rather than failing the whole snapshot.
    let staged_numstat = git_stdout_ok(worktree, &["diff", "--numstat", "--cached", "-z"])
        .map(|b| parse_numstat(&b))
        .unwrap_or_default();
    let unstaged_numstat = git_stdout_ok(worktree, &["diff", "--numstat", "-z"])
        .map(|b| parse_numstat(&b))
        .unwrap_or_default();

    Ok(build_snapshot(
        worktree,
        &entries,
        &staged_numstat,
        &unstaged_numstat,
    ))
}

/// Maps a two-char `XY` code to the file's status class (reference
/// `mapGitChangeStatus`).
fn map_status(x: char, y: char) -> ChangeStatus {
    let (xu, yu) = (x, y);
    if xu == 'U' || yu == 'U' || (xu == 'A' && yu == 'A') || (xu == 'D' && yu == 'D') {
        ChangeStatus::Conflicted
    } else if xu == 'A' || yu == 'A' || xu == '?' || yu == '?' {
        ChangeStatus::Added
    } else if xu == 'D' || yu == 'D' {
        ChangeStatus::Deleted
    } else if xu == 'R' || yu == 'R' {
        ChangeStatus::Renamed
    } else {
        ChangeStatus::Modified
    }
}

fn build_snapshot(
    worktree: &Path,
    entries: &[FileStatus],
    staged_numstat: &Numstat,
    unstaged_numstat: &Numstat,
) -> StatusSnapshot {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();

    for entry in entries {
        let status = map_status(entry.x, entry.y);

        // Staged side: the X column carries a change.
        if entry.x != ' ' && entry.x != '?' {
            let (additions, deletions) =
                staged_numstat.get(&entry.path).copied().unwrap_or_default();
            staged.push(GitChange {
                path: entry.path.clone(),
                orig_path: entry.orig_path.clone(),
                status,
                additions,
                deletions,
            });
        }

        // Unstaged side: untracked, or the Y column carries a change.
        let is_untracked = entry.x == '?' && entry.y == '?';
        let has_unstaged = entry.y != ' ' && entry.y != '?';
        if !is_untracked && !has_unstaged {
            continue;
        }
        let (mut additions, deletions) = unstaged_numstat
            .get(&entry.path)
            .copied()
            .unwrap_or_default();
        if additions == 0 && deletions == 0 && is_untracked {
            additions = count_lines_capped(&worktree.join(&entry.path));
        }
        unstaged.push(GitChange {
            path: entry.path.clone(),
            orig_path: None,
            status,
            additions,
            deletions,
        });
    }

    let staged_additions = staged.iter().map(|c| c.additions).sum();
    let staged_deletions = staged.iter().map(|c| c.deletions).sum();
    StatusSnapshot {
        staged,
        unstaged,
        staged_additions,
        staged_deletions,
        truncated: false,
    }
}

/// Line count for an untracked file, `0` when unreadable or larger than the
/// diff content cap (reference `countFileLines` with `maxBytes`).
fn count_lines_capped(path: &Path) -> u32 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    if meta.len() > MAX_DIFF_CONTENT_BYTES {
        return 0;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    if bytes.ends_with(b"\n") {
        newlines
    } else {
        newlines + 1
    }
}

/// Parses `git status --porcelain=v2 -z` output. Directory rows (trailing
/// `/`) are skipped; unknown record kinds are ignored.
pub fn parse_porcelain_v2(raw: &[u8]) -> Vec<FileStatus> {
    let mut out = Vec::new();
    let mut records = raw
        .split(|b| *b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned());

    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let entry = match record.as_bytes()[0] {
            // `1 XY sub mH mI mW hH hI <path>`
            b'1' => split_fields(&record, 8).map(|(xy, path)| FileStatus {
                x: xy.0,
                y: xy.1,
                path,
                orig_path: None,
            }),
            // `2 XY sub mH mI mW hH hI Xscore <new>\0<orig>`
            b'2' => {
                let head = split_fields(&record, 9);
                let orig = records.next();
                match (head, orig) {
                    (Some((xy, path)), Some(orig_path)) if !orig_path.is_empty() => {
                        Some(FileStatus {
                            x: xy.0,
                            y: xy.1,
                            path,
                            orig_path: Some(orig_path),
                        })
                    }
                    _ => None,
                }
            }
            // `u XY sub m1 m2 m3 mW h1 h2 h3 <path>`
            b'u' => split_fields(&record, 10).map(|(xy, path)| FileStatus {
                x: xy.0,
                y: xy.1,
                path,
                orig_path: None,
            }),
            // `? <path>`
            b'?' => record.strip_prefix("? ").map(|path| FileStatus {
                x: '?',
                y: '?',
                path: path.to_string(),
                orig_path: None,
            }),
            // `! <path>` (ignored entries) and anything unknown.
            _ => None,
        };
        if let Some(entry) = entry {
            if !entry.path.is_empty() && !entry.path.ends_with('/') {
                out.push(entry);
            }
        }
    }
    out
}

/// Splits a porcelain record into `field_count` space-separated prefix
/// fields + the remaining path (which may itself contain spaces). Returns
/// the normalized `XY` pair (field 1) and the path.
fn split_fields(record: &str, field_count: usize) -> Option<((char, char), String)> {
    let mut parts = record.splitn(field_count + 1, ' ');
    let mut xy = None;
    for i in 0..field_count {
        let field = parts.next()?;
        if i == 1 {
            let mut chars = field.chars();
            let x = normalize(chars.next()?);
            let y = normalize(chars.next()?);
            xy = Some((x, y));
        }
    }
    let path = parts.next()?;
    Some((xy?, path.to_string()))
}

fn normalize(c: char) -> char {
    if c == '.' {
        ' '
    } else {
        c
    }
}

/// additions/deletions keyed by (new) path.
type Numstat = HashMap<String, (u32, u32)>;

/// Parses `git diff --numstat -z`. Records: `ADD\tDEL\t<path>` or, for
/// renames, `ADD\tDEL\t` followed by two records `<orig>` then `<new>`
/// (keyed by the new path). Binary files report `-\t-` and count as 0/0.
pub fn parse_numstat(raw: &[u8]) -> Numstat {
    let mut map = Numstat::new();
    let mut records = raw
        .split(|b| *b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned());

    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let mut parts = record.splitn(3, '\t');
        let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let counts = (add.parse().unwrap_or(0), del.parse().unwrap_or(0));
        if path.is_empty() {
            // Rename: two follow-up records, orig then new.
            let _orig = records.next();
            let Some(new_path) = records.next() else {
                continue;
            };
            map.insert(new_path, counts);
        } else {
            map.insert(path.to_string(), counts);
        }
    }
    map
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::process::Command;

    pub(crate) fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .output()
            .expect("git spawns");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    pub(crate) fn repo_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init"]);
        std::fs::write(tmp.path().join("tracked.txt"), "one\ntwo\nthree\n").unwrap();
        git(tmp.path(), &["add", "."]);
        git(tmp.path(), &["commit", "-m", "init"]);
        tmp
    }

    fn by_path<'a>(list: &'a [GitChange], path: &str) -> &'a GitChange {
        list.iter()
            .find(|c| c.path == path)
            .unwrap_or_else(|| panic!("{path} not in {list:?}"))
    }

    #[test]
    fn clean_worktree_is_empty() {
        let repo = repo_fixture();
        let snap = status(repo.path()).unwrap();
        assert!(snap.staged.is_empty());
        assert!(snap.unstaged.is_empty());
        assert!(!snap.truncated);
    }

    #[test]
    fn modified_splits_staged_and_unstaged() {
        let repo = repo_fixture();
        // Stage one modification, then modify again: MM → both lists.
        std::fs::write(repo.path().join("tracked.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        git(repo.path(), &["add", "."]);
        std::fs::write(
            repo.path().join("tracked.txt"),
            "one\ntwo\nthree\nfour\nfive\n",
        )
        .unwrap();

        let snap = status(repo.path()).unwrap();
        let staged = by_path(&snap.staged, "tracked.txt");
        assert_eq!(staged.status, ChangeStatus::Modified);
        assert_eq!((staged.additions, staged.deletions), (1, 0));
        let unstaged = by_path(&snap.unstaged, "tracked.txt");
        assert_eq!(unstaged.status, ChangeStatus::Modified);
        assert_eq!((unstaged.additions, unstaged.deletions), (1, 0));
        assert_eq!(snap.staged_additions, 1);
        assert_eq!(snap.staged_deletions, 0);
    }

    #[test]
    fn untracked_counts_lines_and_staged_added() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("untracked.txt"), "a\nb\nc").unwrap();
        std::fs::write(repo.path().join("staged-new.txt"), "x\n").unwrap();
        git(repo.path(), &["add", "staged-new.txt"]);

        let snap = status(repo.path()).unwrap();
        let untracked = by_path(&snap.unstaged, "untracked.txt");
        assert_eq!(untracked.status, ChangeStatus::Added);
        assert_eq!(untracked.additions, 3, "unterminated last line counts");
        let added = by_path(&snap.staged, "staged-new.txt");
        assert_eq!(added.status, ChangeStatus::Added);
        assert_eq!(added.additions, 1);
        assert!(snap.unstaged.iter().all(|c| c.path != "staged-new.txt"));
    }

    #[test]
    fn deletions_report_on_the_right_side() {
        let repo = repo_fixture();
        std::fs::remove_file(repo.path().join("tracked.txt")).unwrap();
        let snap = status(repo.path()).unwrap();
        let unstaged = by_path(&snap.unstaged, "tracked.txt");
        assert_eq!(unstaged.status, ChangeStatus::Deleted);
        assert!(snap.staged.is_empty());

        git(repo.path(), &["rm", "tracked.txt"]);
        let snap = status(repo.path()).unwrap();
        let staged = by_path(&snap.staged, "tracked.txt");
        assert_eq!(staged.status, ChangeStatus::Deleted);
        assert_eq!(staged.deletions, 3);
        assert!(snap.unstaged.is_empty());
    }

    #[test]
    fn rename_carries_orig_path() {
        let repo = repo_fixture();
        git(repo.path(), &["mv", "tracked.txt", "renamed.txt"]);
        let snap = status(repo.path()).unwrap();
        let staged = by_path(&snap.staged, "renamed.txt");
        assert_eq!(staged.status, ChangeStatus::Renamed);
        assert_eq!(staged.orig_path.as_deref(), Some("tracked.txt"));
        assert!(snap.unstaged.is_empty());
    }

    #[test]
    fn merge_conflict_reports_conflicted_in_both_lists() {
        let repo = repo_fixture();
        git(repo.path(), &["checkout", "-b", "feature"]);
        std::fs::write(repo.path().join("tracked.txt"), "feature\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "feature"]);
        git(repo.path(), &["checkout", "-"]);
        std::fs::write(repo.path().join("tracked.txt"), "main\n").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "main"]);
        // The merge fails with conflicts — run without asserting success.
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["merge", "feature"])
            .output()
            .unwrap();

        let snap = status(repo.path()).unwrap();
        assert_eq!(
            by_path(&snap.staged, "tracked.txt").status,
            ChangeStatus::Conflicted
        );
        assert_eq!(
            by_path(&snap.unstaged, "tracked.txt").status,
            ChangeStatus::Conflicted
        );
    }

    #[test]
    fn paths_with_spaces_survive() {
        let repo = repo_fixture();
        std::fs::write(repo.path().join("has space.txt"), "hi\n").unwrap();
        git(repo.path(), &["add", "."]);
        let snap = status(repo.path()).unwrap();
        assert_eq!(
            by_path(&snap.staged, "has space.txt").status,
            ChangeStatus::Added
        );
    }

    #[test]
    fn non_repo_errors() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(status(tmp.path()).is_err());
    }

    #[test]
    fn parser_caps_at_max_files() {
        // Synthetic porcelain stream: cheap to build, no 10k-file fixture.
        let mut raw = Vec::new();
        for i in 0..(MAX_STATUS_FILES + 1) {
            raw.extend_from_slice(format!("? f{i}.txt\0").as_bytes());
        }
        let entries = parse_porcelain_v2(&raw);
        assert_eq!(entries.len(), MAX_STATUS_FILES + 1);
        // The runner turns an over-cap parse into a truncated snapshot; the
        // split itself is covered by the fixture tests.
    }
    #[test]
    fn parse_numstat_handles_renames_and_binary() {
        let raw = b"3\t1\ta.txt\0-\t-\tbin.dat\x002\t0\t\0old.txt\0new.txt\0";
        let ns = parse_numstat(raw);
        assert_eq!(ns.get("a.txt"), Some(&(3, 1)));
        assert_eq!(ns.get("bin.dat"), Some(&(0, 0)));
        assert_eq!(ns.get("new.txt"), Some(&(2, 0)));
        assert!(!ns.contains_key("old.txt"));
    }

    #[test]
    fn porcelain_parser_normalizes_and_routes_kinds() {
        let raw = b"1 .M N... 100644 100644 100644 aaaa bbbb tracked.txt\0\
2 R. N... 100644 100644 100644 aaaa bbbb R100 new.txt\0old.txt\0\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\0\
? untracked.txt\0\
! ignored.txt\0";
        let entries = parse_porcelain_v2(raw);
        assert_eq!(entries.len(), 4, "ignored entries are skipped: {entries:?}");
        assert_eq!(
            entries[0],
            FileStatus {
                x: ' ',
                y: 'M',
                path: "tracked.txt".into(),
                orig_path: None
            }
        );
        assert_eq!(
            entries[1],
            FileStatus {
                x: 'R',
                y: ' ',
                path: "new.txt".into(),
                orig_path: Some("old.txt".into())
            }
        );
        assert_eq!(
            entries[2],
            FileStatus {
                x: 'U',
                y: 'U',
                path: "conflict.txt".into(),
                orig_path: None
            }
        );
        assert_eq!(
            entries[3],
            FileStatus {
                x: '?',
                y: '?',
                path: "untracked.txt".into(),
                orig_path: None
            }
        );
    }
}
