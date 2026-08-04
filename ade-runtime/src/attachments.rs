//! Per-conversation attachments directory under app data (E2-11-2).

use std::path::{Path, PathBuf};

use crate::error::Error;

/// The attachments dir for one conversation: `<app_data>/attachments/<id>`.
/// The conversation id is sanitized (path segments / `..` rejected) before
/// joining.
pub fn attachments_dir(app_data: &Path, conversation_id: &str) -> Result<PathBuf, Error> {
    let id = conversation_id.trim();
    if id.is_empty()
        || id.contains(['/', '\\'])
        || id.split('.').any(|seg| seg == "..")
        || id == "."
    {
        return Err(Error::Protocol(format!(
            "invalid conversation id for attachments: {conversation_id:?}"
        )));
    }
    Ok(app_data.join("attachments").join(id))
}

/// Creates the attachments dir (and parents) if missing.
pub fn ensure_attachments_dir(app_data: &Path, conversation_id: &str) -> Result<PathBuf, Error> {
    let dir = attachments_dir(app_data, conversation_id)?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_creates_the_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = ensure_attachments_dir(tmp.path(), "conv-123").unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with("attachments/conv-123"));
    }

    #[test]
    fn rejects_traversal_and_separators() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(attachments_dir(tmp.path(), "../evil").is_err());
        assert!(attachments_dir(tmp.path(), "a/b").is_err());
        assert!(attachments_dir(tmp.path(), "a\\b").is_err());
        assert!(attachments_dir(tmp.path(), ".").is_err());
        assert!(attachments_dir(tmp.path(), "  ").is_err());
    }
}
