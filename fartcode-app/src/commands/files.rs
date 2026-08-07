//! Workspace file commands (E4-05): the diff editor's save path. Reads and
//! tree listing land here with E5.

use std::sync::Arc;

use tauri::State;

use crate::app::App;
use crate::commands::git::workspace_path;

/// Writes `content` to `<worktree>/<path>` for an existing workspace.
/// Containment (no absolute, no `..`, no symlink escapes) is enforced in
/// `fartcode_core::files` — this command never writes outside the worktree.
#[tauri::command]
pub fn write_workspace_file(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    path: String,
    content: String,
) -> Result<(), String> {
    let worktree = workspace_path(&app, &workspace_id)?;
    fartcode_core::files::write_file(&worktree, &path, &content).map_err(String::from)
}
