//! fs_watch — file+git event watcher → live refresh pipeline (E4-01, #41).
//!
//! One `notify` watcher observes every registered workspace: the worktree
//! root (working files, recursive) plus the repo's shared git dir when it
//! lives outside the worktree (linked worktrees share the main repo's
//! `.git`). Raw backend events funnel into a dedicated dispatcher thread
//! that debounces them into ~100 ms batches, classifies them per workspace
//! (`classifier`), and publishes `InternalEvent`s on the bus:
//!
//! - `FilesChanged { workspace_id, paths }` — working-file edits (worktree-
//!   relative, deduped, capped at [`MAX_PATHS_PER_EVENT`]).
//! - `GitChanged { project_id, workspace_id }` — git metadata transitions
//!   (HEAD move, index change, ref/packed-refs/config update).
//!
//! Consumers (Changes sidebar, diff view, editor) refresh off these events —
//! no polling. Editor saves, agent writes, and external `git` commands in a
//! terminal all funnel through the same pipeline.
//!
//! Threading: the dispatcher is a plain `std::thread` fed by an mpsc channel
//! (ARCHITECTURE §4's "long-lived event stream" — a dedicated thread keeps
//! the domain module runtime-free and unit-testable; the bus `send` is sync).
//! The thread exits when the service (and with it the notify watcher +
//! channel sender) drops. A backend error/rescan marks every registered
//! workspace git-dirty so consumers resync rather than trust stale state.

pub mod classifier;
pub mod layout;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use rusqlite::OptionalExtension;

use crate::db::Db;
use crate::events::{EventBus, InternalEvent};
use crate::Error;
use classifier::WorkspaceLayout;
use layout::resolve_git_layout;

/// Quiet window: events arriving within this span of the first event in a
/// batch are coalesced (reference `WATCH_DEBOUNCE_MS`).
const DEBOUNCE: Duration = Duration::from_millis(100);
/// Upper bound on `FilesChanged::paths`; larger batches are truncated (the
/// list is a hint — consumers needing exact state refetch).
const MAX_PATHS_PER_EVENT: usize = 128;

/// Registry payload shared between the service handle and the dispatcher.
#[derive(Default)]
struct Registry {
    /// task id → workspace id (several tasks may share a workspace).
    tasks: HashMap<String, String>,
    workspaces: HashMap<String, WorkspaceEntry>,
    /// canonical watch root → refcount.
    roots: HashMap<PathBuf, usize>,
}

struct WorkspaceEntry {
    layout: WorkspaceLayout,
    /// Watch roots this workspace holds a reference on.
    roots: Vec<PathBuf>,
    task_count: usize,
}

impl Registry {
    fn snapshot(&self) -> Vec<WorkspaceLayout> {
        self.workspaces.values().map(|w| w.layout.clone()).collect()
    }
}

/// Message from the notify callback to the dispatcher thread.
enum Raw {
    Paths(Vec<PathBuf>),
    /// Backend error or rescan — events may have been lost.
    Resync,
}

/// Watches registered workspaces and publishes `FilesChanged` / `GitChanged`.
pub struct FsWatchService {
    registry: Arc<Mutex<Registry>>,
    watcher: Mutex<RecommendedWatcher>,
}

impl FsWatchService {
    /// Creates the watcher and spawns the dispatcher thread (runs until the
    /// service drops).
    pub fn new(bus: Arc<dyn EventBus>) -> Result<Self, Error> {
        let (tx, rx) = std::sync::mpsc::channel::<Raw>();
        let watcher = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| forward(&tx, res),
            notify::Config::default(),
        )
        .map_err(|e| Error::Watch(e.to_string()))?;

        let registry = Arc::new(Mutex::new(Registry::default()));
        spawn_dispatcher(rx, registry.clone(), bus);

        Ok(Self {
            registry,
            watcher: Mutex::new(watcher),
        })
    }

    /// Registers a task's workspace for watching. Idempotent per task; a
    /// workspace shared by several tasks is watched once and released when
    /// the last task unregisters. A worktree that is not a git repo is
    /// watched for file changes only.
    pub fn register_task(
        &self,
        task_id: &str,
        project_id: &str,
        workspace_id: &str,
        worktree: &Path,
    ) -> Result<(), Error> {
        let mut reg = self.registry.lock();

        match reg.tasks.get(task_id) {
            Some(ws) if ws == workspace_id => return Ok(()),
            Some(_) => {
                // Task re-provisioned onto a different workspace: drop the
                // old reference first.
                drop(reg);
                self.unregister_task(task_id);
                reg = self.registry.lock();
            }
            None => {}
        }

        if let Some(entry) = reg.workspaces.get_mut(workspace_id) {
            entry.task_count += 1;
            reg.tasks.insert(task_id.into(), workspace_id.into());
            return Ok(());
        }

        // First task on this workspace: resolve layout + acquire watches.
        let worktree = worktree.canonicalize()?;
        let git = match resolve_git_layout(&worktree) {
            Ok(l) => Some((l.git_dir, l.common_dir)),
            Err(e) => {
                tracing::warn!(
                    workspace_id,
                    worktree = %worktree.display(),
                    error = %e,
                    "workspace is not a watchable git repo; watching files only"
                );
                None
            }
        };

        let mut roots = vec![worktree.clone()];
        if let Some((_, common_dir)) = &git {
            if !common_dir.starts_with(&worktree) {
                roots.push(common_dir.clone());
            }
        }

        let mut watcher = self.watcher.lock();
        let mut acquired: Vec<PathBuf> = Vec::new();
        for root in &roots {
            let count = reg.roots.entry(root.clone()).or_insert(0);
            if *count == 0 {
                if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
                    // Roll back this call's acquisitions.
                    reg.roots.remove(root);
                    for prev in &acquired {
                        let c = reg.roots.get_mut(prev).expect("just acquired");
                        *c -= 1;
                        if *c == 0 {
                            reg.roots.remove(prev);
                            let _ = watcher.unwatch(prev);
                        }
                    }
                    return Err(Error::Watch(format!("{}: {e}", root.display())));
                }
            }
            *count += 1;
            acquired.push(root.clone());
        }

        reg.workspaces.insert(
            workspace_id.into(),
            WorkspaceEntry {
                layout: WorkspaceLayout {
                    workspace_id: workspace_id.into(),
                    project_id: project_id.into(),
                    worktree,
                    git,
                },
                roots,
                task_count: 1,
            },
        );
        reg.tasks.insert(task_id.into(), workspace_id.into());
        Ok(())
    }

    /// Releases a task's watch reference. The workspace's watches stop when
    /// its last task unregisters. Unknown tasks are a no-op.
    pub fn unregister_task(&self, task_id: &str) {
        let mut reg = self.registry.lock();
        let Some(workspace_id) = reg.tasks.remove(task_id) else {
            return;
        };
        let Some(entry) = reg.workspaces.get_mut(&workspace_id) else {
            return;
        };
        entry.task_count -= 1;
        if entry.task_count > 0 {
            return;
        }
        let entry = reg.workspaces.remove(&workspace_id).expect("checked above");
        let mut watcher = self.watcher.lock();
        for root in entry.roots {
            if let Some(count) = reg.roots.get_mut(&root) {
                *count -= 1;
                if *count == 0 {
                    reg.roots.remove(&root);
                    // The root may already be gone (worktree pruned) — the
                    // backend error is expected then.
                    if let Err(e) = watcher.unwatch(&root) {
                        tracing::debug!(root = %root.display(), error = %e, "unwatch failed");
                    }
                }
            }
        }
    }
}

/// notify callback → channel. Filters read-only (`Access`) events and
/// platform junk before they reach the dispatcher.
fn forward(tx: &Sender<Raw>, res: notify::Result<notify::Event>) {
    match res {
        Ok(event) => {
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            if event.need_rescan() {
                let _ = tx.send(Raw::Resync);
                return;
            }
            let paths: Vec<PathBuf> = event
                .paths
                .into_iter()
                .filter(|p| p.file_name().is_none_or(|n| n != ".DS_Store"))
                .collect();
            if !paths.is_empty() {
                let _ = tx.send(Raw::Paths(paths));
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "watch backend error; forcing resync");
            let _ = tx.send(Raw::Resync);
        }
    }
}

fn spawn_dispatcher(rx: Receiver<Raw>, registry: Arc<Mutex<Registry>>, bus: Arc<dyn EventBus>) {
    std::thread::Builder::new()
        .name("fs-watch-dispatcher".into())
        .spawn(move || loop {
            // Block for the first event of a batch; channel closed = service
            // dropped.
            let first = match rx.recv() {
                Ok(raw) => raw,
                Err(_) => return,
            };
            let mut paths = Vec::new();
            let mut resync = false;
            collect(first, &mut paths, &mut resync);

            let deadline = Instant::now() + DEBOUNCE;
            loop {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                match rx.recv_timeout(deadline - now) {
                    Ok(raw) => collect(raw, &mut paths, &mut resync),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }

            let layouts = registry.lock().snapshot();
            dispatch(&paths, resync, &layouts, bus.as_ref());
        })
        .expect("spawning fs-watch dispatcher thread");
}

fn collect(raw: Raw, paths: &mut Vec<PathBuf>, resync: &mut bool) {
    match raw {
        Raw::Paths(p) => paths.extend(p),
        Raw::Resync => *resync = true,
    }
}

/// Classifies one debounced batch and publishes bus events.
fn dispatch(paths: &[PathBuf], resync: bool, layouts: &[WorkspaceLayout], bus: &dyn EventBus) {
    if resync {
        // Events may have been lost — every workspace's git state is suspect.
        for layout in layouts {
            bus.send(InternalEvent::GitChanged {
                project_id: layout.project_id.clone(),
                workspace_id: layout.workspace_id.clone(),
            });
        }
        return;
    }

    let effects = classifier::classify(paths, layouts);
    if effects.is_empty() {
        return;
    }
    let project_of: HashMap<&str, &str> = layouts
        .iter()
        .map(|l| (l.workspace_id.as_str(), l.project_id.as_str()))
        .collect();

    for (workspace_id, eff) in effects {
        if !eff.files.is_empty() {
            let paths: Vec<String> = eff
                .files
                .iter()
                .take(MAX_PATHS_PER_EVENT)
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            bus.send(InternalEvent::FilesChanged {
                workspace_id: workspace_id.clone(),
                paths,
            });
        }
        if eff.git {
            let project_id = project_of
                .get(workspace_id.as_str())
                .copied()
                .unwrap_or_default()
                .to_string();
            bus.send(InternalEvent::GitChanged {
                project_id,
                workspace_id,
            });
        }
    }
}

/// A task/workspace pair the app layer should register at boot or on
/// provision.
#[derive(Debug, Clone)]
pub struct WatchTarget {
    pub task_id: String,
    pub project_id: String,
    pub workspace_id: String,
    pub worktree: PathBuf,
}

/// Boot-time targets: every non-archived task whose workspace has a local
/// path. Stale rows (paths gone from disk) fail registration individually
/// and are skipped by the caller.
pub fn boot_targets(db: &dyn Db) -> Result<Vec<WatchTarget>, Error> {
    let conn = db.conn().lock().unwrap_or_else(PoisonError::into_inner);
    let mut stmt = conn.prepare(
        "SELECT t.id, t.project_id, t.workspace_id, w.path
         FROM tasks t JOIN workspaces w ON w.id = t.workspace_id
         WHERE t.archived_at IS NULL AND w.path IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(WatchTarget {
            task_id: row.get(0)?,
            project_id: row.get(1)?,
            workspace_id: row.get(2)?,
            worktree: PathBuf::from(row.get::<_, String>(3)?),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Target for a just-provisioned task (`TaskProvisioned` handler). `None`
/// when the workspace has no local path (e.g. remote/BYOI).
pub fn target_for(
    db: &dyn Db,
    task_id: &str,
    workspace_id: &str,
) -> Result<Option<WatchTarget>, Error> {
    let conn = db.conn().lock().unwrap_or_else(PoisonError::into_inner);
    conn.query_row(
        "SELECT t.project_id, w.path
         FROM tasks t JOIN workspaces w ON w.id = ?2
         WHERE t.id = ?1",
        [task_id, workspace_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
    )
    .optional()?
    .map_or(Ok(None), |(project_id, path)| {
        Ok(path.map(|p| WatchTarget {
            task_id: task_id.into(),
            project_id,
            workspace_id: workspace_id.into(),
            worktree: PathBuf::from(p),
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::BroadcastEventBus;
    use std::process::Command;
    use tokio::sync::broadcast::error::TryRecvError;

    fn git(dir: &Path, args: &[&str]) {
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

    fn repo_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init"]);
        std::fs::write(tmp.path().join("tracked.txt"), "v1\n").unwrap();
        git(tmp.path(), &["add", "."]);
        git(tmp.path(), &["commit", "-m", "init"]);
        tmp
    }

    fn service() -> (Arc<FsWatchService>, Arc<BroadcastEventBus>) {
        let bus = Arc::new(BroadcastEventBus::new(256));
        let svc = Arc::new(FsWatchService::new(bus.clone() as Arc<dyn EventBus>).unwrap());
        (svc, bus)
    }

    /// Collects bus events until `pred` matches or the deadline passes.
    fn wait_for(
        rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>,
        timeout: Duration,
        pred: impl Fn(&InternalEvent) -> bool,
    ) -> Option<InternalEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            match rx.try_recv() {
                Ok(ev) if pred(&ev) => return Some(ev),
                Ok(_) => continue,
                Err(TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return None,
            }
        }
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) {
        while rx.try_recv().is_ok() {}
    }

    /// Waits out any in-flight debounce window and drains its output.
    fn settle(rx: &mut tokio::sync::broadcast::Receiver<InternalEvent>) {
        std::thread::sleep(Duration::from_millis(300));
        drain(rx);
    }

    #[test]
    fn file_change_emits_files_changed() {
        let repo = repo_fixture();
        let (svc, bus) = service();
        let mut rx = bus.subscribe();
        svc.register_task("t1", "p1", "ws1", repo.path()).unwrap();
        settle(&mut rx);

        std::fs::write(repo.path().join("tracked.txt"), "v2\n").unwrap();

        let ev = wait_for(&mut rx, Duration::from_secs(5), |e| {
            matches!(e, InternalEvent::FilesChanged { workspace_id, .. } if workspace_id == "ws1")
        })
        .expect("FilesChanged for ws1");
        let InternalEvent::FilesChanged { paths, .. } = ev else {
            unreachable!()
        };
        assert!(
            paths.iter().any(|p| p == "tracked.txt"),
            "expected tracked.txt in {paths:?}"
        );
    }

    #[test]
    fn git_index_and_head_changes_emit_git_changed() {
        let repo = repo_fixture();
        let (svc, bus) = service();
        let mut rx = bus.subscribe();
        svc.register_task("t1", "p1", "ws1", repo.path()).unwrap();
        settle(&mut rx);

        std::fs::write(repo.path().join("tracked.txt"), "v2\n").unwrap();
        git(repo.path(), &["add", "."]);
        let ev = wait_for(
            &mut rx,
            Duration::from_secs(5),
            |e| matches!(e, InternalEvent::GitChanged { workspace_id, project_id } if workspace_id == "ws1" && project_id == "p1"),
        );
        assert!(ev.is_some(), "git add should emit GitChanged");
        settle(&mut rx);

        git(repo.path(), &["commit", "-m", "v2"]);
        let ev = wait_for(
            &mut rx,
            Duration::from_secs(5),
            |e| matches!(e, InternalEvent::GitChanged { workspace_id, .. } if workspace_id == "ws1"),
        );
        assert!(ev.is_some(), "commit should emit GitChanged");
    }

    #[test]
    fn burst_collapses_into_one_debounced_batch() {
        let repo = repo_fixture();
        let (svc, bus) = service();
        let mut rx = bus.subscribe();
        svc.register_task("t1", "p1", "ws1", repo.path()).unwrap();
        settle(&mut rx);

        for i in 0..10 {
            std::fs::write(repo.path().join("tracked.txt"), format!("v{i}\n")).unwrap();
        }

        assert!(
            wait_for(&mut rx, Duration::from_secs(5), |e| {
                matches!(e, InternalEvent::FilesChanged { .. })
            })
            .is_some(),
            "burst should produce a FilesChanged batch"
        );
        // The quiet window after the batch must stay quiet: same-path burst
        // dedupes into the one batch (allowing one straggler batch if the
        // backend split delivery across the debounce boundary).
        let mut extra = 0;
        while wait_for(&mut rx, Duration::from_millis(400), |e| {
            matches!(e, InternalEvent::FilesChanged { .. })
        })
        .is_some()
        {
            extra += 1;
        }
        assert!(extra <= 1, "10 writes produced {} batches", 2 + extra);
    }

    #[test]
    fn unregister_stops_events_and_refcounts_shared_workspaces() {
        let repo = repo_fixture();
        let (svc, bus) = service();
        let mut rx = bus.subscribe();
        // Two tasks share one workspace (repository-instance reuse).
        svc.register_task("t1", "p1", "ws1", repo.path()).unwrap();
        svc.register_task("t2", "p1", "ws1", repo.path()).unwrap();
        settle(&mut rx);

        svc.unregister_task("t1");
        std::fs::write(repo.path().join("tracked.txt"), "still watched\n").unwrap();
        assert!(
            wait_for(&mut rx, Duration::from_secs(5), |e| {
                matches!(e, InternalEvent::FilesChanged { .. })
            })
            .is_some(),
            "workspace still referenced by t2 must keep watching"
        );
        settle(&mut rx);

        svc.unregister_task("t2");
        std::fs::write(repo.path().join("tracked.txt"), "unwatched\n").unwrap();
        assert!(
            wait_for(&mut rx, Duration::from_millis(600), |e| {
                matches!(e, InternalEvent::FilesChanged { .. })
            })
            .is_none(),
            "last unregister must stop events"
        );
    }

    #[test]
    fn linked_worktree_commit_fans_out_but_index_stays_scoped() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir(&main).unwrap();
        git(&main, &["init"]);
        std::fs::write(main.join("f.txt"), "x\n").unwrap();
        git(&main, &["add", "."]);
        git(&main, &["commit", "-m", "init"]);
        let linked = tmp.path().join("linked");
        git(
            &main,
            &["worktree", "add", "-b", "wt", linked.to_str().unwrap()],
        );

        let (svc, bus) = service();
        let mut rx = bus.subscribe();
        svc.register_task("t-main", "p1", "ws-main", &main).unwrap();
        svc.register_task("t-linked", "p1", "ws-linked", &linked)
            .unwrap();
        settle(&mut rx);

        // Commit in main moves refs/heads/main → conservative fan-out to both.
        std::fs::write(main.join("f.txt"), "y\n").unwrap();
        git(&main, &["add", "."]);
        git(&main, &["commit", "-m", "fanout"]);
        let mut saw = std::collections::BTreeSet::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while saw.len() < 2 && Instant::now() < deadline {
            if let Some(InternalEvent::GitChanged { workspace_id, .. }) =
                wait_for(&mut rx, Duration::from_millis(200), |e| {
                    matches!(e, InternalEvent::GitChanged { .. })
                })
            {
                saw.insert(workspace_id);
            }
        }
        assert!(saw.contains("ws-main"), "commit fans out to main: {saw:?}");
        assert!(
            saw.contains("ws-linked"),
            "commit fans out to linked: {saw:?}"
        );
        settle(&mut rx);

        // `git add` in the linked worktree touches only its own index.
        std::fs::write(linked.join("f.txt"), "z\n").unwrap();
        git(&linked, &["add", "."]);
        let mut saw = std::collections::BTreeSet::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(InternalEvent::GitChanged { workspace_id, .. }) =
                wait_for(&mut rx, Duration::from_millis(200), |e| {
                    matches!(e, InternalEvent::GitChanged { .. })
                })
            {
                saw.insert(workspace_id);
            }
        }
        assert!(
            saw.contains("ws-linked"),
            "linked index change must hit linked: {saw:?}"
        );
        assert!(
            !saw.contains("ws-main"),
            "linked index change must not hit main: {saw:?}"
        );
    }

    #[test]
    fn register_is_idempotent_per_task() {
        let repo = repo_fixture();
        let (svc, _bus) = service();
        svc.register_task("t1", "p1", "ws1", repo.path()).unwrap();
        svc.register_task("t1", "p1", "ws1", repo.path()).unwrap();
        {
            let reg = svc.registry.lock();
            assert_eq!(reg.workspaces["ws1"].task_count, 1);
        }
        svc.unregister_task("t1");
        let reg = svc.registry.lock();
        assert!(reg.workspaces.is_empty());
        assert!(reg.roots.is_empty());
    }

    #[test]
    fn boot_targets_and_target_for_select_live_local_workspaces() {
        let db = crate::db::SqliteDb::init(Some(":memory:")).unwrap();
        {
            let conn = db.conn().lock().unwrap();
            conn.execute(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'P', '/proj')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, kind, path) VALUES ('w1', 'worktree', '/wt1')",
                [],
            )
            .unwrap();
            // No local path (remote/BYOI) — never a watch target.
            conn.execute(
                "INSERT INTO workspaces (id, kind) VALUES ('w2', 'byoi')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, name, status, workspace_id)
                 VALUES ('t1', 'p1', 'live', 'running', 'w1')",
                [],
            )
            .unwrap();
            // Archived — excluded from boot backfill.
            conn.execute(
                "INSERT INTO tasks (id, project_id, name, status, workspace_id, archived_at)
                 VALUES ('t2', 'p1', 'archived', 'done', 'w1', datetime('now'))",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (id, project_id, name, status, workspace_id)
                 VALUES ('t3', 'p1', 'remote', 'running', 'w2')",
                [],
            )
            .unwrap();
        }

        let targets = boot_targets(db.as_ref()).unwrap();
        assert_eq!(
            targets.len(),
            1,
            "only live tasks with local paths: {targets:?}"
        );
        assert_eq!(targets[0].task_id, "t1");
        assert_eq!(targets[0].project_id, "p1");
        assert_eq!(targets[0].workspace_id, "w1");
        assert_eq!(targets[0].worktree, PathBuf::from("/wt1"));

        let t = target_for(db.as_ref(), "t1", "w1")
            .unwrap()
            .expect("t1 has a target");
        assert_eq!(
            (t.project_id.as_str(), t.worktree.as_path()),
            ("p1", Path::new("/wt1"))
        );
        assert!(target_for(db.as_ref(), "t3", "w2").unwrap().is_none());
        assert!(target_for(db.as_ref(), "missing", "w1").unwrap().is_none());
    }
}
