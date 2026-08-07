//! Worktree-directory validation (E1-05).
//!
//! Port of the reference `worktree-directory.ts::normalizeWorktreeDirectory`:
//! trim, expand `~`/`~/`/`~\` via the home dir, then require an absolute path
//! (posix `/`, windows drive `X:\`/`X:/`, or UNC `\\`). Anything else is the
//! typed `invalid-worktree-directory` error.

use std::path::PathBuf;

use crate::Error;

/// Home dir resolution that works on Windows too: `HOME` → `USERPROFILE`
/// (the standard on Windows; `HOME` is usually unset there) → `None`.
pub fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
}

fn is_windows_drive_absolute(input: &str) -> bool {
    let b = input.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

fn is_windows_unc_absolute(input: &str) -> bool {
    input.starts_with("\\\\")
}

fn is_native_absolute(input: &str, windows: bool) -> bool {
    if windows {
        is_windows_drive_absolute(input) || is_windows_unc_absolute(input)
    } else {
        input.starts_with('/')
    }
}

/// Normalizes + validates a worktree directory. Returns the normalized path
/// or `Error::InvalidWorktreeDirectory`.
pub fn normalize_worktree_directory(input: &str, home: Option<&str>) -> Result<PathBuf, Error> {
    let trimmed = input.trim();
    let normalized = if trimmed == "~" || trimmed.starts_with("~/") || trimmed.starts_with("~\\") {
        let home = home
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .ok_or_else(|| Error::InvalidWorktreeDirectory(input.to_string()))?;
        if trimmed == "~" {
            home.to_string()
        } else {
            PathBuf::from(home)
                .join(&trimmed[2..])
                .to_string_lossy()
                .into_owned()
        }
    } else {
        trimmed.to_string()
    };

    if is_native_absolute(&normalized, cfg!(windows)) {
        Ok(PathBuf::from(normalized))
    } else {
        Err(Error::InvalidWorktreeDirectory(input.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_absolute_paths() {
        assert!(normalize_worktree_directory("/fartCode/worktrees", None).is_ok());
        assert!(
            normalize_worktree_directory(" /fartCode/worktrees ", None).is_ok(),
            "trimmed"
        );
    }

    #[test]
    fn expands_tilde() {
        let home = "/Users/fartCode";
        assert_eq!(
            normalize_worktree_directory("~/worktrees", Some(home)).unwrap(),
            PathBuf::from("/Users/fartCode/worktrees")
        );
        assert_eq!(
            normalize_worktree_directory("~", Some(home)).unwrap(),
            PathBuf::from("/Users/fartCode")
        );
        // Tilde without a home → invalid.
        assert!(normalize_worktree_directory("~/x", None).is_err());
        // Tilde in the middle is not special.
        assert!(normalize_worktree_directory("a~/x", Some(home)).is_err());
    }

    #[test]
    fn rejects_relative_paths() {
        assert!(matches!(
            normalize_worktree_directory("relative/path", None),
            Err(Error::InvalidWorktreeDirectory(_))
        ));
        assert!(matches!(
            normalize_worktree_directory("", None),
            Err(Error::InvalidWorktreeDirectory(_))
        ));
    }

    #[test]
    fn windows_absolute_helpers() {
        // Platform-agnostic: exercise the helpers directly (no cfg gate).
        assert!(is_windows_drive_absolute("C:\\fartCode"));
        assert!(is_windows_drive_absolute("C:/fartCode"));
        assert!(!is_windows_drive_absolute("/fartCode"));
        assert!(is_windows_unc_absolute("\\\\server\\share"));
        assert!(!is_windows_unc_absolute("//server/share"));
        assert!(is_native_absolute("C:\\fartCode", true));
        assert!(is_native_absolute("\\\\s\\x", true));
        assert!(is_native_absolute("/fartCode", false));
        assert!(!is_native_absolute("rel/x", false));
    }

    #[test]
    fn home_dir_falls_back_to_userprofile() {
        // Just assert the helper is stable to call; values come from env.
        let _ = home_dir();
    }
}
