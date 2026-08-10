//! SFTP filesystem layer (E12-02).
//!
//! Remote file operations over SFTP: browse, read, write, stat,
//! realpath, mkdir, remove. All operations are path-constrained to
//! a workspace root directory.

use std::path::{Path, PathBuf};

use fartcode_core::Error;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{OpenFlags, StatusCode};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tracing::debug;

// ── File entry ───────────────────────────────────────────────

/// A single file or directory entry from an SFTP listing.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Relative path from the workspace root.
    pub path: String,
    /// File name (last component).
    pub name: String,
    /// Entry type: file, dir, or symlink.
    pub kind: FileKind,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time (unix seconds).
    pub mtime: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
}

/// Result of a read operation.
#[derive(Debug)]
pub struct ReadResult {
    /// File content as UTF-8 string.
    pub content: String,
    /// True if the file was larger than max_bytes.
    pub truncated: bool,
    /// Total file size on the remote side.
    pub total_size: u64,
}

// ── SFTP session ─────────────────────────────────────────────

/// SFTP session bound to a workspace root directory.
///
/// All file paths are resolved relative to `root` and validated
/// to not escape it.
pub struct RemoteSftp {
    session: SftpSession,
    root: PathBuf,
}

impl RemoteSftp {
    /// Create a new SFTP session from an SSH channel and a workspace root.
    pub async fn new(
        channel: russh::Channel<russh::client::Msg>,
        root: &str,
    ) -> Result<Self, Error> {
        let stream = channel.into_stream();
        let session = SftpSession::new(stream)
            .await
            .map_err(|e| Error::SshSftp(format!("init sftp: {e}")))?;

        let canonical = session
            .canonicalize(root)
            .await
            .map_err(|e| Error::SshSftp(format!("canonicalize root {root}: {e}")))?;

        debug!(root = %canonical, "SFTP session opened");
        Ok(Self {
            session,
            root: PathBuf::from(canonical),
        })
    }

    // ── Public API ───────────────────────────────────────────

    /// List directory contents.
    ///
    /// Returns entries sorted: directories first, then files, both
    /// alphabetically. Hidden files (dot-prefixed) are skipped unless
    /// `include_hidden` is true.
    pub async fn list(
        &mut self,
        path: &str,
        include_hidden: bool,
    ) -> Result<Vec<FileEntry>, Error> {
        let full = self.resolve(path)?;
        let p = full.to_string_lossy().to_string();
        let mut entries = self
            .session
            .read_dir(&p)
            .await
            .map_err(|e| Error::SshSftp(format!("readdir {path}: {e}")))?;

        let mut result: Vec<FileEntry> = Vec::new();
        for entry in &mut entries {
            let name = entry.file_name().to_string();
            if !include_hidden && name.starts_with('.') {
                continue;
            }
            let attrs = entry.metadata();
            let kind = if attrs.is_dir() {
                FileKind::Dir
            } else if attrs.is_symlink() {
                FileKind::Symlink
            } else {
                FileKind::File
            };
            result.push(FileEntry {
                path: self.relative(&full, &name),
                name,
                kind,
                size: attrs.size.unwrap_or(0),
                mtime: attrs.mtime.unwrap_or(0) as u64,
            });
        }

        result.sort_by(|a, b| match (&a.kind, &b.kind) {
            (FileKind::Dir, FileKind::Dir)
            | (FileKind::File, FileKind::File)
            | (FileKind::Symlink, FileKind::Symlink) => a.name.cmp(&b.name),
            (FileKind::Dir, _) => std::cmp::Ordering::Less,
            (_, FileKind::Dir) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(result)
    }

    /// Read a file, capped at `max_bytes` (default 200KB).
    ///
    /// Returns the content as UTF-8, a truncated flag, and the total
    /// remote file size. Rejects paths over 100MB to prevent OOM.
    pub async fn read(&mut self, path: &str, max_bytes: Option<u64>) -> Result<ReadResult, Error> {
        let full = self.resolve(path)?;
        let p = full.to_string_lossy().to_string();

        let mut file = self
            .session
            .open_with_flags(&p, OpenFlags::READ)
            .await
            .map_err(|e| Error::SshSftp(format!("open {path}: {e}")))?;

        let total_size = file
            .metadata()
            .await
            .map_err(|e| Error::SshSftp(format!("fstat {path}: {e}")))?
            .size
            .unwrap_or(0);

        let cap = max_bytes.unwrap_or(200 * 1024).min(100 * 1024 * 1024);
        let read_size = total_size.min(cap) as usize;

        if read_size == 0 {
            file.shutdown().await.ok();
            return Ok(ReadResult {
                content: String::new(),
                truncated: total_size > cap,
                total_size,
            });
        }

        let mut buf = vec![0u8; read_size];
        file.rewind()
            .await
            .map_err(|e| Error::SshSftp(format!("seek {path}: {e}")))?;
        file.read_exact(&mut buf)
            .await
            .map_err(|e| Error::SshSftp(format!("read {path}: {e}")))?;

        file.shutdown().await.ok();

        let content = String::from_utf8(buf)
            .map_err(|e| Error::SshSftp(format!("invalid utf-8 in {path}: {e}")))?;

        Ok(ReadResult {
            content,
            truncated: total_size > cap,
            total_size,
        })
    }

    /// Write content to a file. Creates parent directories recursively.
    pub async fn write(&mut self, path: &str, content: &[u8]) -> Result<(), Error> {
        let full = self.resolve(path)?;

        if let Some(parent) = full.parent() {
            self.ensure_dir(parent).await?;
        }

        let p = full.to_string_lossy().to_string();
        let mut file = self
            .session
            .open_with_flags(
                &p,
                OpenFlags::CREATE | OpenFlags::WRITE | OpenFlags::TRUNCATE,
            )
            .await
            .map_err(|e| Error::SshSftp(format!("open {path}: {e}")))?;

        file.write_all(content)
            .await
            .map_err(|e| Error::SshSftp(format!("write {path}: {e}")))?;

        file.shutdown().await.ok();
        debug!(path = %p, bytes = content.len(), "SFTP write done");
        Ok(())
    }

    /// Get file or directory metadata.
    pub async fn stat(&mut self, path: &str) -> Result<Option<FileEntry>, Error> {
        let full = self.resolve(path)?;
        let p = full.to_string_lossy().to_string();
        match self.session.metadata(&p).await {
            Ok(attrs) => {
                let name = full
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let kind = if attrs.is_dir() {
                    FileKind::Dir
                } else if attrs.is_symlink() {
                    FileKind::Symlink
                } else {
                    FileKind::File
                };
                Ok(Some(FileEntry {
                    path: path.to_string(),
                    name,
                    kind,
                    size: attrs.size.unwrap_or(0),
                    mtime: attrs.mtime.unwrap_or(0) as u64,
                }))
            }
            Err(e) if is_no_such_file(&e) => Ok(None),
            Err(e) => Err(Error::SshSftp(format!("stat {path}: {e}"))),
        }
    }

    /// Resolve a path to its canonical (real) path on the remote host.
    pub async fn realpath(&mut self, path: &str) -> Result<String, Error> {
        let full = self.resolve(path)?;
        self.session
            .canonicalize(full.to_string_lossy().to_string())
            .await
            .map_err(|e| Error::SshSftp(format!("realpath {path}: {e}")))
    }

    /// Check if a path exists.
    pub async fn exists(&mut self, path: &str) -> Result<bool, Error> {
        Ok(self.stat(path).await?.is_some())
    }

    /// Create a directory. If `recursive`, creates missing parents.
    pub async fn mkdir(&mut self, path: &str, recursive: bool) -> Result<(), Error> {
        let full = self.resolve(path)?;
        if recursive {
            self.ensure_dir(&full).await
        } else {
            self.session
                .create_dir(full.to_string_lossy().to_string())
                .await
                .map_err(|e| Error::SshSftp(format!("mkdir {path}: {e}")))
        }
    }

    /// Remove a file or directory. For directories, `recursive` must be true.
    pub async fn remove(&mut self, path: &str, recursive: bool) -> Result<(), Error> {
        // Use an iterative stack to avoid recursive async fn.
        let mut stack: Vec<String> = vec![path.to_string()];
        let mut dirs: Vec<String> = Vec::new();

        while let Some(p) = stack.pop() {
            let entry = self
                .stat(&p)
                .await?
                .ok_or_else(|| Error::SshSftp(format!("remove {p}: not found")))?;

            match entry.kind {
                FileKind::Dir => {
                    if !recursive {
                        return Err(Error::SshSftp(format!(
                            "remove {p}: is a directory, use recursive=true"
                        )));
                    }
                    // Collect children, then push this dir to remove later.
                    let children = self.list(&p, true).await?;
                    dirs.push(p.clone());
                    for child in children {
                        let child_path = if child.name == "." || child.name == ".." {
                            continue;
                        } else if p.is_empty() || p == "." {
                            child.name.clone()
                        } else {
                            format!("{}/{}", p.trim_end_matches('/'), child.name)
                        };
                        stack.push(child_path);
                    }
                }
                _ => {
                    self.session
                        .remove_file(self.resolve(&p)?.to_string_lossy().to_string())
                        .await
                        .map_err(|e| Error::SshSftp(format!("unlink {p}: {e}")))?;
                }
            }
        }

        // Remove directories bottom-up (reverse order).
        for d in dirs.into_iter().rev() {
            self.session
                .remove_dir(self.resolve(&d)?.to_string_lossy().to_string())
                .await
                .map_err(|e| Error::SshSftp(format!("rmdir {d}: {e}")))?;
        }

        Ok(())
    }

    // ── Path utilities ───────────────────────────────────────

    fn resolve(&self, path: &str) -> Result<PathBuf, Error> {
        let target = if path.is_empty() || path == "." {
            self.root.clone()
        } else {
            let p = Path::new(path);
            if p.is_absolute() {
                let normalized = normalize_path(p);
                if !normalized.starts_with(&self.root) {
                    return Err(Error::PathEscape(format!(
                        "absolute path {path} escapes workspace root {}",
                        self.root.display()
                    )));
                }
                normalized
            } else {
                let joined = self.root.join(p);
                let normalized = normalize_path(&joined);
                if !normalized.starts_with(&self.root) {
                    return Err(Error::PathEscape(format!(
                        "path {path} escapes workspace root {}",
                        self.root.display()
                    )));
                }
                normalized
            }
        };
        Ok(target)
    }

    fn relative(&self, full_dir: &Path, name: &str) -> String {
        let full = full_dir.join(name);
        full.strip_prefix(&self.root)
            .ok()
            .and_then(|p| {
                let s = p.to_string_lossy().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            })
            .unwrap_or_else(|| ".".to_string())
    }

    async fn ensure_dir(&mut self, dir: &Path) -> Result<(), Error> {
        if !dir.starts_with(&self.root) {
            return Err(Error::PathEscape(format!(
                "mkdir target escapes workspace root: {}",
                dir.display()
            )));
        }

        // Build path components from root down to target.
        let mut to_create: Vec<PathBuf> = Vec::new();
        let mut current = dir.to_path_buf();
        loop {
            let p = current.to_string_lossy().to_string();
            match self.session.create_dir(&p).await {
                Ok(_) => break,
                Err(ref e) if is_generic_failure(e) => {
                    // Generic Failure is ambiguous — confirm the path is an
                    // existing directory, otherwise surface the error.
                    match self.session.metadata(&p).await {
                        Ok(attrs) if attrs.is_dir() => break,
                        _ => return Err(Error::SshSftp(format!("mkdir {p}: {e}"))),
                    }
                }
                Err(ref e) if is_no_such_file(e) => {
                    to_create.push(current.clone());
                    match current.parent() {
                        Some(parent) if parent != current && parent.starts_with(&self.root) => {
                            current = parent.to_path_buf();
                        }
                        _ => {
                            return Err(Error::SshSftp(format!("mkdir {}: {}", dir.display(), e)));
                        }
                    }
                }
                Err(e) => {
                    return Err(Error::SshSftp(format!("mkdir {}: {e}", dir.display())));
                }
            }
        }
        // Create queued dirs bottom-up.
        for d in to_create.into_iter().rev() {
            self.session
                .create_dir(d.to_string_lossy().to_string())
                .await
                .map_err(|e| Error::SshSftp(format!("mkdir {}: {e}", d.display())))?;
        }
        Ok(())
    }
}

// ── Helpers ──────────────────────────────────────────────────

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(c) => components.push(c),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::RootDir => {
                components.clear();
                components.push(std::ffi::OsStr::new("/"));
            }
            std::path::Component::Prefix(p) => {
                components.push(p.as_os_str());
            }
        }
    }
    components.iter().collect()
}

fn status_code(e: &russh_sftp::client::error::Error) -> Option<StatusCode> {
    match e {
        russh_sftp::client::error::Error::Status(s) => Some(s.status_code),
        _ => None,
    }
}

fn is_no_such_file(e: &russh_sftp::client::error::Error) -> bool {
    matches!(status_code(e), Some(StatusCode::NoSuchFile))
}

/// Servers (OpenSSH included) report "mkdir on existing dir" as a generic
/// `Failure`, so callers must confirm with a stat before treating it as OK.
fn is_generic_failure(e: &russh_sftp::client::error::Error) -> bool {
    matches!(status_code(e), Some(StatusCode::Failure))
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_removes_dotdot() {
        let p = normalize_path(Path::new("/root/sub/../file.txt"));
        assert_eq!(p, PathBuf::from("/root/file.txt"));
    }

    #[test]
    fn normalize_path_removes_curdir() {
        let p = normalize_path(Path::new("/root/./file.txt"));
        assert_eq!(p, PathBuf::from("/root/file.txt"));
    }

    #[test]
    fn normalize_path_relative() {
        let p = normalize_path(Path::new("a/b/../c"));
        assert_eq!(p, PathBuf::from("a/c"));
    }

    #[test]
    fn file_kind_partial_eq() {
        assert_eq!(FileKind::File, FileKind::File);
        assert_ne!(FileKind::Dir, FileKind::File);
    }

    #[test]
    fn file_entry_debug() {
        let e = FileEntry {
            path: "test.txt".into(),
            name: "test.txt".into(),
            kind: FileKind::File,
            size: 42,
            mtime: 1000,
        };
        let _ = format!("{e:?}");
    }

    #[test]
    fn path_escape_detected() {
        let escaped = normalize_path(Path::new("/workspace/../../etc/passwd"));
        assert!(!escaped.starts_with("/workspace"));
    }

    #[test]
    fn status_code_maps_no_such_file() {
        let err = russh_sftp::client::error::Error::Status(russh_sftp::protocol::Status {
            id: 1,
            status_code: StatusCode::NoSuchFile,
            error_message: "no such file".into(),
            language_tag: "en".into(),
        });
        assert!(is_no_such_file(&err));
        assert!(!is_generic_failure(&err));
    }

    #[test]
    fn status_code_maps_generic_failure() {
        let err = russh_sftp::client::error::Error::Status(russh_sftp::protocol::Status {
            id: 2,
            status_code: StatusCode::Failure,
            error_message: "failure".into(),
            language_tag: "en".into(),
        });
        assert!(is_generic_failure(&err));
        assert!(!is_no_such_file(&err));
    }

    #[test]
    fn non_status_errors_are_not_enoent() {
        let err = russh_sftp::client::error::Error::Timeout;
        assert!(!is_no_such_file(&err));
        assert!(!is_generic_failure(&err));
        let io = russh_sftp::client::error::Error::IO("no such file".into());
        assert!(!is_no_such_file(&io));
    }

    #[test]
    fn path_inside_workspace() {
        let safe = normalize_path(Path::new("/workspace/subdir/file.txt"));
        assert!(safe.starts_with("/workspace"));
    }
}
