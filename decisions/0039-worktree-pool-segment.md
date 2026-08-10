# ADR-0039: Per-project worktree pool segments — unique pools, adopt-in-place migration

**Status:** accepted (2026-08-09, issue #81). Supersedes the name-only pool
segment documented as a limitation in ADR-0015's project teardown notes.

## Context

The worktree pool for a local project was
`join(default_worktree_directory, safePathSegment(name, id))` — the segment
is the project's **basename**. Two projects whose directories share a
basename (`~/work/ade` and `~/archive/ade`) therefore shared one pool
directory, and `DbProjectStore::delete` runs `remove_dir_all` on the pool:
deleting one project destroyed the other's on-disk worktrees and any
uncommitted work in them. Data loss (FIRST-58).

The same pass also fixes a dead setting: `ProjectSettings.worktree_directory`
(per-project) was read and validated but consumed by nothing — pools always
landed in the app-level `default_worktree_directory`.

## Decision

1. **Segment scheme.** Each project row stores its pool segment
   (`projects.worktree_pool_segment`, migration 0010, nullable). New segments
   are `<safe_path_segment(name,id)>-<hash8>` where hash8 is the first 8 hex
   chars of sha256 over the project's **stored** path. Human-navigable and
   `cd`-usable, unique per project. We hash the stored path rather than
   canonicalizing at resolve time: paths are canonicalized once at create
   (`create_local` realpath), and SSH/remote project paths (Phase 3) may not
   be locally canonicalizable at all.

2. **Resolver.** `worktree_pool_path(db, settings, project)`: root is the
   per-project `worktree_directory` override when set (normalization already
   drops invalid values on read, so an invalid override falls back to the
   app-level `default_worktree_directory`), else the app default. Segment is
   the stored value when present; otherwise the deterministic new-scheme
   segment is computed and **persisted** (lazy stamping, `UPDATE ... WHERE
   worktree_pool_segment IS NULL` so it never overwrites an adopted value).
   Every project that resolves a pool ends up with a unique segment.

3. **Adoption pass** (one-shot, Rust, kv-gated
   `worktree_pool_adoption_v1`, wired into `DbProjectStore::new` so it runs
   before anything resolves a pool):
   - Legacy segment claimed by **one** project → *adopt in place*: stamp the
     legacy value. No filesystem changes — zero risk, recorded paths and `cd`
     paths unchanged.
   - **Collision** → one keeper keeps the legacy dir; the others get
     new-scheme segments and their worktree subdirectories are moved out of
     the shared dir. Keeper tiebreak is deterministic: the sole project with
     worktrees on disk under the shared pool, else earliest `created_at`,
     else smallest id.
   - Moving: subdirectories are attributed to a project via the DB
     (`workspaces.path` of `kind='worktree'` rows for its tasks under the
     shared pool), each is `fs::rename`d into the new pool, then
     `git worktree repair <moved-path>` (new `GitOps::worktree_repair`,
     CLI shell-out — git2 has no repair) re-links `.git/worktrees/*/gitdir`,
     and the stored `workspaces.path` is rewritten. `issues.dossier_path`
     needs no rewrite — it is repo-relative, not absolute.
   - Failures never block startup: per-project errors warn-log and skip.
     The gate is set ONLY when the whole pass succeeded — re-runs are safe
     (stamping is idempotent, moved rows take the repoint branch), so a
     partial pass retries on the next startup (see Hardening below).

4. **Delete.** `DbProjectStore::delete` removes
   `<root>/<stored-or-computed segment>` — unique per project, so deleting
   one project can never touch another's worktrees. The `pool != project.path`
   guard stays.

## Consequences

- Adopt-in-place means the common case (no basename collision) upgrades with
  zero filesystem churn — only a column stamp.
- Collision losers move once, at startup, before any pool resolution; the
  rename + repair is bounded (git CLI timeout class) and recorded paths keep
  resolving because the DB rows are rewritten in the same pass.
- The per-project `worktree_directory` override finally takes effect: pools
  created after setting it land there. (Adoption itself only considers the
  app-level default, because the legacy code ignored the override — all
  existing pools live under the app default.)
- A project whose adoption move fails retries the move on the next startup
  (gate-on-success); its worktrees are never stranded in a half-moved state
  because moves are idempotent (already-moved rows just repoint).
- Regression coverage: FIRST-58 (delete isolation), collision adoption
  (move + repair + rewrite + `ensure_worktree` reuse, with NESTED
  `<branch_prefix>/<branch>` names as in production), sole-adopter
  stability, interrupted-adoption distinct segments, override relocation,
  delete foreign-content guard, dirty-stale-path refusal, resolver
  stamping.

## Hardening (adversarial review of #81 round 1)

Confirmed review findings fixed on the same branch, keeping the decisions
above with these refinements:

1. **Gate-on-success + retry (F2a, F9).** `worktree_pool_adoption_v1` is set
   only when every stamp/move succeeded; a failed per-project move no longer
   strands worktrees forever — the pass re-runs next startup. Top-level
   errors (project list, kv) still return before the gate. The app default
   root is resolved lazily per project; a `localProject` read failure skips
   that project instead of aborting the pass (an override-less project with
   no worktree rows is still stamped, since it needs no filesystem
   knowledge).
2. **Delete foreign-content guard (F2b).** Teardown queries for `kind=
   'worktree'` workspace rows of OTHER projects and skips `remove_dir_all`
   (with a warn) when any canonicalized path lies inside the pool dir
   (component-safe `Path::starts_with`). Covers a half-finished adoption
   that left two projects sharing one pool dir.
3. **Override-aware adoption (F3).** The resolver honors the per-project
   `worktree_directory` override, but pre-#81 the override was dead, so all
   legacy pools live under the app default. Adoption resolves each project's
   target root the SAME way as the resolver; when it differs from the app
   default, the legacy dir moves `default_root/<legacy> →
   override_root/<legacy>` (sole claimants keep the legacy segment — it is
   unique across projects; collision keepers likewise, after movers leave).
   The move machinery is generalized to `from_pool → to_pool` across roots.
4. **Repair as a retryable sweep (F5).** After moving, `git worktree
   repair` runs for EVERY worktree row of the project now under the new
   pool (idempotent — heals rows moved by a previous failed run too). The
   segment is stamped only when every repair succeeded; otherwise it stays
   NULL and the pass retries. This prevents `.git/worktrees/*/gitdir` from
   pointing at the old path, which the next prune would turn into a stale
   path.
5. **Dirty-guard on stale-path removal (F5).** `remove_stale_path` removes
   only when `is_worktree_clean` PROVES the dir clean; a dirty result or a
   clean-check error (broken linkage — the usual stale-path state) refuses
   with `Error::Internal`, which propagates to the task-launch caller
   (fail-loud beats silently destroying uncommitted work).
   `CliGit::is_worktree_clean` now treats a non-zero `git status` exit as
   Err (cleanliness unknown ≠ clean).
6. **Interrupted-run segment check (F6).** A sole-member group checks
   `SELECT ... WHERE worktree_pool_segment = ? AND id != ?` before adopting
   in place; if another project holds the legacy segment (crash between
   keeper stamp and mover completion), it becomes a mover to its new-scheme
   segment instead of duplicating the legacy one.
7. **Nested-path moves (F1).** Production worktrees sit at
   `pool/<branch_prefix>/<branch>-<suffix>`, so the move creates
   `new_path.parent()` before `fs::rename`.
8. **Canonicalized matching (F4, F8).** Adoption compares canonicalized
   paths (fallback raw) on both sides of `starts_with`/`strip_prefix`
   (stored rows can be realpathed, e.g. `/private/var` on macOS). Delete's
   `pool != project.path` root guard canonicalizes both sides too
   (symlink-aliased overrides).
