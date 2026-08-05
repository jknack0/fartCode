//! Classify raw watch-event paths into per-workspace effects (E4-01).
//!
//! Port of the reference classifier
//! (`packages/core/src/git/watch/classifier.ts`):
//!
//! - shared git dir (`common_dir`) `HEAD`, `refs/heads/*`, `packed-refs` →
//!   git effect for **every** workspace sharing that common dir. Shared
//!   branch refs do not identify which registered worktree is on that
//!   branch, so fan out conservatively (reference does the same).
//! - Deviation (superset): `refs/remotes/*` and `config` also fan out to the
//!   sharing workspaces — the reference tracked those as repo-level effects
//!   with no worktree consumer; our footer/status consumers (E4-08) want
//!   ahead/behind counts refreshed after fetches and remote changes.
//! - per-worktree git dir `HEAD` / `index` → git effect for that workspace
//!   only (routes linked-worktree HEAD moves and index changes precisely).
//! - any path inside the worktree not under `.git` → file effect with the
//!   worktree-relative path.
//! - everything else (`objects/`, `logs/`, hook footprints, …) → no effect.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// One registered workspace, canonical paths (see `layout::GitLayout`).
#[derive(Debug, Clone)]
pub struct WorkspaceLayout {
    pub workspace_id: String,
    pub project_id: String,
    /// Canonical worktree root.
    pub worktree: PathBuf,
    /// `(git_dir, common_dir)` when the worktree is a git repo; `None`
    /// watches working files only.
    pub git: Option<(PathBuf, PathBuf)>,
}

/// Effects for one workspace out of one debounced batch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkspaceEffects {
    /// Worktree-relative changed working files (deduped, ordered).
    pub files: BTreeSet<PathBuf>,
    /// Git metadata changed (HEAD move, index change, ref update).
    pub git: bool,
}

/// Classifies a batch of event paths against the registered layouts.
/// Returns effects keyed by workspace id; workspaces without effects are
/// absent.
pub fn classify(
    paths: &[PathBuf],
    layouts: &[WorkspaceLayout],
) -> BTreeMap<String, WorkspaceEffects> {
    let mut out: BTreeMap<String, WorkspaceEffects> = BTreeMap::new();

    for path in paths {
        for layout in layouts {
            if let Some((git_dir, common_dir)) = &layout.git {
                let shared_hit =
                    rel_inside(common_dir, path).is_some_and(|rel| is_shared_git_metadata(&rel));
                let own_hit = !shared_hit
                    && rel_inside(git_dir, path)
                        .is_some_and(|rel| rel == Path::new("HEAD") || rel == Path::new("index"));
                if shared_hit || own_hit {
                    out.entry(layout.workspace_id.clone()).or_default().git = true;
                    // A git-metadata path is never also a working file.
                    continue;
                }
            }

            if let Some(rel) = rel_inside(&layout.worktree, path) {
                if !rel.as_os_str().is_empty() && !is_dot_git_path(&rel) {
                    out.entry(layout.workspace_id.clone())
                        .or_default()
                        .files
                        .insert(rel);
                }
            }
        }
    }

    out
}

/// Shared-git-dir paths that invalidate per-worktree git state.
fn is_shared_git_metadata(rel: &Path) -> bool {
    rel == Path::new("HEAD")
        || rel == Path::new("packed-refs")
        || rel == Path::new("config")
        || rel.starts_with("refs/heads")
        || rel.starts_with("refs/remotes")
}

fn is_dot_git_path(rel: &Path) -> bool {
    rel.components()
        .next()
        .is_some_and(|c| c.as_os_str() == ".git")
}

/// `child` relative to `root`, or `None` when outside. Both sides must be
/// pre-canonicalized (registration guarantees this; notify reports realpaths
/// under the watched roots).
fn rel_inside(root: &Path, child: &Path) -> Option<PathBuf> {
    child.strip_prefix(root).ok().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<WorkspaceLayout> {
        vec![
            WorkspaceLayout {
                workspace_id: "ws-main".into(),
                project_id: "proj".into(),
                worktree: "/repo".into(),
                git: Some(("/repo/.git".into(), "/repo/.git".into())),
            },
            WorkspaceLayout {
                workspace_id: "ws-linked".into(),
                project_id: "proj".into(),
                worktree: "/wt".into(),
                git: Some(("/repo/.git/worktrees/wt".into(), "/repo/.git".into())),
            },
        ]
    }

    fn classify_one(path: &str) -> BTreeMap<String, WorkspaceEffects> {
        classify(&[PathBuf::from(path)], &fixture())
    }

    #[test]
    fn worktree_file_is_a_file_effect_for_its_workspace_only() {
        let out = classify_one("/repo/src/main.rs");
        assert_eq!(out.len(), 1);
        let eff = &out["ws-main"];
        assert!(!eff.git);
        assert_eq!(
            eff.files.iter().collect::<Vec<_>>(),
            vec![Path::new("src/main.rs")]
        );

        let out = classify_one("/wt/lib.rs");
        assert_eq!(out.len(), 1);
        assert!(out["ws-linked"].files.contains(Path::new("lib.rs")));
    }

    #[test]
    fn index_change_routes_to_owning_worktree_only() {
        let out = classify_one("/repo/.git/index");
        assert_eq!(out.len(), 1);
        assert!(out["ws-main"].git);

        let out = classify_one("/repo/.git/worktrees/wt/index");
        assert_eq!(out.len(), 1);
        assert!(out["ws-linked"].git);
    }

    #[test]
    fn linked_head_move_routes_to_linked_only() {
        let out = classify_one("/repo/.git/worktrees/wt/HEAD");
        assert_eq!(out.len(), 1);
        assert!(out["ws-linked"].git);
    }

    #[test]
    fn shared_branch_ref_fans_out_to_all_sharing_workspaces() {
        for p in [
            "/repo/.git/refs/heads/main",
            "/repo/.git/HEAD",
            "/repo/.git/packed-refs",
            "/repo/.git/refs/remotes/origin/main",
            "/repo/.git/config",
        ] {
            let out = classify_one(p);
            assert!(out["ws-main"].git, "{p} should hit ws-main");
            assert!(out["ws-linked"].git, "{p} should hit ws-linked");
        }
    }

    #[test]
    fn git_noise_paths_have_no_effect() {
        for p in [
            "/repo/.git/objects/ab/cdef0123",
            "/repo/.git/logs/HEAD",
            "/repo/.git/worktrees/wt/logs/HEAD",
            "/repo/.git/FETCH_HEAD",
            "/elsewhere/file.txt",
        ] {
            assert!(classify_one(p).is_empty(), "{p} should classify to nothing");
        }
    }

    #[test]
    fn git_metadata_is_never_a_working_file() {
        // Main worktree: .git lives inside the worktree — metadata paths must
        // not leak into `files`.
        let out = classify_one("/repo/.git/index");
        assert!(out["ws-main"].files.is_empty());
    }

    #[test]
    fn dedup_and_multi_path_batches() {
        let out = classify(
            &[
                PathBuf::from("/repo/a.txt"),
                PathBuf::from("/repo/a.txt"),
                PathBuf::from("/repo/b.txt"),
                PathBuf::from("/repo/.git/index"),
            ],
            &fixture(),
        );
        let eff = &out["ws-main"];
        assert_eq!(eff.files.len(), 2);
        assert!(eff.git);
    }

    #[test]
    fn non_repo_layout_reports_files_only() {
        let layouts = vec![WorkspaceLayout {
            workspace_id: "ws".into(),
            project_id: "p".into(),
            worktree: "/plain".into(),
            git: None,
        }];
        let out = classify(&[PathBuf::from("/plain/notes.md")], &layouts);
        assert!(out["ws"].files.contains(Path::new("notes.md")));
        assert!(!out["ws"].git);
    }

    #[test]
    fn worktree_root_event_is_ignored() {
        assert!(classify_one("/repo").is_empty());
    }
}
