//! PR sync engine + scheduler (E4-09, #49).
//!
//! The engine is the single write path for the `pull_requests` cache:
//! resolve the worktree's PR target, fetch from GitHub, idempotent upsert.
//! Two callers share it — the on-demand hook (PR tab open / head push) and
//! the periodic scheduler — with an in-flight set so one PR never syncs
//! twice concurrently.
//!
//! Restart safety: the scheduler rebuilds its target list from the DB every
//! cycle and the cursors live in `kv`, so a kill/relaunch resumes exactly
//! where it left off (cached rows render offline immediately).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fartcode_core::db::Db;
use fartcode_core::github::{parse_github_slug, token, GitHubClient};
use fartcode_core::pr_sync::PrSyncStore;
use fartcode_core::Error;
use std::sync::Mutex;

use crate::commit::PrLookup;
use crate::pr_target;
use crate::remote;

/// Commit-card PR-open guard backed by the sync cache (E4-09): a local,
/// offline-safe read — the scheduler + on-demand syncs keep it fresh.
/// Unknown repositories (no remote URL, not GitHub) resolve to `None`.
pub struct CachedPrLookup {
    pub store: Arc<PrSyncStore>,
}

impl PrLookup for CachedPrLookup {
    fn pr_url(
        &self,
        repo: &Path,
        remote_name: &str,
        branch: &str,
    ) -> Result<Option<String>, Error> {
        let Some(url) =
            remote::remote_url(repo, remote_name).or_else(|| remote::remote_url(repo, "origin"))
        else {
            return Ok(None);
        };
        let Some((owner, repo_name)) = parse_github_slug(&url) else {
            return Ok(None);
        };
        self.store.open_pr_url(&owner, &repo_name, branch)
    }
}

/// Sync targets currently in flight (per workspace) — the "no duplicate
/// concurrent syncs per PR" guard, shared by the scheduler and the
/// on-demand hook.
static IN_FLIGHT: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn with_inflight<T>(f: impl FnOnce(&mut HashSet<String>) -> T) -> T {
    let mut guard = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashSet::new))
}

/// One sync pass for a workspace's worktree. Quiet no-ops when the repo
/// isn't GitHub-hosted or no token is configured (the PR tab owns those
/// empty states; the scheduler must not spam errors for them).
pub async fn sync_workspace(
    store: &PrSyncStore,
    workspace_id: &str,
    worktree: &Path,
    push_remote: &str,
) -> Result<(), Error> {
    let claimed = with_inflight(|set| set.insert(workspace_id.to_string()));
    if !claimed {
        return Ok(()); // another sync is already running for this PR
    }
    let result = sync_workspace_inner(store, workspace_id, worktree, push_remote).await;
    with_inflight(|set| set.remove(workspace_id));
    result
}

async fn sync_workspace_inner(
    store: &PrSyncStore,
    workspace_id: &str,
    worktree: &Path,
    push_remote: &str,
) -> Result<(), Error> {
    let Some(target) = pr_target::resolve_pr_target(worktree, push_remote)? else {
        return Ok(());
    };
    let Some(gh_token) = token::resolve_token()? else {
        return Ok(());
    };
    let client = GitHubClient::new(gh_token)?;
    match client
        .fetch_pr_by_branch(&target.owner, &target.repo, &target.branch)
        .await
    {
        Ok(Some(dto)) => {
            store.upsert(Some(workspace_id), &target.owner, &target.repo, &dto)?;
            Ok(())
        }
        // No PR for the branch is data, not failure.
        Ok(None) => Ok(()),
        Err(e) => Err(e),
    }
}

/// `(workspace_id, worktree_path)` pairs for every live task workspace —
/// the scheduler's target list, rebuilt from the DB each cycle (restart
/// safety: nothing in-memory survives, everything re-derives).
pub fn list_sync_targets(db: &Arc<dyn Db>) -> Result<Vec<(String, PathBuf)>, Error> {
    let conn = db
        .conn()
        .lock()
        .map_err(|_| Error::Internal("db connection mutex poisoned".into()))?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT t.workspace_id, w.path
           FROM tasks t
           JOIN workspaces w ON w.id = t.workspace_id
          WHERE t.archived_at IS NULL
            AND w.path IS NOT NULL AND w.path != ''",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (ws, path) = row?;
        out.push((ws, PathBuf::from(path)));
    }
    Ok(out)
}

/// Scheduler knobs (ponytail defaults — tune when rate limits say so).
pub struct SchedulerConfig {
    pub base_interval: Duration,
    pub max_backoff: Duration,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            base_interval: Duration::from_secs(60),
            max_backoff: Duration::from_secs(3600),
        }
    }
}

/// Runs the periodic sync loop for the app's lifetime. One sequential pass
/// per tick — inherently free of duplicate syncs; rate-limit errors cut the
/// pass short (the limit is account-global) and per-workspace failures back
/// off exponentially up to `max_backoff` (with a little jitter so multiple
/// workspaces don't stampede the same second).
pub async fn run_scheduler(store: Arc<PrSyncStore>, db: Arc<dyn Db>, config: SchedulerConfig) {
    loop {
        tokio::time::sleep(config.base_interval + jitter()).await;
        let targets = match list_sync_targets(&db) {
            Ok(targets) => targets,
            Err(e) => {
                tracing::warn!(error = %e, "pr sync: failed to list targets");
                continue;
            }
        };
        for (workspace_id, path) in targets {
            // Backoff gate: skip until the per-workspace interval elapses.
            let failures = store.failure_count(&workspace_id).unwrap_or(0);
            let interval = backoff_interval(config.base_interval, config.max_backoff, failures);
            let due = match store.last_sync(&workspace_id) {
                Ok(Some(last)) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    now - last >= interval.as_secs() as i64
                }
                _ => true, // never synced (or cursor unreadable) → try
            };
            if !due {
                continue;
            }
            // ponytail: the scheduler syncs against "origin" — a project's
            // custom pushRemote still works on-demand via the PR tab.
            match sync_workspace(&store, &workspace_id, &path, "origin").await {
                Ok(()) => {
                    let _ = store.record_success(&workspace_id);
                }
                Err(Error::GithubRateLimited { reset_at }) => {
                    let _ = store.record_failure(&workspace_id);
                    tracing::warn!(reset_at, "pr sync: rate limited — ending this cycle");
                    break; // account-global limit; the rest would fail too
                }
                Err(e) => {
                    let failures = store.record_failure(&workspace_id).unwrap_or(1);
                    tracing::debug!(workspace = %workspace_id, failures, error = %e, "pr sync failed");
                }
            }
        }
    }
}

/// `base * 2^failures` capped at `max`.
fn backoff_interval(base: Duration, max: Duration, failures: u32) -> Duration {
    let secs = base.as_secs().saturating_mul(1u64 << failures.min(16));
    Duration::from_secs(secs.min(max.as_secs()))
}

/// 0–15 s of jitter from the wall clock (no rand dependency needed).
fn jitter() -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    Duration::from_secs(u64::from(nanos % 15))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::tests::{git, repo_fixture};
    use fartcode_core::db::SqliteDb;
    use fartcode_core::events::{BroadcastEventBus, EventBus};

    #[test]
    fn backoff_doubles_and_caps() {
        let base = Duration::from_secs(60);
        let max = Duration::from_secs(3600);
        assert_eq!(backoff_interval(base, max, 0), Duration::from_secs(60));
        assert_eq!(backoff_interval(base, max, 1), Duration::from_secs(120));
        assert_eq!(backoff_interval(base, max, 3), Duration::from_secs(480));
        assert_eq!(backoff_interval(base, max, 10), Duration::from_secs(3600));
    }

    #[test]
    fn list_targets_excludes_archived_and_pathless_workspaces() {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        {
            let conn = db.conn().lock().unwrap();
            conn.execute_batch(
                "INSERT INTO projects (id, name, path) VALUES ('p1', 'demo', '/tmp/demo');
                 INSERT INTO tasks (id, project_id, name, status, workspace_id)
                   VALUES ('t1', 'p1', 'a', 'in_progress', 'w1'),
                          ('t2', 'p1', 'b', 'in_progress', 'w2'),
                          ('t3', 'p1', 'c', 'in_progress', 'w3');
                 UPDATE tasks SET archived_at = datetime('now') WHERE id = 't2';
                 INSERT INTO workspaces (id, path) VALUES ('w1', '/tmp/w1'), ('w2', '/tmp/w2'), ('w3', NULL);",
            )
            .unwrap();
        }
        let targets = list_sync_targets(&db).unwrap();
        assert_eq!(targets, vec![("w1".to_string(), PathBuf::from("/tmp/w1"))]);
    }

    #[tokio::test]
    async fn sync_workspace_is_a_quiet_noop_without_a_remote() {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        let store = PrSyncStore::new(db.clone(), bus.clone() as Arc<dyn EventBus>);
        let repo = repo_fixture();
        // No remote at all → resolves to no target → Ok(()), no writes.
        sync_workspace(&store, "w1", repo.path(), "origin")
            .await
            .unwrap();
        assert!(store.list_for_workspace("w1").unwrap().is_empty());
    }

    #[tokio::test]
    async fn sync_workspace_is_a_quiet_noop_for_non_github_remotes() {
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        let store = PrSyncStore::new(db.clone(), bus.clone() as Arc<dyn EventBus>);
        let repo = repo_fixture();
        git(
            repo.path(),
            &["remote", "add", "origin", "git@gitlab.com:o/r.git"],
        );
        sync_workspace(&store, "w1", repo.path(), "origin")
            .await
            .unwrap();
        assert!(store.list_for_workspace("w1").unwrap().is_empty());
    }

    #[test]
    fn inflight_dedupe_is_reentrant_safe() {
        let claimed = with_inflight(|set| set.insert("dedupe-test".into()));
        assert!(claimed);
        // Second claim for the same workspace fails; the first holder
        // releases it afterwards.
        assert!(!with_inflight(|set| set.insert("dedupe-test".into())));
        with_inflight(|set| set.remove("dedupe-test"));
        assert!(with_inflight(|set| set.insert("dedupe-test".into())));
        with_inflight(|set| set.remove("dedupe-test"));
    }

    #[test]
    fn cached_pr_lookup_reads_the_sync_cache() {
        use fartcode_core::github::{PrDto, PrStatus, PrUserDto};
        let db: Arc<dyn Db> = SqliteDb::init(Some(":memory:")).unwrap();
        let bus = Arc::new(BroadcastEventBus::new(16));
        let store = Arc::new(PrSyncStore::new(db, bus.clone() as Arc<dyn EventBus>));
        let lookup = CachedPrLookup {
            store: store.clone(),
        };

        let repo = repo_fixture();
        git(
            repo.path(),
            &["remote", "add", "origin", "git@github.com:o/r.git"],
        );
        // Empty cache → no guard.
        assert!(lookup
            .pr_url(repo.path(), "origin", "feat/x")
            .unwrap()
            .is_none());

        let dto = PrDto {
            number: 42,
            title: "t".into(),
            url: "https://github.com/o/r/pull/42".into(),
            status: PrStatus::Open,
            draft: false,
            author: Some(PrUserDto {
                login: "j".into(),
                avatar_url: None,
                url: None,
            }),
            base_ref: "main".into(),
            head_ref: "feat/x".into(),
            head_oid: "h1".into(),
            additions: 0,
            deletions: 0,
            changed_files: 0,
            commit_count: 0,
            mergeable_state: None,
            review_decision: None,
            created_at: None,
            updated_at: None,
            files: vec![],
            commits: vec![],
            checks: vec![],
            comments: vec![],
        };
        store.upsert(Some("w1"), "o", "r", &dto).unwrap();
        assert_eq!(
            lookup
                .pr_url(repo.path(), "origin", "feat/x")
                .unwrap()
                .as_deref(),
            Some("https://github.com/o/r/pull/42")
        );
        // Non-GitHub remote → None.
        let repo2 = repo_fixture();
        git(
            repo2.path(),
            &["remote", "add", "origin", "git@gitlab.com:o/r.git"],
        );
        assert!(lookup
            .pr_url(repo2.path(), "origin", "feat/x")
            .unwrap()
            .is_none());
    }
}
