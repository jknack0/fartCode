//! Workspace file commands (E4-05): the diff editor's save path. Reads and
//! tree listing land here with E5.

use std::sync::Arc;

use tauri::State;

use crate::app::App;
use crate::commands::git::workspace_path;
use crate::commands::off_main_thread;

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

/// Reads one worktree file as UTF-8 for the editor (E5-02). Containment in
/// `fartcode_core::files::read_file`; async per the #80 main-thread rule.
#[tauri::command]
pub async fn read_workspace_file(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    path: String,
) -> Result<String, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        fartcode_core::files::read_file(&worktree, &path).map_err(String::from)
    })
    .await
}

/// One file-tree row (E5-01).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryDto {
    pub name: String,
    pub is_dir: bool,
}

/// Lists a directory inside a workspace's worktree for the file tree
/// (E5-01). `path` is worktree-relative; empty = root. Containment and the
/// hidden-dir filter (node_modules/.git/build output) live in
/// `fartcode_core::files::list_dir`.
#[tauri::command]
pub async fn list_workspace_dir(
    app: State<'_, Arc<App>>,
    workspace_id: String,
    path: String,
) -> Result<Vec<DirEntryDto>, String> {
    let app = app.inner().clone();
    off_main_thread(move || {
        let worktree = workspace_path(&app, &workspace_id)?;
        let entries = fartcode_core::files::list_dir(&worktree, &path).map_err(String::from)?;
        Ok(entries
            .into_iter()
            .map(|e| DirEntryDto {
                name: e.name,
                is_dir: e.is_dir,
            })
            .collect())
    })
    .await
}
