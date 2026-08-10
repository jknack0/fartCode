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
   - Failures never block startup: per-project errors warn-log and skip
     (a skipped project lazily gets a fresh unique pool on first resolve —
     its leftovers stay with the keeper); the gate is set once the pass
     completes, so it runs exactly once and is safe to re-run (stamped rows
     and moved directories are simply not re-processed).

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
- A project whose adoption move fails simply gets a fresh pool later; its old
  worktrees remain on disk under the keeper's pool until the user removes
  them — loud-enough via warn log, and never data loss.
- Regression coverage: FIRST-58 (delete isolation), collision adoption
  (move + repair + rewrite + `ensure_worktree` reuse), sole-adopter
  stability, override + fallback, resolver stamping.
