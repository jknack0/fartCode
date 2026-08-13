# Architecture Deep Dive — fartCode

**Date:** 2026-08-12 · **Scope:** full workspace (12 crates, ~64k Rust LOC) + `app-frontend` (~21k TS LOC) · **Base commit:** `cf4763c`
**Method:** 9 parallel area surveys (each reading real code) → dedupe → 4 adversarial verifiers re-reading every cited line and trying to refute → completeness critic. 75 raw findings → 62 deduped → **61 survived verification** (49 confirmed, 12 confirmed-with-caveats, 1 rejected as fighting a documented decision).

**Verdict:** the codebase is disciplined where it decided to be — append-only migrations, the #80 threading gate, no-egress telemetry, the ACP transcript reducer — and its debt is concentrated in a small number of *systemic* shapes rather than scattered mess. One live bug surfaced (§T1). The six themes below cover ~80% of the findings; the full verified catalogue follows in the appendix.

---

## The six themes

### T1 — One contract, three hand-written copies (highest leverage)

The backend↔frontend contract is maintained by hand in three places that must agree and don't:

- `InternalEvent` (~40 variants, `fartcode-core/src/events.rs`) has a serde derive **that never reaches the wire** — `event_to_value` (`fartcode-app/src/app.rs:223-411`) re-keys every variant by hand into a ~190-line `json!` match, and `FartcodeEvent` in `app-frontend/src/lib/tauri.ts` re-types it a third time. Shipped drift: `task:created` carries `id` while `task:renamed`/`deleted`/`status_changed` carry `taskId` — consumers branch on which key to read.
- `tauri.ts` hand-mirrors **92 Rust DTOs** across **119 invoke wrappers** with zero `invoke<T>` typing and no codegen. TS `TaskDto` already silently lacks `automationRunId`/`createdAt`/`updatedAt`; `BranchRef` leaks `is_remote` because the Rust struct never got its `rename_all`.
- **Live bug:** `UpdateIssueRequest` declares five tri-state fields as bare `Option<Option<T>>` — serde collapses explicit `null` to "keep", so clearing a card's body from `CardDetail.tsx` (which sends `body: body || null`) silently keeps the old value. The correct `double_option` deserializer exists 30 lines away in `commands/columns.rs`, with a regression test calling this exact defect "the reviewers' reproduced defect".
- Nothing could have caught any of this: there are **no cross-boundary contract tests** (the 651-line static gate for #80 proves the repo knows how to build such gates).

**Fix, in order:** (1) share `double_option` and annotate `UpdateIssueRequest` — S, closes the live bug; (2) make the enum the single wire source (camelCase serde + one `event_name()` helper), shrinking `event_to_value` to a skip-set; (3) adopt `ts-rs` or `tauri-specta` and generate the TS types; (4) add a golden-file event-contract test as the gate.

### T2 — The promised `workspaces` module doesn't exist

`ARCHITECTURE.md` §2 lists `workspaces/ { mod.rs, model.rs }` as a core domain module. It was never built, and the vacuum shaped the codebase: raw `FROM workspaces` SQL in **8 files / 12 sites in core** (including the PTY launcher reaching into another domain's table), **5 divergent task→worktree path resolutions** in the app layer, and — the sharpest edge — the worktree **path-containment security check copied 4×** (`files.rs` ×3, `line_comments` ×1) and already diverging (`./x` legal in one, rejected in the other; stricter-not-weaker so far, but four copies of a security invariant is four places to harden).

**Fix:** build `WorkspaceStore` with the queries the sites actually need, port the 12 call sites, and collapse the containment check into one `files::resolve_contained(worktree, rel, mode)`. The containment helper alone is an S-effort security win; do it first.

### T3 — DB discipline: one lock, three poison policies, racy read-merge-write

Every DB operation in the app serializes through one `std::sync::Mutex<Connection>`. The lock *ritual* is hand-rolled in ~16 places with **three divergent poison policies**; the dominant write template (get → merge in memory → **new lock** → write ALL columns back → emit → re-get) repeats ~12× and loses concurrent patches in the unlock window; `DbTaskStore::delete` spans four lock acquisitions with no transaction while create correctly uses one.

**Fix:** one `mutate(id, event, closure)` helper per store holding a single guard across fetch-modify-write (collapses ~10 method bodies and closes the lost-update window); transactions on the delete/move paths; and — per the completeness critic — treat the single-connection choice as an explicit architectural decision (WAL + read connections, or a documented "one writer is fine for solo-desktop" ADR) rather than an inherited accident.

### T4 — Boilerplate whose cure already exists, buried in the wrong module

- `off_main_thread` — the correct abstraction for the #80 spawn-blocking discipline — lives inside `commands/git.rs`; four modules import it from there (a panicking telemetry command reports as *"git task did not complete"*), while ~30 other sites hand-roll the dance. Promote it to `commands/mod.rs`; the 651-line gate test shrinks to checking one helper name.
- Frontend equivalents: the async-submit ritual (busy-guard + `String(e)` + close-on-success) is copy-pasted **14+×**; the per-key async-cache store shape **7×**; hand-rolled event subscriptions **10×** with two missing the disposed-race guard the others have; the editable-target keyboard guard **8×** with drift while the canonical one sits unexported.

### T5 — God files by accretion, each with a documented split that never happened

`lib/tauri.ts` (1,919 LOC, 56 commits in 3 months — every feature lands there; ARCHITECTURE §12 already specifies the ipc/events/commands split). `step_engine.rs` (2,640 LOC — half tests; the state machine, orchestration, and board mutation are separable, and the layering lens argues the policy half belongs in core). `TerminalManager::open` (250 lines with a 90-line inline pump thread). `BoardView.tsx` (1,409 LOC with a separable ~250-line drag engine). `Modals.tsx` (1,061 LOC of unrelated dialogs sharing store-bookkeeping idioms).

### T6 — Dead weight and unenforced decisions

`Git2Ops` + the entire **libgit2 native dependency** exist to serve one example binary — every production site constructs `CliGit`, directly contradicting accepted ADR-0007 (pick a side: wire it in for its stricter worktree semantics, or delete 18KB + the C dependency). `fartcode-integrations` (11 LOC) and `fartcode-server` (7 LOC) are placeholders; `fartcode-runtime` (881 LOC) is the dormant worker ADR-0030 promised to retire. Dead IPC wrappers (`issueDispatch`, `shareWithTeam`, `agentAddLineComment`), dead core fns, and an `ARCHITECTURE.md` crate-graph claim ("core depends on nothing") that drifted from reality. (`fartcode-scheduler`'s dormancy is *documented* — the one finding the verifiers rejected was the one that failed to acknowledge that.)

---

## What the review itself missed (completeness critic)

1. **The event bus is lossy with no resync path** — drop-on-lag at capacity 256, all four subscriber loops `Lagged(_) => continue`, and the entire UI liveness model is "event → refetch". A settle-storm burst silently strands the UI stale. Deserves a design decision (resync signal à la `fs_watch`'s dirty-marking), not just deduped loop code.
2. **Zero cross-boundary tests** for the T1 contract (detailed above).
3. **~4.7k LOC of core got zero findings** — `dependencies/` (which hides a *third* PATH-lookup copy), `fs_watch/`, `github/`, `conversations/`, `line_comments/` internals. Finding-density anomaly, not proof of cleanliness.
4. **Stringly-typed errors everywhere** (57 + 126 `map_err` sites; `String(e)` in the frontend): no error taxonomy on the IPC surface, so retry/toast/silent decisions are hand-made per call site. The poison-policy divergence and submit boilerplate are symptoms.
5. **The CSS layer (5,350 LOC) was never audited** despite DESIGN.md being binding — settings.css alone is 1,139 lines; the TSX duplication findings likely have unexamined CSS twins.

---

## Recommended sequence

| # | Action | Theme | Impact | Effort |
|---|--------|-------|--------|--------|
| 1 | Share `double_option`; fix `UpdateIssueRequest` tri-state fields + round-trip test | T1 | **live bug** | S |
| 2 | `files::resolve_contained` — collapse the 4 security-check copies | T2 | high | S |
| 3 | Promote `off_main_thread` to `commands/mod.rs`; convert ~30 sites | T4 | high | M |
| 4 | Decide ADR-0007: wire `Git2Ops` in or delete it + libgit2 | T6 | high | M |
| 5 | Event contract: serde-as-source + `ts-rs` codegen + golden-file gate | T1 | high | M→L |
| 6 | `WorkspaceStore` in core; port the 12 raw-SQL sites | T2 | high | L |
| 7 | Store `mutate()` helper; single-guard updates; transactions on delete | T3 | high | M |
| 8 | Split `tauri.ts` per ARCHITECTURE §12 (barrel re-export keeps imports stable) | T5 | high | M |
| 9 | Frontend factories: `createAsyncSubmit`, per-key cache store, `wireEvents` helper | T4 | medium | M |
| 10 | Event-bus loss semantics: resync signal or documented at-most-once ADR | critic | high | M |

Everything below this line is the full verified catalogue, ranked by impact then effort.

---

# Appendix — all 61 verified findings

### Worktree path-containment check copied 4x and already drifting (security invariant)
`CONFIRMED` · impact **high** · effort **S** · fartcode-core · infrastructure
Files: `fartcode-core/src/files.rs:18` · `fartcode-core/src/files.rs:71` · `fartcode-core/src/files.rs:115` · `fartcode-core/src/line_comments/mod.rs:344`

The lexical (`components().all(|c| matches!(c, Normal…))` + canonicalize + `starts_with(canonical_worktree)`) containment check is inlined in files.rs three times (write_file:19-32, read_file:72-84, list_dir:116-127) and a fourth time in line_comments::validate_comment_anchor (350-370), whose comment says 'mirrors files.rs'. They have already diverged: files.rs accepts `Component::CurDir` (`./x` is legal), line_comments accepts only `Component::Normal` (rejects `./x`); files.rs write path uses resolve_for_write for not-yet-existing files, the others require existence.

**Proposal:** One `files::resolve_contained(worktree: &Path, rel_path: &str, mode: ResolveMode{MustExist, ForWrite}) -> Result<PathBuf, Error>` that owns the lexical check, canonicalization, and starts_with test (reusing the existing resolve_for_write for the ForWrite mode). write_file/read_file/list_dir and validate_comment_anchor all collapse into calls to it. Any future hardening (e.g. NFC normalization, Windows verbatim paths) then lands in exactly one place instead of four security-critical copies.

### issue_update explicit-null 'clear' is a silent no-op; double_option not shared
`CONFIRMED` · impact **high** · effort **S** · fartcode-app · command layer
Files: `fartcode-app/src/commands/issues.rs:45` · `fartcode-app/src/commands/issues.rs:48` · `fartcode-app/src/commands/columns.rs:24` · `app-frontend/src/lib/tauri.ts:1307`

columns.rs:18-30 hand-rolls a `double_option` deserializer with the comment: "Plain `Option<Option<T>>` cannot express this — serde collapses an explicit `null` into `None`, silently turning 'clear' into 'keep'" (and a regression test at columns.rs:232 calling it "The reviewers' reproduced defect"). Yet issues.rs:45-55 `UpdateIssueRequest` declares body/provider/model/prd_path/prd_section as bare `Option<Option<String>>` with a doc comment claiming "null → Some(None)" — false without the deserializer. The frontend contract requires it: tauri.ts:1307-1308 documents "explicit null clears a nullable field" and types patch fields `string | null`. So clearing an issue's body/provider/model/PRD link over the wire silently keeps the old value.

**Proposal:** Move `double_option` from commands/columns.rs into a shared `commands::serde_util` (or commands/mod.rs) and annotate the five tri-state fields of `UpdateIssueRequest` with `#[serde(default, deserialize_with = "double_option")]`, mirroring UpdateColumnRequest. Add the absent/null/value round-trip test issues.rs currently lacks (columns.rs:232-261 is the template).

### Read-merge-write-all update loses concurrent patches; write template repeated ~12x
`CONFIRMED` · impact **high** · effort **M** · fartcode-core · domain
Files: `fartcode-core/src/issues/mod.rs:631` · `fartcode-core/src/issues/mod.rs:668` · `fartcode-core/src/issues/columns.rs:634` · `fartcode-core/src/tasks/mod.rs:406` · `fartcode-app/src/dossiers.rs:166` · `fartcode-app/src/commands/issues.rs:110`

IssueStore::update fetches the issue (own lock), merges the IssuePatch in memory, then under a NEW lock writes ALL eight columns back: "UPDATE issues SET title = ?2, body = ?3, acceptance = ?4, provider = ?5, model = ?6, prd_path = ?7, prd_section = ?8, dossier_path = ?9 ..." (issues/mod.rs:668-684). ColumnStore::update does the same full-row write-back (columns.rs:720-741). The surrounding template — get→ok_or(NotFound), lock+execute, event_bus.send, get→ok_or(Internal("vanished after ...")) — repeats in update/move_to/enter_column/set_linked_task/add_dependency/remove_dependency (7 "vanished after" strings in issues/mod.rs) and in DbTaskStore's update_status/rename/set_pinned/archive/restore.

**Proposal:** Add a private IssueStore::mutate(id, event, f: FnOnce(&Connection, &Issue) -> Result<(), Error>) -> Result<Issue, Error> (and the DbTaskStore twin) that holds ONE MutexGuard across fetch, closure, and write, emits the event after the guard drops, and does the re-fetch; convert update to per-field SET clauses (or keep full-row but inside the single guard). Collapses ~10 near-identical method bodies and closes the lost-update window.

### spawn_blocking dance hand-rolled ~30x; off_main_thread hides inside commands::git
`CONFIRMED` · impact **high** · effort **M** · fartcode-app · command layer
Files: `fartcode-app/src/commands/git.rs:61` · `fartcode-app/src/commands/git.rs:68` · `fartcode-app/src/commands/telemetry.rs:20` · `fartcode-app/src/commands/tasks.rs:54` · `fartcode-app/src/commands/provider_accounts.rs:94` · `fartcode-app/src/commands/github.rs:41`

The #80 pattern `let app = app.inner().clone(); tauri::async_runtime::spawn_blocking(move || ...).await.map_err(|e| e.to_string())?` is hand-written at ~30 call sites in tasks.rs, terminals.rs, issues.rs, steps.rs, projects.rs, provider_accounts.rs, ssh_connections.rs, remote_projects.rs, dependencies.rs, lifecycle.rs, line_comments.rs — while `off_main_thread` (git.rs:61-69) already abstracts it and is imported cross-module from commands::git by telemetry.rs:20, files.rs:9, dossiers.rs:40, github.rs:19. Its join-error string is "git task did not complete: {e}" (git.rs:68), so a panic in telemetry_memory_value or github_token_set reports itself as a failed *git* task. The idiom is fragile enough that a 651-line text-parsing static gate (tests/no_blocking_tauri_commands.rs) exists to police it, and drift still slipped through: pr_section_get (github.rs:36-44) runs section_from_cache inline in the async body — a keyring read (which github.rs:150-152 itself calls "an unbounded wait" behind a locked keychain) plus resolve_pr_target's git subprocess — on the tokio runtime, the exact "async alone only moves the stall" failure the module headers warn about.

**Proposal:** Promote `off_main_thread` to commands/mod.rs with a neutral join-error message ("command did not complete: {e}"), convert the ~30 hand-rolled sites to it, and route pr_section_get's cache read through it. The gate test then shrinks to checking for one helper name instead of parsing spawn_blocking shapes per command.

### Git2Ops and the libgit2 dependency serve only an example; production uses CliGit
`CONFIRMED` · impact **high** · effort **M** · satellite crates
Files: `fartcode-git/src/git2ops.rs:81` · `fartcode-git/src/git2ops.rs:260` · `fartcode-core/examples/smoke.rs:464` · `fartcode-app/src/app.rs:92` · `fartcode-app/src/app.rs:139` · `fartcode-app/src/app.rs:155`

ADR-0007 (accepted) made Git2Ops the worktree strategy: git2-backed worktree_list/add/prune/remove with everything else delegated to CliGit. But every production construction site passes CliGit — app.rs:92 (project store), app.rs:139 and app.rs:155-156 (both WorktreeManager::new calls, the exact worktree lifecycle Git2Ops was built for). The only Git2Ops consumer in the repo is fartcode-core/examples/smoke.rs:464. The cost is double: 18KB of carefully-written git2 code (mutex serialization, synthesized main worktree, force checkout) is dead in production, and the workspace pays a native libgit2 C build (git2 = "0.21") on every clean compile for it. git2ops.rs also carries a 20-method hand-written delegation wall (lines 260-323) that must grow every time the GitOps trait grows.

**Proposal:** Pick a side of ADR-0007. Either wire Git2Ops into the two WorktreeManager constructions in app.rs (the ADR's intent — its worktree semantics are stricter than CliGit's rm-rf-based worktree_remove, which lib.rs:611 itself flags as the less-safe Phase-0 stopgap), or supersede the ADR and delete git2ops.rs plus the git2 workspace dependency, updating smoke.rs to CliGit. Deleting also erases the 20-method forwarding wall.

### lib/tauri.ts god module: 92 types, 119 invoke wrappers, events, and behavior in one file
`CONFIRMED` · impact **high** · effort **M** · frontend · state & IPC
Files: `app-frontend/src/lib/tauri.ts:1` · `app-frontend/src/lib/tauri.ts:546` · `app-frontend/src/lib/tauri.ts:1203` · `ARCHITECTURE.md:1389`

1919 LOC, 127 exported functions, 92 exported types, 56 commits in the last 3 months (every feature lands here — it is modified in the current working tree too). The header claims "Thin typed wrappers over the fartCode Tauri commands + the event channel", but the file also carries real behavior: waitForTerminalReady (lines 546-571) is a stateful race-closing helper with timers and subscription management, and commentAuthor (1203-1208) is a pure domain parser. ARCHITECTURE.md §12 itself specifies a split (lib/ipc.ts, lib/events.ts, lib/commands.ts) that was never realized.

**Proposal:** Split into lib/ipc/<domain>.ts mirroring the backend command modules — projects.ts, tasks.ts, terminals.ts, git.ts, pr.ts, comments.ts, acp.ts, board.ts, columns.ts, steps.ts, dossiers.ts, ssh.ts, settings.ts, telemetry.ts — plus lib/ipc/events.ts for the FartcodeEvent union and listen helpers. Keep lib/tauri.ts as a pure barrel re-export so the 30+ import sites need no churn, then migrate imports opportunistically. Move waitForTerminalReady into lib/terminals.ts (it already owns terminal behavior) and commentAuthor next to the comment store.

### Event contract hand-maintained in three copies: enum, json! map, TS union
`CONFIRMED` · impact **high** · effort **M** · cross-cutting duplication
Files: `fartcode-core/src/events.rs:14` · `fartcode-app/src/app.rs:223` · `app-frontend/src/lib/tauri.ts:31`

InternalEvent (events.rs:14-298, ~40 variants) carries a serde derive (tag="type", content="payload", snake_case at events.rs:11-13) that never reaches the wire — the only path to the frontend is event_to_value (app.rs:223-411), ~25 hand-written json! blocks re-keying each variant to "domain:name" + camelCase, mirrored a third time as the FartcodeEvent TS union (tauri.ts:31-105). Drift is already visible: task:created carries "id" (app.rs:269) while task:renamed/deleted/status_changed carry "taskId" (app.rs:276-280), faithfully re-typed in TS (tauri.ts:46-48), so consumers branch on which key holds the task id. app.rs:441-576 is a 135-line test that re-asserts each mapping field by field — a fourth copy. Every new event costs 3 synchronized edits plus test.

**Proposal:** Make the enum the single source: put #[serde(rename_all = "camelCase")] on variant fields, derive the "domain:event" wire name from the variant name with one event_name() helper, and serialize via serde_json::to_value + injected "type" — event_to_value shrinks to a skip-set of non-UI variants. Generate the TS union from the enum with ts-rs (#[derive(TS)]) into app-frontend/src/lib/generated/events.ts, replacing the hand-typed FartcodeEvent.

### Promised workspaces domain missing; 8 files hand-roll SQL against the workspaces table
`CONFIRMED` · impact **high** · effort **L** · fartcode-core · infrastructure
Files: `fartcode-core/src/pty/launcher.rs:713` · `fartcode-core/src/tasks/mod.rs:485` · `fartcode-core/src/tasks/operations.rs:288` · `fartcode-core/src/tasks/byoi.rs:380` · `fartcode-core/src/projects/provider.rs:80` · `fartcode-core/src/projects/mod.rs:347`

ARCHITECTURE.md §2 lists `workspaces/ { mod.rs, model.rs }` as a domain module; no such directory exists in fartcode-core/src. Instead, raw `FROM workspaces` SQL is scattered across at least 8 files/12 sites: the PTY launcher's boot rehydration queries `SELECT path, COALESCE(location, 'local') FROM workspaces WHERE id = ?1` inline (launcher.rs:707-718 — the agent launcher reaching into another domain's table); line_comments joins tasks→workspaces for task_workspace_path (301-307); tasks/mod.rs:485,510 subselects kind and deletes rows; tasks/operations.rs:288,426,614 reads kind/config; byoi.rs:380 reads config; projects/{mod.rs:303,347, provider.rs:80, adoption.rs:323} select/delete by key/path; fs_watch/mod.rs:365,388 joins for watch targets. Any workspaces schema change (e.g. a location enum, a new column) is shotgun surgery across all of them, and the COALESCE-default-'local' convention is re-encoded per site.

**Proposal:** Create `workspaces/mod.rs` with a `WorkspaceStore` (Arc<dyn Db>) exposing the queries the sites actually need: `get(id) -> {path, kind, location, config}`, `path_for_task(task_id)`, `kind(id)`, `delete(id)`, `insert(row)`, plus the tasks-join helpers (`watch_targets()`, `local_worktrees_not_in_project(project_id)`). Ports the 12 call sites onto it; the 'location defaults to local' rule lives in one row-mapper. This also removes the launcher's direct `Arc<dyn Db>` dependency (launcher.rs:629) in favor of the narrower store.

### tauri.ts hand-mirrors every Rust DTO: 92 interfaces, 119 invoke wrappers, no codegen
`CONFIRMED` · impact **high** · effort **L** · cross-cutting duplication
Files: `app-frontend/src/lib/tauri.ts:5` · `fartcode-core/src/tasks/model.rs:126` · `fartcode-core/src/projects/model.rs:67` · `fartcode-app/src/commands/ssh_connections.rs:23` · `fartcode-app/src/commands/dossiers.rs:45` · `fartcode-app/src/commands/provider_accounts.rs:43`

tauri.ts (1,919 lines) contains 92 export interface/type declarations and 119 invoke() calls, each restating a Rust #[serde(rename_all="camelCase")] struct (81 camelCase attributes across core+app). Concrete drift: Rust TaskDto (tasks/model.rs:126-142) has automation_run_id/created_at/updated_at that the TS TaskDto (tauri.ts:16-29) silently lacks; BranchRef (tauri.ts:134-139) leaks snake_case (is_remote) against the camelCase convention because its Rust struct never got the rename. Neither ts-rs nor specta/tauri-specta appears in any Cargo.toml or package.json.

**Proposal:** Adopt ts-rs (or tauri-specta to also type the 119 command signatures): #[derive(TS)] on every *Dto and event payload, emit to app-frontend/src/lib/generated/, and reduce tauri.ts to thin invoke wrappers importing generated types. Migrate incrementally starting with the shapes already drifting (TaskDto, BranchRef, the step/dispatch payloads).

### Hand-rolled Mutex<Connection> lock/poison boilerplate duplicated in ~16 places
`CONFIRMED` · impact **medium** · effort **S** · fartcode-core · domain
Files: `fartcode-core/src/db/connection.rs:33` · `fartcode-core/src/issues/mod.rs:475` · `fartcode-core/src/issues/columns.rs:528` · `fartcode-core/src/issues/ledger.rs:99` · `fartcode-core/src/tasks/mod.rs:174` · `fartcode-core/src/tasks/mod.rs:244`

Two competing spellings of the same 5-line lock: fn conn() -> Result<MutexGuard<...>> mapping to Internal("db mutex poisoned: {e}") (issues/mod.rs:475-480, columns.rs:528-533, ledger.rs:99-104, line_comments, pr_sync, ssh_connections) and fn with_conn(f) mapping to Internal("db connection mutex poisoned") (tasks/mod.rs:174-185, operations.rs:393-403, projects, conversations, settings/service, adoption). settings/kv.rs inlines the map_err five separate times (41, 69, 84, 95, 108); tasks/byoi.rs and deletion.rs inline it in free functions.

**Proposal:** Provided methods on the Db trait itself (db/connection.rs): fn lock(&self) -> Result<MutexGuard<'_, Connection>, Error> and fn with_conn<T>(&self, f) -> Result<T, Error>, defaulted on the trait since conn() is already required. Delete every per-store copy; new stores get it for free and the poison message stops forking.

### TaskCreationService rebuilds DbTaskStore ad hoc instead of taking the injected TaskStore
`CONFIRMED` · impact **medium** · effort **S** · fartcode-core · domain
Files: `fartcode-core/src/tasks/operations.rs:315` · `fartcode-core/src/tasks/operations.rs:411` · `fartcode-core/src/tasks/mod.rs:129` · `fartcode-app/src/app.rs:95`

operations.rs:315 and :411: "let store = crate::tasks::DbTaskStore::new(self.db.clone(), self.event_bus.clone());" inside create() and provision(), even though the App already wires the canonical instance (app.rs:95 "let tasks = Arc::new(DbTaskStore::new(db.clone(), event_bus.clone()));") and the trait's own doc says it is "the surface used by the Tauri layer" (tasks/mod.rs:129-130). The service's db and event_bus fields exist partly to enable this reconstruction.

**Proposal:** TaskCreationService::new takes tasks: Arc<dyn TaskStore> alongside its other deps; create() and provision() call self.tasks. This restores the one seam the trait exists for (service tests can fake row commits without a real SQLite), and guarantees any future state DbTaskStore grows (caches, throttles) isn't silently forked across instances.

### DbTaskStore::delete spans four lock acquisitions with no transaction; create uses one
`CONFIRMED` · impact **medium** · effort **S** · fartcode-core · domain
Files: `fartcode-core/src/tasks/mod.rs:470` · `fartcode-core/src/tasks/mod.rs:492` · `fartcode-core/src/tasks/mod.rs:501` · `fartcode-core/src/tasks/mod.rs:509` · `fartcode-core/src/tasks/mod.rs:249`

delete() performs: workspace snapshot (with_conn #1, :481), DELETE FROM tasks (with_conn #2, :492), sibling COUNT (with_conn #3, :501), then DELETE workspaces + workspace_file_index + workspace_file_index_meta (with_conn #4, :509-522) — each releasing the mutex between steps, no transaction anywhere. create() in the same impl wraps its three inserts in conn.transaction() (:249-327). The comment at :476-478 concedes "workspaces has no FK to tasks, so bare DELETE leaks the row" — yet the compensating cleanup is itself non-atomic. IssueStore::enter_column similarly runs lane UPDATE + two renumber loops (n+m single-row UPDATEs) with no transaction (issues/mod.rs:770-815).

**Proposal:** Fold delete()'s row-side work into a single locked transaction mirroring create(): one guard, one tx doing task DELETE, sibling COUNT, and conditional workspace + index DELETEs; same one-tx treatment for enter_column's UPDATE + renumbers. The TOCTOU between COUNT and workspace DELETE closes because the guard is held throughout.

### TaskStore::provision is a dead Phase-0 stub that emits a false TaskProvisioned
`CONFIRMED` · impact **medium** · effort **S** · fartcode-core · domain
Files: `fartcode-core/src/tasks/mod.rs:156` · `fartcode-core/src/tasks/mod.rs:532` · `fartcode-app/src/commands/tasks.rs:408` · `fartcode-core/tests/tasks_integration.rs:343` · `fartcode-core/examples/smoke.rs:411`

The trait method (tasks/mod.rs:156-159) and impl (:532-552) only touch last_interacted_at, then emit InternalEvent::TaskProvisioned (:546) and task_provisioned telemetry — no worktree, no workspace bootstrap. Grep shows the only callers are tests/tasks_integration.rs:343 and examples/smoke.rs:411; every production path goes through TaskCreationService::provision (commands/tasks.rs:408-418 calls app.task_creation.provision, which does the real ensure_worktree work).

**Proposal:** Delete TaskStore::provision and DbTaskStore::provision; port the two test/example call sites to TaskCreationService::provision (or to a direct last_interacted_at touch). If the recency-touch behavior matters, keep it as an explicitly named touch_interacted(id) with no event.

### Eight domain enums hand-roll as_str/parse tables that duplicate their serde renames
`PARTIAL` · impact **medium** · effort **S** · fartcode-core · domain
Files: `fartcode-core/src/issues/mod.rs:61` · `fartcode-core/src/issues/columns.rs:55` · `fartcode-core/src/issues/columns.rs:86` · `fartcode-core/src/issues/columns.rs:115` · `fartcode-core/src/tasks/model.rs:28` · `fartcode-core/src/tasks/model.rs:67`

Lane, ColumnKind, OnEnter, OnSettle, TaskStatus, TaskType, HoldReason and WorkspaceTarget each carry a hand-written as_str() match plus (for five of them) a parse() match — ~20 lines apiece, ~150 lines total — while simultaneously declaring the same strings via #[serde(rename_all = "snake_case")] / #[serde(rename = ...)]. The DB stores as_str()'s output (e.g. enter_column writes lane.as_str(), issues/mod.rs:793) while the wire uses serde's name; nothing ties the two tables together.

**Proposal:** One declarative macro (string_enum! in a small crate::strings module, ~30 lines) generating the enum, as_str, and parse from a single variant→string table, with a per-enum error constructor parameter (InvalidIssueInput / InvalidBoardColumnInput / Internal); or derive Display/FromStr from serde via serde_plain. Either way the stored string and the wire string become the same declaration.

**Verifier caveat:** The dual-table claim holds for Lane, ColumnKind, OnEnter, OnSettle, TaskStatus, TaskType, and HoldReason, and the silent-coercion failure scenario is real (verified `Lane::parse(..).unwrap_or(Lane::Backlog)` at issues/mod.rs:361 and the ColumnKind/OnEnter/OnSettle unwrap_or fallbacks at columns.rs:451-455). But WorkspaceTarget has NO serde attributes and data-carrying variants (RepositoryInstance{workspace_id}, Byoi{..}), so it cannot use the proposed macro, and HoldReason is as_str-only — the finding is really ~6 enums with both tables, not 8. Corrected scope; the macro still clears rule-of-three and fights no documented convention (§2's no-rename rule is about struct fields).

### Executable-on-PATH lookup implemented twice with drift; launcher consults both
`CONFIRMED` · impact **medium** · effort **S** · fartcode-core · infrastructure
Files: `fartcode-core/src/pty/launcher.rs:888` · `fartcode-core/src/dependencies/mod.rs:487` · `fartcode-core/src/pty/launcher.rs:222` · `fartcode-core/src/pty/tmux.rs:153`

`pty/launcher.rs::find_on_path` (888-916) searches PATH plus `common_bin_dirs()` fallbacks (~/.local/bin, ~/.bun/bin, ~/.cargo/bin, /opt/homebrew/bin…) with a unix 0o111 check but NO Windows PATHEXT handling (`#[cfg(not(unix))] return Some(candidate)` on any plain file). `dependencies/mod.rs::find_in_path` + `is_executable` (487-527) searches PATH ONLY (no GUI-launch fallback dirs) but handles PATHEXT correctly on Windows. `AgentLauncher::resolve_binary` (launcher.rs:222-240) consults both in sequence — cached dep-store detection (find_in_path) then find_on_path — so detection and launch can disagree about whether a provider binary exists depending on which list found it (a Dock-launched app with homebrew-installed CLI: dependency status says 'not installed', launch succeeds). tmux.rs:153 carries a third hardcoded fallback-dir list for the same GUI-PATH problem.

**Proposal:** One `fartcode_core::exec_lookup` module: `pub fn find_executable(names: &[&str], include_user_dirs: bool) -> Option<PathBuf>` owning PATH iteration, `common_bin_dirs()`, the unix mode check, and the PATHEXT check; `find_on_path` and `find_in_path` become thin calls (dependencies passes its detect_paths as `names` to keep names-outer ordering), and tmux's probe loop uses `common_bin_dirs()`.

### Agent binary resolution copy-pasted 4x; full agent-terminal launch pipeline duplicated 2x
`CONFIRMED` · impact **medium** · effort **S** · fartcode-app · command layer
Files: `fartcode-app/src/commands/terminals.rs:209` · `fartcode-app/src/commands/tasks.rs:262` · `fartcode-app/src/commands/provider_accounts.rs:243` · `fartcode-app/src/commands/provider_accounts.rs:312`

The sequence `fartcode_providers::get(id).ok_or_else(...)` then `provider.binaries.iter().find_map(|b| fartcode_core::pty::launcher::find_on_path(b)).ok_or_else(|| format!("agent not installed: ..."))` appears verbatim at terminals.rs:209-215, tasks.rs:262-268, provider_accounts.rs:243-252 and provider_accounts.rs:312-321, with the unknown-id error already drifting ("unknown agent: {agent}" vs "unknown provider: {provider_id}"). Above that, terminal_open_agent_blocking (terminals.rs:195-233) and launch_default_agent (tasks.rs:255-294) are near-identical pipelines (resolve binary → resolve_task_context → agent_env_removals → TerminalSpec{agent: Some(..)} → open), differing only in the explicit find_running_agent reattach check and rows/cols — two places the ADR-0033 one-agent-terminal-per-task and ADR-0034 env-removal rules must be kept in sync.

**Proposal:** Add `fartcode_providers::resolve_binary(provider_id) -> Result<(&'static ProviderDef, PathBuf), Error>` collapsing the 4 lookup sites (one canonical error text), and one `open_agent_terminal(app, terminals, task_id, provider, rows, cols)` helper (natural home: the task_flow module from the layering finding) that both terminal_open_agent and launch_default_agent call, so reattach + env-removal policy lives once.

### Dispatch and step engine duplicate agent resolution and finished-blocker prompt assembly
`CONFIRMED` · impact **medium** · effort **S** · fartcode-app · runtime
Files: `fartcode-app/src/dispatch.rs:115-128` · `fartcode-app/src/step_engine.rs:589-599` · `fartcode-app/src/step_engine.rs:616-624`

dispatch.rs:115-117 resolves the provider as `issue.provider` else `app.settings.get(&DEFAULT_AGENT)`; step_engine::resolve_agent (589-599) contains the identical two-arm fallback as its column-NULL branch, with a doc comment promising it "behaves byte-identically to E17-03 dispatch" — a contract currently enforced only by hand-synchronized code. Likewise the finished-blockers filter `issue.blockers.iter().filter(|b| b.counts_as_done).map(|b| b.title.clone()).collect()` feeding `build_dispatch_prompt` appears verbatim in dispatch.rs:122-127 and step_engine::step_prompt_for:617-622. Any change to what "finished" means or to provider precedence must be made twice or the two dispatch paths silently diverge.

**Proposal:** Move the blocker filter into fartcode-core next to the prompt builder — either `build_dispatch_prompt(issue)` computes finished titles itself or `Issue::finished_blocker_titles()` — and add one app-layer `resolve_issue_provider(app, &issue) -> Result<String>` that both dispatch.rs and resolve_agent's fallback arm call. The "byte-identical" contract becomes structural instead of copy-maintained.

### dossiers::on_pr_updated hand-writes SQL over core-owned pull_requests/issues
`CONFIRMED` · impact **medium** · effort **S** · fartcode-app · runtime
Files: `fartcode-app/src/dossiers.rs:359-417`

on_pr_updated locks the raw connection and runs `SELECT status FROM pull_requests WHERE url = ?1` (367-374) plus `SELECT i.id FROM issues i JOIN tasks t ON t.id = i.linked_task_id WHERE t.workspace_id = ?1 AND i.dossier_path IS NOT NULL` (376-386) — feature-envy of `app.pr_sync` (which owns the pull_requests cache, app.rs:174-177) and `app.issues`. It then re-reads every issue row it just selected ids for (398-401), and derives project consent from `issues.first()` (402). This is the only app-layer module that queries pull_requests directly; a pr_sync schema or status-vocabulary change ("open"/"merged" strings are matched here at 389-393) will not be found by looking at the store.

**Proposal:** Add `PrSyncStore::status_for_url(&self, url) -> Result<Option<PrStatus>>` (typed status enum, single home for the open/merged vocabulary) and `IssueStore::list_dossier_issues_by_workspace(&self, workspace_id) -> Result<Vec<Issue>>` to fartcode-core; on_pr_updated collapses to two store calls plus the existing consent/fan-out logic.

### fartcode-runtime: dormant worker crate ADR-0030 promised to retire, still shipping
`PARTIAL` · impact **medium** · effort **S** · satellite crates
Files: `fartcode-runtime/src/lib.rs:20` · `fartcode-runtime/src/bin/fartcode_acp_runtime.rs:26` · `fartcode-runtime/src/session_host.rs:36` · `fartcode-runtime/src/protocol.rs:108` · `decisions/0030-acp-runtime-in-app.md:34` · `fartcode-app/src/acp_runtime.rs:1`

ADR-0030 chose the in-app runtime (fartcode-app/src/acp_runtime.rs owns SessionManager, adapter spawned via AcpClient::spawn) and said the fartcode-acp-runtime worker 'stays dormant... Retiring or repurposing it gets its own ticket.' That ticket never happened: ~1,100 loc (protocol.rs 224, process_host.rs 219, session_host.rs 312, bin 339) plus tests/worker_integration.rs compile on every workspace build, and the only references to the crate anywhere are its own files and the workspace manifest. The bin maintains a second bespoke JSON-RPC framing layer (write_frame/reply/reply_error, bin lines 26-48) parallel to fartcode-acp/src/transport.rs:181-196 write_frame/respond/respond_error — two framing implementations that would have to evolve together if the worker ever woke up.

**Proposal:** Do the retirement ADR-0030 called for: delete the fartcode-runtime crate (git history is the archive; the ADR notes the env-injection invariant is preserved by construction in-app). If the prepare_session env-discard regression test is valued, move that one test's assertion next to fartcode-app::acp_runtime's env resolution. If out-of-process isolation ever returns, the ADR already names the right shape (an ACP-stdio proxy speaking real ACP so StdioTransport::from_child is the seam) — the bespoke worker protocol should not be the starting point.

**Verifier caveat:** The dead-code evidence fully holds: zero references outside the workspace manifest and the crate's own files, ~1,400 loc compiling every build, and the parallel JSON-RPC framing verified (bin :26-48 vs fartcode-acp/transport.rs:181-195). The ADR framing is overstated: ADR-0030 says the worker 'stays dormant', explicitly keeps its tests green, and defers 'retiring or repurposing' to a future ticket if out-of-process isolation returns — it did not promise retirement. Deleting is a new decision the ADR left open (and is reasonable to propose now), not the execution of a promised one.

### CliGit's bounded-subprocess invariant has three unbounded production escapes
`CONFIRMED` · impact **medium** · effort **S** · satellite crates
Files: `fartcode-git/src/lib.rs:57` · `fartcode-git/src/lib.rs:615` · `fartcode-git/src/lib.rs:665` · `fartcode-git/src/issues.rs:91`

The crate doc (lib.rs:55-59) states 'Every git invocation in this crate now goes through output_bounded, which kills the child at its deadline' — the #80 fix for commands hanging Tauri forever. Three production sites violate it: (1) is_worktree_clean (lib.rs:615-622) spawns raw std::process::Command::new("git") with .output() — unbounded, and also skips git_cmd's NON_INTERACTIVE_ENV; (2) config_get (lib.rs:661-668) builds via git_cmd but calls cmd.output() directly, unbounded; (3) issues.rs:91-104 runs `gh issue list` — a network call from a Tauri command — with no timeout at all, the exact failure class (#80: unreachable remote, caller never returns) the GitTimeout machinery exists for.

**Proposal:** Route all three through the existing runner: output_bounded(cmd, GitTimeout::Local, ...) for is_worktree_clean and config_get (both are disk-only; preserve config_get's exit-1→None mapping and is_worktree_clean's GIT_OPTIONAL_LOCKS=0 env on the git_cmd-built Command), and output_bounded(cmd, GitTimeout::Network, "gh issue list") in issues.rs — output_within is already generic over Command, so gh needs no new machinery. Then the doc comment becomes true again and greppable: any raw .output() in this crate is a bug.

### Live-agent probe and open-agent ritual re-implemented at three sites each
`CONFIRMED` · impact **medium** · effort **S** · frontend · state & IPC
Files: `app-frontend/src/lib/commands.ts:91` · `app-frontend/src/lib/commands.ts:349` · `app-frontend/src/store/steps.ts:230` · `app-frontend/src/lib/commands.ts:97` · `app-frontend/src/lib/commands.ts:107` · `app-frontend/src/store/steps.ts:249`

The probe `terminalListForTask(taskId) ... terms.find((t) => t.kind === "agent" && t.running)` appears in resumeAgentTab (commands.ts:91-92), the stop-agent command (commands.ts:349-351), and steps.ts hasLiveAgent (230-234) — the latter's comment explicitly notes "it is the same check the ⌘T resume path already makes (lib/commands.ts)". The spawn ritual `terminalOpenAgent → useScripts.noteAgentSpawn → addTab/focus` is likewise repeated in resumeAgentTab (97-104), openOmpTab (107-115), and runLaunchDirective (249-250). This is the exact area where the duplicate-agent-spawn bug was just fixed (commit 7e6d34b), so divergence here has already bitten once.

**Proposal:** lib/agent-terminal.ts: `liveAgentTerminal(taskId): Promise<TaskTerminalDto | null>` (with the useScripts.agentByTask fast path from hasLiveAgent) and `openAgentTerminal(taskId, agent, pane?): Promise<string>` doing open+noteAgentSpawn+addTab. resumeAgentTab, openOmpTab, stop-agent, and runLaunchDirective all collapse onto them, giving ADR-0033's one-agent-per-task rule a single enforcement point.

### Settings row kit (Row/InlineInput/InlineTextarea) duplicated verbatim across two panes
`CONFIRMED` · impact **medium** · effort **S** · frontend · components
Files: `app-frontend/src/components/ProjectSettings.tsx:38-153` · `app-frontend/src/components/ColumnsEditor.tsx:41-151` · `app-frontend/src/components/ProjectSettings.tsx:359-390` · `app-frontend/src/components/ProjectSettings.tsx:490-535` · `app-frontend/src/components/ColumnsEditor.tsx:288-323`

InlineInput is byte-for-byte identical between the two files (verified by diff of ProjectSettings.tsx:79-114 vs ColumnsEditor.tsx:75-110). Row differs only by ProjectSettings' `shared` tag prop; InlineTextarea only by ColumnsEditor's keysHint prop. Both files additionally hand-roll the same raw field idiom (defaultValue + Escape-stops-propagation-and-closes + save-on-blur-if-changed) six more times: remotes pair, workspaceProvider pair, scripts textareas in ProjectSettings; model/effort inputs in ColumnsEditor. Any change to the fc-set-* editing grammar (key hints, esc semantics, blur-save rules) is currently shotgun surgery across ~10 sites in two files.

**Proposal:** Create components/settings/fields.tsx exporting Row (with optional shared tag), InlineInput, InlineTextarea (with keysHint prop), and a BlurField wrapping the defaultValue/esc/blur-if-changed idiom. Both panes import; ColumnsEditor's copies and ProjectSettings' copies are deleted.

### Modal dialogs mirror sidebar-store bookkeeping via raw useSidebar.setState
`CONFIRMED` · impact **medium** · effort **S** · frontend · components
Files: `app-frontend/src/components/Modals.tsx:447-469` · `app-frontend/src/components/Modals.tsx:664-674`

CreateTaskDialog.submit patches useSidebar.setState directly with a comment admitting the duplication: "Mirror the sidebar store's createTask bookkeeping (that path hardcodes the name — see notes): append + select immediately" — including a race guard against the task:created refetch that store logic should own. DeleteTaskConfirm.doArchive likewise patches tasksByProject/selectedTaskId inline because "The sidebar event wiring has no task:archived handler". Store invariants (dedupe against event refetch, selection rules, archived filtering) now live in a modal component and must evolve in lockstep with store/sidebar.ts.

**Proposal:** Add two actions to store/sidebar.ts — createTaskWithOptions(projectId, name, opts) subsuming the append/select/pendingTitle dance, and archiveTask(projectId, taskId) (or a task:archived handler in wireSidebarEvents) — and have Modals.tsx call them. The setState blocks and their race commentary move next to the event wiring they race against.

### Async submit boilerplate (busy guard + String(e) + close-on-success) repeated 14+ times
`CONFIRMED` · impact **medium** · effort **S** · frontend · components
Files: `app-frontend/src/components/Modals.tsx:126-142` · `app-frontend/src/components/Modals.tsx:427-475` · `app-frontend/src/components/Modals.tsx:645-680` · `app-frontend/src/components/Modals.tsx:793-804` · `app-frontend/src/components/board/CardDetail.tsx:317-347` · `app-frontend/src/components/ColumnsEditor.tsx:585-648`

The exact shape `if (busy) return; setBusy(true); setError(null); try { await …; onClose(); } catch (e) { setError(String(e)); setBusy(false); }` appears in CreateProjectDialog, CreateTaskDialog, DeleteTaskConfirm (twice), ConfirmDelete, QuickTaskDialog, CardDetail (save/ask/moveForward), ColumnsPane (mutate/addColumn/deleteColumn), TokenGate (twice), and ChangesSidebar's provision button — 14+ hand-rolled copies with drifting details (some reset busy in finally, some only on error, some keep busy on success until close). Related smell: DeleteTaskConfirm and ConfirmDelete install their window keydown listeners in useEffect with NO dependency array (Modals.tsx:683-701, 808-818), re-registering a capture-phase listener on every render.

**Proposal:** Add lib/useAsyncAction.ts: useAsyncAction(fn, {onSuccess}) returning {busy, error, run, reset} with the guard/String(e)/finally policy decided once (a keepBusyOnSuccess flag covers the dialogs that close on success). Convert the 14 call sites; fix the two confirm key effects to depend on [] with a ref for the handler while touching them.

### Two independent POSIX shell-quoting implementations in fartcode-core
`CONFIRMED` · impact **medium** · effort **S** · cross-cutting duplication
Files: `fartcode-core/src/shell_escape.rs:17` · `fartcode-core/src/pty/mod.rs:273`

shell_escape::single_quote (shell_escape.rs:17-30) is the ARCHITECTURE §10.6 canonical quoter ("No ad-hoc quoting anywhere") used by tmux.rs, byoi.rs, remote.rs, fartcode-ssh/pty.rs, lifecycle.rs. pty/mod.rs:273-284 defines quote_shell_arg — the identical '\'' encoding plus a bare-word fast path and empty-string case — used only by wrap_with_stdin_pipe (pty/mod.rs:261,264), which builds a bash -c line for agent prompts. Quoting is security-sensitive (prompt text reaches sh); a fix to one implementation (e.g. a newline or locale edge case) will not reach the other.

**Proposal:** Move the bare-word fast path into shell_escape as quote_arg(input) (returns input verbatim when safe, else single_quote), delete pty::quote_shell_arg, and point wrap_with_stdin_pipe at it. Keep the round-trip-through-sh test from shell_escape.rs and add quote_shell_arg's metacharacter cases to it.

### workspaces.config JSON codec bypassed by raw-pointer access in two modules
`CONFIRMED` · impact **medium** · effort **M** · fartcode-core · domain
Files: `fartcode-core/src/tasks/operations.rs:663` · `fartcode-core/src/tasks/operations.rs:707` · `fartcode-core/src/tasks/deletion.rs:264` · `fartcode-core/src/tasks/deletion.rs:291` · `fartcode-core/src/tasks/byoi.rs:344` · `fartcode-core/src/tasks/byoi.rs:387`

operations.rs owns build_workspace_config / parse_git_setup / parse_workspace_target (the v2 {version, git, workspace} envelope). deletion.rs re-reads the same blob untyped: c.pointer("/git/fromBranch/branch") (deletion.rs:273) and provisioned_branch's git.get("kind")/get("branchName") chain (deletion.rs:295-306). byoi.rs reads v["workspace"]["remoteWorkspaceId"] raw (byoi.rs:348-351) and mutates the blob in place with value["workspace"]["remoteWorkspaceId"] = json!(machine_id) (byoi.rs:391-394), inventing {"kind":"byoi"} when the object is missing.

**Proposal:** New module fartcode-core/src/tasks/workspace_config.rs with a WorkspaceConfig type (GitSetup, WorkspaceTarget, remote_workspace_id) owning parse/serialize plus the derived accessors provisioned_branch() and set_remote_workspace_id(); move build_workspace_config/parse_* into it and rewrite deletion.rs and byoi.rs against it. A v2→v3 shape change becomes one file instead of a four-file shotgun, and the branch-deletion gate can no longer drift from the writer's shape.

### Conn-lock ritual reimplemented in 14 modules with 3 divergent mutex-poison policies
`CONFIRMED` · impact **medium** · effort **M** · fartcode-core · infrastructure
Files: `fartcode-core/src/db/connection.rs:175` · `fartcode-core/src/tasks/mod.rs:174` · `fartcode-core/src/projects/mod.rs:86` · `fartcode-core/src/conversations/mod.rs:206` · `fartcode-core/src/settings/service.rs:94` · `fartcode-core/src/line_comments/mod.rs:285`

db/connection.rs already has the right helper — `fn lock_conn` (line 175) — but it is private, so every domain reimplements it: identical `with_conn` closures in tasks/mod.rs:174, projects/mod.rs:86, conversations/mod.rs:206, settings/service.rs:94, tasks/operations.rs:393, projects/adoption.rs:395; identical `conn()->MutexGuard` helpers in pr_sync:207, ssh_connections:223, line_comments:285, issues/mod:475, issues/ledger:99, issues/columns:528; plus ~25 raw `.conn().lock().map_err(|_| Error::Internal("db … poisoned"))` inline sites (search.rs x8, settings/kv.rs x5, provider_accounts x5, view_state.rs:48, projects/{provider,remote,worktrees}, tasks/byoi …). Poison handling diverges in production code: stores return Err, fs_watch/mod.rs:364,388 and pty/launcher.rs:711 continue via `unwrap_or_else(PoisonError::into_inner)`. Bonus duplication: settings/kv.rs:70-74 re-writes the exact `INSERT … ON CONFLICT(key) DO UPDATE` upsert SQL that db/connection.rs:161-167 (kv_set_raw) already owns, instead of calling the Db trait's kv_set/kv_get/kv_delete.

**Proposal:** Add `pub trait DbExt` in db/connection.rs with a blanket `impl<T: Db + ?Sized> DbExt for T` providing `with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T, Error>)` and `with_tx` (unchecked_transaction wrapper), built on the now-pub `lock_conn` — generic methods on an extension trait keep `Arc<dyn Db>` object-safe. Delete the 14 per-module helpers and convert the inline sites; pick ONE poison policy (into_inner, matching SQLite semantics — a poisoned guard is still a valid connection) and document it there. Have KvStore.get/set/delete delegate to the Db trait methods, keeping SQL only for clear/get_all.

### Frontend event wire format hand-duplicated per variant; derived Serialize is dead
`CONFIRMED` · impact **medium** · effort **M** · fartcode-core · infrastructure
Files: `fartcode-app/src/app.rs:223` · `fartcode-core/src/events.rs:11` · `fartcode-app/src/app.rs:269` · `fartcode-app/src/app.rs:276`

`InternalEvent` derives `Serialize` with `#[serde(tag = "type", content = "payload", rename_all = "snake_case")]` (events.rs:11-13), but that serialization is used only by a unit test (events.rs:359). The real wire format is `event_to_value` in fartcode-app/src/app.rs:223-411 — a ~190-line match that re-encodes every forwarded variant into ad-hoc json!({}) with hand-camelCased keys and a hand-written `"type"` string per variant. Adding an event today means touching events.rs AND this match AND the frontend listener, and the mapping has already drifted: TaskCreated sends `"id"` (app.rs:269) while TaskRenamed/TaskDeleted/TaskStatusChanged send `"taskId"` (app.rs:276-281) for the same field.

**Proposal:** Make the enum itself the wire format: add `#[serde(rename_all_fields = "camelCase")]` to InternalEvent and a single `pub fn wire_name(&self) -> Option<&'static str>` table in events.rs (returning None for internal-only variants like IssueColumnChanged/PtyOutput). `event_to_value` collapses to `event.wire_name().map(|t| { let mut v = serde_json::to_value(payload); v["type"] = t; v })`. One place to add an event; the id/taskId inconsistency gets fixed once, coordinated with the frontend listener types.

### Task/workspace context resolution is raw SQL in the command layer, repeated four times
`CONFIRMED` · impact **medium** · effort **M** · fartcode-app · command layer
Files: `fartcode-app/src/commands/terminals.rs:34` · `fartcode-app/src/commands/git.rs:28` · `fartcode-app/src/commands/git.rs:355` · `fartcode-app/src/commands/line_comments.rs:275`

Four hand-written SQL queries against `app.db.conn().lock()` live in command modules: resolve_task_context (terminals.rs:34-60, tasks JOIN projects with a workspace-path COALESCE subquery), workspace_path (git.rs:28-50), project_push_remote (git.rs:355-383, tasks-by-workspace_id lookup), and reviewed_workspace_branch (line_comments.rs:275-292, tasks JOIN workspaces). Each repeats the `.lock().map_err(|_| "db connection mutex poisoned")` dance and re-derives task→workspace→project resolution that fartcode-core's stores own. ARCHITECTURE §10.4 says "Tauri commands are thin wrappers... No business logic in command handlers", and these helpers are imported across module boundaries (resolve_task_context by lifecycle.rs, tasks.rs, provider_accounts.rs; workspace_path/project_push_remote by github.rs, files.rs), making commands::git and commands::terminals load-bearing data-access modules.

**Proposal:** Add `fartcode_core::tasks::context` exporting `resolve_task_context(db, task_id) -> Result<TaskContext, Error>`, `workspace_worktree_path(db, workspace_id)`, and `effective_push_remote(db, settings, workspace_id)` with typed Error variants (TaskNotFound, WorkspaceNotFound already exist in the central enum). Command modules keep one-line wrappers that map to String; the four SQL blocks and the poisoned-mutex boilerplate collapse into core where the schema lives.

### Engines depend on command modules: task-creation/lifecycle logic lives in commands/*
`CONFIRMED` · impact **medium** · effort **M** · fartcode-app · command layer
Files: `fartcode-app/src/dispatch.rs:26` · `fartcode-app/src/commands/tasks.rs:303` · `fartcode-app/src/commands/lifecycle.rs:174` · `fartcode-app/src/commands/line_comments.rs:28` · `fartcode-app/src/step_engine.rs:88`

dispatch.rs:26 (`use crate::commands::tasks::create_task_params`) makes the board-dispatch engine — and transitively step_engine.rs via dispatch::provision_issue_task (step_engine.rs:88) — depend on a command module. create_task_params (tasks.rs:303-368) is 65 lines of branch-naming/base-ref/worktree-target policy, not a wrapper. Likewise run_auto_lifecycle_scripts + spawn_lifecycle_script (lifecycle.rs:99-213, script composition, env contract, dedupe) are imported by commands/tasks.rs:31 and commands/line_comments.rs:28. The dependency arrow points engine→command, inverting the layer contract stated in commands/mod.rs:1-3 ("thin wrappers over the domain services") and ARCHITECTURE §10.4; any refactor of the tasks or lifecycle command modules ripples into both engines.

**Proposal:** Extract an app-level `crate::task_flow` module owning create_task_params, launch_default_agent, launch_default_agent_after_setup, run_auto_lifecycle_scripts and spawn_lifecycle_script. commands/tasks.rs, commands/line_comments.rs, dispatch.rs and step_engine.rs all import from task_flow; commands/* returns to containing only #[tauri::command] fns plus DTO/request types, and the engine modules no longer name `commands::` at all.

### event_to_value: hand-written 25-arm serializer with shipped key drift
`CONFIRMED` · impact **medium** · effort **M** · fartcode-app · command layer
Files: `fartcode-app/src/app.rs:223` · `fartcode-app/src/app.rs:269` · `fartcode-app/src/app.rs:276` · `fartcode-core/src/events.rs:11`

InternalEvent already derives Serialize with `#[serde(tag = "type", content = "payload")]` (events.rs:11-13) — used only by a core unit test (events.rs:359). Production serialization is app.rs:223-411: ~190 lines of hand-written `json!` arms, one per consumed variant, each new event requiring an enum variant + a match arm + a test-arm (app.rs tests span 435-577). The hand mapping has already drifted: `TaskCreated` emits its task id as "id" (app.rs:269) while `TaskRenamed`/`TaskDeleted`/`TaskStatusChanged`/`TaskArchived`/`TaskRestored` emit "taskId" (app.rs:276-289), so frontend listeners for the same concept read different keys per event.

**Proposal:** Replace the match with data: `#[serde(rename_all = "camelCase")]` on the enum's variant fields plus a single `frontend_name(&self) -> Option<&'static str>` table (variant → "task:created"; None = internal-only). `event_to_value` becomes: look up name, `serde_json::to_value` the payload fields, insert "type". Field-key drift becomes impossible and adding an event touches one table row. Keep the current wire keys during migration by renaming the drifted arms deliberately (taskId everywhere) in one reviewed change.

### Task→worktree path resolution: 5 divergent raw-SQL copies in the app layer
`PARTIAL` · impact **medium** · effort **M** · fartcode-app · runtime
Files: `fartcode-app/src/acp_runtime.rs:340-396` · `fartcode-app/src/dossiers.rs:81-100` · `fartcode-app/src/commands/terminals.rs:34-60` · `fartcode-app/src/commands/git.rs:29-45` · `fartcode-app/src/commands/line_comments.rs:278-281` · `fartcode-core/src/tasks/deletion.rs:174`

"task → workspace path" is answered by five separate hand-written SQL resolvers in the app crate alone, each with different fallback rules: acp_runtime::resolve_cwd runs `SELECT path FROM workspaces WHERE id = ?1`, checks `Path::new(&path).is_dir()`, warns and falls back to project root; dossiers::task_worktree runs `SELECT w.path FROM tasks t JOIN workspaces w ON w.id = t.workspace_id`, checks is_dir, returns None with NO fallback; commands/terminals.rs resolve_task_context does it in one query with `COALESCE(...path IS NOT NULL AND path != ''..., p.path)` and NO is_dir check; commands/git.rs workspace_path errors on empty string; commands/line_comments.rs:278 repeats the join again. fartcode-core carries four more copies of the same join (tasks/deletion.rs:174, tasks/byoi.rs:335, projects/remote.rs:247, fs_watch/mod.rs:367). ARCHITECTURE.md §2 specifies a `fartcode-core/src/workspaces/` domain module — `ls fartcode-core/src/workspaces/` shows it does not exist — and §10 item 4 says command handlers are thin wrappers, yet the shell layer is running joins over core-owned tables. The divergence is a live bug surface: a workspace whose directory was pruned resolves to project root in ACP, to None in dossiers, and to the stale path in terminals.

**Proposal:** Create the `fartcode_core::workspaces` module ARCHITECTURE.md already promises, with two named resolvers: `worktree_path_for_task(db, task_id) -> Result<Option<PathBuf>>` (validated: non-empty AND is_dir, one definition of "materialized") and `task_cwd(db, task_id) -> Result<TaskCwd { project_id, project_path, cwd }>` (the project-root-fallback variant). Migrate all five app-layer call sites and the four core joins onto them; dossiers/telemetry/dossier_index keep their existing shared entry point (task_worktree) but it becomes a one-line delegate.

**Verifier caveat:** The app-layer half holds and the divergence is verified (acp_runtime: is_dir check + warn + project-root fallback; dossiers::task_worktree: is_dir, None, no fallback; terminals: COALESCE with empty-string check but NO is_dir; git.rs: workspace-id-keyed, errors on empty; line_comments:278: bare join, no checks), and fartcode-core/src/workspaces/ promised by ARCHITECTURE §2 indeed does not exist. But the four core-side "copies" are overstated: deletion.rs selects a 5-column workspace snapshot, remote.rs filters location='remote' for ssh_connection_id, byoi.rs filters kind='byoi' for config, and fs_watch bulk-scans all non-archived tasks — different SELECTs and WHEREs that would not collapse onto the two proposed resolvers. Migrate the five app sites; leave the core queries alone. Impact corrected to medium: the fallback divergence is a hazard/inconsistency, each branch locally deliberate.

### TerminalManager::open is a 250-line god function with a 90-line inline pump thread
`CONFIRMED` · impact **medium** · effort **M** · fartcode-app · runtime
Files: `fartcode-app/src/terminals.rs:263-512` · `fartcode-app/src/terminals.rs:417-509`

open() (263-512) interleaves five concerns: agent dedupe, remote route resolution (286-308), tmux spawn-plan construction (310-348: slot pick, session naming, sh -c wrapping, TERM/PATH overlay), spawn + failure rollback (365-395), and an inline std::thread::spawn closure (417-509) that is the entire output pump — chunked reads, base64 emit, lifecycle exit-line rendering, exit-code recording, slot release, terminal:exited emit, the settle hook (flip_for_exited_agent), and the retain-vs-drop entry policy. The tmux/remote/plain spawn-plan matrix — the trickiest logic in the file per its own ADR citations (ADR-0025/0028, E12-05, E13-02) — is unreachable by unit tests: the tests module (751-776) covers only release_slot, because exercising the plan requires a real PTY spawn.

**Proposal:** Split open() into `fn resolve_spawn_plan(&self, &TerminalSpec) -> Result<SpawnPlan, Error>` (pure given the route + live-session list: cmd, args, env, tmux_session_id, remote handles) and `fn spawn_output_pump(entry: Arc<Entry>, app, terminals, slots)` holding the loop. The plan function gets direct unit tests over the tmux/remote/args matrix; the pump's exit-handling branch (450-505) becomes a named `fn on_pty_exit(...)` instead of a closure tail.

### step_engine.rs fuses a pure state machine, orchestration, and 1300 test lines
`CONFIRMED` · impact **medium** · effort **M** · fartcode-app · runtime
Files: `fartcode-app/src/step_engine.rs:74-491` · `fartcode-app/src/step_engine.rs:550-1325` · `fartcode-app/src/step_engine.rs:1327-2640` · `fartcode-app/src/step_engine.rs:406-442` · `fartcode-app/src/step_engine.rs:1013-1039`

The 2640-line headline is really three files fused: (1) StepEngine (74-491) is a self-contained mutex-guarded state machine — parks, launch registry, consumed-session epochs, chain state — with zero App/DB dependency; (2) orchestration free functions (550-1325: enter_column, confirm_step, settle_issues_observed, chain_guard) that touch stores and the bus; (3) `mod tests` from 1327 to 2640. The app's most intricate invariants (settle epochs, tombstones, repark-on-restart, atomic park-take) live in (1) but can only be tested through App::init(":memory:") fixtures. Within (1), forget_project (406-442) iterates `st.parked` twice with the identical `p.project_id == project_id` filter (the `chained` collect at 419-425 and the `dropped` collect at 428-433) — a fold-into-one-pass cleanup a decomposition pass would catch. In (2), the post-Act side-effect block of settle_issues_observed (1013-1039) is an unlabeled grab-bag — dossier reindex, telemetry observation, ledger token backfill, chain bookkeeping — four best-effort calls inline before the hold/advance match.

**Proposal:** Convert to a `step_engine/` directory module: `state.rs` (StepEngine + SettleDecision + ParkTake + ChainState, with its own unit tests needing no DB), `mod.rs` (enter/confirm/settle orchestration), `tests.rs` (the existing App-fixture integration tests). Extract the settle side-effect block into `fn record_settled_step(app, issue, column, session, transcript)` and collapse forget_project's duplicate parked iteration into one pass. Note: the deeper cut the module docs hint at (trigger evaluation vs board mutation) is already done — board mutation lives in core's issues.enter_column — so state-vs-orchestration is the honest seam.

### Ten hand-rolled event subscriptions; two miss the disposed-race guard the rest have
`CONFIRMED` · impact **medium** · effort **M** · frontend · state & IPC
Files: `app-frontend/src/store/sidebar.ts:336` · `app-frontend/src/store/tabs.ts:445` · `app-frontend/src/store/changes.ts:112` · `app-frontend/src/store/pr.ts:87` · `app-frontend/src/store/diffs.ts:228` · `app-frontend/src/store/line-comments.ts:124`

Eight wireXEvents() functions installed from App.tsx plus three lazy module-level wirings (columns.ts wireEviction, taskCard.ts wireEvents, scripts.ts wireExitEvents) each re-implement the same boilerplate: promise-based listen(), captured unlisten, and a disposed flag to close the race where cleanup runs before listen resolves. Six copies have the `let disposed = false; ... .then((off) => { if (disposed) off(); else unlisten = off; })` guard; sidebar.ts:336-413 and tabs.ts:445-453 do NOT (`return () => unlisten?.()` with unlisten still null). Under React.StrictMode (enabled in main.tsx) the first mount's cleanup fires before listen resolves, permanently leaking a duplicate subscription in dev — every task event then triggers double listTasks refetches. Ten separate native listen() registrations also mean every backend event is deserialized and switch-matched ten times.

**Proposal:** lib/event-bus.ts: one listen("fartcode:event") for the process, plus a typed `onBusEvent(type | type[], handler): () => void` registry keyed by the discriminant. Store wirings become one-line reducer registrations; the disposed race is solved once in the bus; sidebar/tabs get the fix for free. The lazy module wirings (columns, taskCard) also stop needing their wired-boolean pattern.

### Per-key async-cache store shape duplicated seven times across stores
`CONFIRMED` · impact **medium** · effort **M** · frontend · state & IPC
Files: `app-frontend/src/store/changes.ts:36` · `app-frontend/src/store/commit-state.ts:42` · `app-frontend/src/store/pr.ts:30` · `app-frontend/src/store/diffs.ts:74` · `app-frontend/src/store/editors.ts:44` · `app-frontend/src/store/columns.ts:56`

changes.ts, commit-state.ts, pr.ts, diffs.ts, editors.ts each declare a module-level `const inflight = new Set<string>()`, an `EMPTY` entry, an identical `patch(key, part)` spread helper, a `fetchX` that add/deletes from inflight and writes {payload, loading:false, error:null} vs {loading:false, error:String(e)}, and ensure/refetch that gate on `entry?.X || inflight.has(key)`. columns.ts (fetchInto + load/reload + loading/loaded Records) and taskCard.ts (fetchInto + load/reload + loading/loaded, explicitly commented "Load-and-cache like store/columns.ts") are a sixth and seventh instance of the same contract with a loaded-flag variant. Any policy change (retry, error normalization, eviction) is currently a seven-file edit.

**Proposal:** lib/keyed-resource.ts: `createKeyedResource<T>(fetcher: (key: string) => Promise<T>)` returning a zustand slice fragment {byKey, ensure, refetch, patch} with in-flight dedupe and the loading/error envelope built in, plus an opt-in {loaded} flag variant for the columns/taskCard shape. Each store composes the slice and keeps only its domain actions (stage/commit/save/...).

### Right sheet's five modes are four unsynchronized booleans policed by hand at 6+ sites
`CONFIRMED` · impact **medium** · effort **M** · frontend · components
Files: `app-frontend/src/store/ui.ts:29-40` · `app-frontend/src/components/ChangesSidebar.tsx:85-107` · `app-frontend/src/components/ChangesSidebar.tsx:211-242` · `app-frontend/src/components/board/CardDetail.tsx:124-131` · `app-frontend/src/components/TaskChatPanel.tsx:53-57` · `app-frontend/src/components/board/BoardView.tsx:523-528`

ui.ts holds changesOpen/projectChatOpen/taskChatOpen/fileTreeOpen/boardDetailIssueId as independent fields; ChangesSidebar.tsx comments "they alternate, never stack" but nothing enforces it — every opener/closer hand-maintains the invariant (CardDetail.close sets two flags, TaskChatPanel's and ProjectChatPanel's close buttons set two flags with the comment "close the sheet, not just switch modes", BoardView.openCard sets detail id + changesOpen). Mode precedence is computed twice in ChangesSidebar (changesVisible at 95-102 vs the render ternary chain at 211-242), and they already drift: showTaskChat is derived at line 88 but the render branch at 240 re-derives `taskId && taskChatOpen`. The sheet header (title + ref + × close) is also duplicated at ChangesSidebar.tsx:220-237 vs 251-268 and again in both chat panels.

**Proposal:** Replace the booleans with one discriminated field in useUi: sheet: null | {kind: "changes"|"cardDetail"|"projectChat"|"taskChat"|"files"; issueId?: string}, with openSheet(mode)/closeSheet() actions owning the exclusivity. ChangesSidebar becomes a switch over sheet.kind; extract a SheetHeader({title, meta, onClose}) used by all five modes. All six call sites shrink to one action call and the two-precedence-computations problem disappears.

### Issue-list fetch + event resubscription re-implemented 4x beside an existing issues store
`CONFIRMED` · impact **medium** · effort **M** · frontend · components
Files: `app-frontend/src/components/board/BoardView.tsx:211-255` · `app-frontend/src/components/board/CardDetail.tsx:137-201` · `app-frontend/src/components/ColumnsEditor.tsx:546-574` · `app-frontend/src/store/taskCard.ts:44-96` · `app-frontend/src/components/projectChat/TicketEditCard.tsx:38`

Four surfaces each call issueList(projectId) into local state and each install their own onFartcodeEvent subscription over near-identical event sets. store/taskCard.ts is already a proper event-wired issue cache and its header comment admits the split: "BoardView keeps its issues in component state". The copies have diverged: CardDetail additionally reloads on step:chain_held (CardDetail.tsx:188-195), BoardView additionally reloads on task:deleted/task:status_changed (BoardView.tsx:237-239), ColumnsEditor on neither. With board + card detail + columns pane open, one issue:updated event triggers up to four identical issueList round-trips over the Tauri bridge.

**Proposal:** Promote store/taskCard.ts into store/issues.ts: useIssues with byProject/loaded/error, one process-wide event wiring covering the union event set (including step:chain_held), plus a selectIssue(projectId, issueId) helper. BoardView, CardDetail, ColumnsPane, and TicketEditCard select from it and drop their private fetch+subscribe effects (~150 lines deleted); TaskHeader keeps working unchanged. New step events then get added in exactly one place.

### BoardView is a 1409-line god component; its drag engine is a separable 250-line subsystem
`CONFIRMED` · impact **medium** · effort **M** · frontend · components
Files: `app-frontend/src/components/board/BoardView.tsx:129-184` · `app-frontend/src/components/board/BoardView.tsx:660-748` · `app-frontend/src/components/board/BoardView.tsx:817-879` · `app-frontend/src/components/board/BoardView.tsx:917-935`

One component owns issue loading, step-park reconciliation, consent gating, confirm overlays, the keyboard column/card cursor, narrow-mode layout, AND a complete pointer-drag implementation: five state/ref clusters (dragId/over/dragPos + dragOrigin/draggedNow/dragPosRef/dragGrab, 129-184), hit-testing (dropIndex/columnElAt/updateDragOver 660-690), commit logic (commitDragAt 694-712), an rAF edge auto-scroll loop (719-748), four per-card pointer handlers threaded through props (817-879), and the ghost renderer (917-935). None of it touches board semantics — it only needs columns/issues and two callbacks (reorder, requestMove).

**Proposal:** Extract useBoardDrag({issues, columns, onReorder, onMove}) into board/useBoardDrag.ts returning {ghost, dropLineFor(columnId,index), cardHandlers(issue), draggingId}. BoardView drops ~250 lines and future drag fixes (touch support, drop-line math) stop churning the file every board feature also lands in; the keyboard cursor and confirm gating become independently readable.

### Test fixtures git_ok/make_repo/fixture/DbHold copy-pasted across 18+ files
`CONFIRMED` · impact **medium** · effort **M** · cross-cutting duplication
Files: `fartcode-app/tests/create_task_params.rs:15` · `fartcode-app/tests/dispatch_integration.rs:20` · `fartcode-app/src/commands/issues.rs:575` · `fartcode-app/src/commands/steps.rs:391` · `fartcode-git/src/status.rs:347` · `fartcode-core/src/fs_watch/mod.rs:414`

`fn git`/`git_ok` (run git, assert success) exists 18 times: 10 copies in fartcode-app/tests/*.rs, plus commands/git.rs:403+413, projects.rs:128, line_comments.rs:339, fs_watch/layout.rs:75, fs_watch/mod.rs:414, fartcode-git/src/status.rs:347, git2ops.rs:350. `make_repo` (init repo, commit README with inline user.name/user.email -c flags, branch -M main) is copied in 10 integration files (create_task_params.rs:24, dispatch/dossier*/skills/task_creation/tasks_terminals/telemetry _integration.rs). The 26-line DbHold mutex-holder struct is verbatim-identical in commands/issues.rs:575-600 and commands/steps.rs:391-416. A single fixture policy change (e.g. GIT_CONFIG_GLOBAL isolation so a host gpgsign config can't break CI) is currently an ~15-file edit.

**Proposal:** fartcode-app/tests/common/mod.rs exporting git_ok, make_repo, and the App fixture builder; move DbHold into a #[cfg(test)] test_support module in fartcode-app/src; give fartcode-git a `test-util` feature exporting its git helper for the in-crate unit tests.

### The 2,640-line board step engine is domain policy living in the Tauri shell
`PARTIAL` · impact **medium** · effort **L** · architecture & layering
Files: `fartcode-app/src/step_engine.rs:74` · `fartcode-app/src/dispatch.rs:17` · `fartcode-core/src/issues/columns.rs:1` · `fartcode-core/src/issues/ledger.rs:1`

step_engine.rs implements the board pipeline runner — enter/park/settle state machine, settle epochs, chain guard (depth/cycle/budget), restart contract — and its imports (lines 74-88) are exclusively fartcode_core types plus crate::app::App and crate::dispatch::provision_issue_task; its only two `tauri` references are in tests (lines 1747, 1789). Meanwhile its sibling domain — ColumnStore, StepLedgerStore, blocked derivation, build_dispatch_prompt — lives in fartcode-core::issues. The one domain (ADR-0037 pipeline) is split across two crates, and ARCHITECTURE.md §10 rule 4 says business logic does not live in the shell. dispatch.rs's only shell dependency is `use tauri::Manager` (line 17) to fetch App state, which callers could pass in.

**Proposal:** Move the engine to `fartcode_core::issues::step_engine` as a StepEngineService struct holding the Arcs it actually uses (issues, columns, ledger, tasks, event_bus, db, task_creation); move provision_issue_task from app::dispatch into core::issues alongside build_dispatch_prompt. fartcode-app keeps only the thin command wrappers in commands/steps.rs and the Manager-based state lookup.

**Verifier caveat:** The facts verify (2,640 lines; tauri appears only at test lines 1747/1789; imports are core types + App + dispatch; the sibling ColumnStore/StepLedgerStore/build_dispatch_prompt live in core::issues), but the layering citation overreaches — §10 rule 4 forbids business logic in COMMAND HANDLERS, not in fartcode-app modules generally. More important, the proposal understates the move: provision_issue_task (dispatch.rs:203-208) calls crate::dossiers::create_for_task and crate::skills::seed_for_task, app-owned by ADR-0038's documented split (core owns dossier content/file ops, app owns consent-gated lifecycle), so pulling it into core drags that boundary along or needs injected hooks — and it also calls create_task_params, which the task_flow finding keeps app-side. Effort is L, not M; a real cleanup but a bigger and more design-entangled one than proposed.

### Settings registry: adding a group requires touching 4 parallel per-key lists
`PARTIAL` · impact **low** · effort **S** · fartcode-core · infrastructure
Files: `fartcode-core/src/settings/registry.rs:206` · `fartcode-core/src/settings/registry.rs:216` · `fartcode-core/src/settings/registry.rs:231` · `fartcode-core/src/settings/registry.rs:250`

Every app-settings group appears in four hand-synced places in registry.rs: the typed statics (206-213), the `all_keys()` string list (216-227), the `default_value()` match (231-244), and the `canonical_value()` match (250-281) — the last two with structurally identical `serde_json::from_value::<G>(v.clone()).and_then(to_value)` arms per key. The lists can silently drift (all_keys order already differs from the statics' declaration order; a key added to the statics but forgotten in canonical_value would make set_json reject it as InvalidSettingKey).

**Proposal:** A single declarative macro `settings_groups! { PROJECT: "project" => ProjectGroup, TASKS: "tasks" => TaskGroup, … }` that expands to the statics plus a `static REGISTRY: &[SettingDescriptor]` where `SettingDescriptor { name, default: fn() -> Value, canonicalize: fn(&Value) -> Result<Value, Error> }` is monomorphized per group; `all_keys`, `default_value`, and `canonical_value` become lookups over REGISTRY. One line per new group, impossible to half-register.

**Verifier caveat:** The four lists exist as cited and their orders already differ, but two corrections shrink the finding: all_keys() has ZERO consumers anywhere in the repo (dead code — deletable today, reducing the problem to 3 sites), and a group forgotten in canonical_value fails LOUDLY as InvalidSettingKey on first set_json, not silently — the only silent drift is the inert ordering. With 8 groups the rule of three is met, but the macro buys one line per rare new group against a loud runtime failure mode; impact is low.

### POSIX shell quoting duplicated: pty::quote_shell_arg vs shell_escape::single_quote
`CONFIRMED` · impact **low** · effort **S** · fartcode-core · infrastructure
Files: `fartcode-core/src/pty/mod.rs:273` · `fartcode-core/src/shell_escape.rs:17`

ARCHITECTURE.md §10.6: 'Shell quoting via the shared module. No ad-hoc format!(…) or manual escaping. Call fartcode_core::shell_escape::quote(input).' Yet pty/mod.rs:273-284 defines `pub fn quote_shell_arg` implementing the same `'…'` + `'\''` POSIX escaping as shell_escape::single_quote (17-30), with an added bare-word fast path, and wrap_with_stdin_pipe (258-270) builds the stdin-pipe shell line from it. Two independent implementations of the encoding that keeps hostile prompts from breaking out of the `printf '%s\n' <prompt> | <cli>` line must now evolve together.

**Proposal:** Move the fast-path variant into shell_escape as `pub fn quote(arg: &str) -> String` (bare-word passthrough + single_quote fallback), delete pty::quote_shell_arg, and point wrap_with_stdin_pipe and its tests at shell_escape. The module doc's claim of being 'the single canonical place for quoting rules' becomes true.

### default_shareable defined three times; the public registry one is dead
`CONFIRMED` · impact **low** · effort **S** · fartcode-core · infrastructure
Files: `fartcode-core/src/settings/registry.rs:422` · `fartcode-core/src/settings/service.rs:760` · `fartcode-core/src/settings/service.rs:140`

`registry.rs:422-432` exports `pub fn default_shareable()` (ShareableProjectSettings with DEFAULT_PRESERVE_PATTERNS) — grep across the workspace finds no caller. `service.rs:760-770` defines a private byte-identical `fn default_shareable()` used at service.rs:196 and 411. A third inline copy of the same DEFAULT_PRESERVE_PATTERNS→Vec<String> materialization sits in seed_project_settings (service.rs:140-149). Three sites must stay in sync for provenance tagging to keep reporting seeded defaults as "default" rather than "local" (the invariant service.rs:397-425 depends on).

**Proposal:** Delete service.rs's private copy and the inline seed materialization; have both call the existing `registry::default_shareable()` (it is already the schema-owning module). One definition, and the seed/provenance equality contract is by-construction.

### LLM title summarization (~90 lines, subprocess spawns) embedded in tasks command module
`CONFIRMED` · impact **low** · effort **S** · fartcode-app · command layer
Files: `fartcode-app/src/commands/tasks.rs:132` · `fartcode-app/src/commands/tasks.rs:177` · `fartcode-app/src/commands/tasks.rs:184`

tasks.rs:132-222 contains naming policy (SUMMARIZE_NAME_THRESHOLD, OLLAMA_TITLE_MODEL), a prompt template, and two subprocess drivers (summarize_via_claude, summarize_via_ollama) with output scrubbing (first_line_title) — none of it command-shaped. It also re-implements the ADR-0034 auth rule as a one-off `.env_remove("ANTHROPIC_API_KEY")` (tasks.rs:184) with a comment pointing at terminals.rs, instead of using the existing agent_env_removals resolution — a second copy of the billing-flip rule that will not follow provider-account changes.

**Proposal:** Move the summarizer into `fartcode_core::tasks::naming::summarize_title` (or an app-level title_summary.rs) behind a small `TitleSummarizer` trait so the CLI calls are stubbable; have it take the removal list from provider_accounts::resolve_removals rather than a hardcoded env_remove. commands/tasks.rs keeps only the fire-and-forget hook calling app.tasks.rename on completion.

### Broadcast-subscriber loop skeleton hand-rolled four times
`CONFIRMED` · impact **low** · effort **S** · fartcode-app · runtime
Files: `fartcode-app/src/watchers.rs:34-57` · `fartcode-app/src/indexer.rs:27-92` · `fartcode-app/src/dossiers.rs:430-448` · `fartcode-app/src/app.rs:415-433`

Four spawned subscribers repeat the identical skeleton `loop { match rx.recv().await { Ok(e) => ..., Err(RecvError::Lagged(_)) => continue, Err(RecvError::Closed) => break } }` — watchers.rs:36-56, indexer.rs:44-51, dossiers.rs:434-446, app.rs:419-431 — each re-stating the lag/closed policy in its own comment ("a lagging subscriber drops events but must survive"). Only dossiers.rs wraps its handler in spawn_blocking; a future subscriber doing filesystem work has to remember that distinction from scratch. Every new event consumer re-decides backpressure policy that should be decided once.

**Proposal:** Add one helper in fartcode-app (e.g. `events::spawn_subscriber(bus: Arc<BroadcastEventBus>, name: &'static str, blocking: bool, handler: impl FnMut(InternalEvent))`) that owns the loop, the Lagged/Closed policy, and the optional spawn_blocking offload. The four call sites shrink to their handler bodies; the AGENTS.md "never block the runtime worker" rule becomes a flag instead of tribal knowledge.

### Registry sweep/lookup boilerplate repeated across three runtime registries
`CONFIRMED` · impact **low** · effort **S** · fartcode-app · runtime
Files: `fartcode-app/src/terminals.rs:708-735` · `fartcode-app/src/terminals.rs:661-678` · `fartcode-app/src/port_forwards.rs:131-144` · `fartcode-app/src/step_engine.rs:457-484` · `fartcode-app/src/step_engine.rs:406-442`

The runtime holds four Mutex<HashMap<String, _>> registries (TerminalManager.terminals, PortForwardService.tunnels, AcpRuntime.clients, RemotePtyRegistry.managers). Their entries and lifecycles differ enough that a generic Registry type is NOT warranted — but two mechanical shapes recur verbatim: (a) the scoped sweep "collect ids matching predicate, remove each, act on removed" at terminals::close_task (712-721), port_forwards::stop_for_connection (134-139), step_engine::take_parks_for_column (459-483), and step_engine::forget_project (406-442, where the hand-rolling already produced a duplicated second pass); (b) the lookup boilerplate `self.terminals.lock().get(id).cloned().ok_or_else(|| Error::Internal(format!("terminal not found: {id}")))` copied character-for-character in write (663-665) and resize (673-675), with two more bare `.lock().get(id).cloned()` in tail (651) and wait_for_exit (627).

**Proposal:** Two small helpers, not an abstraction: `fn drain_where<K: Eq+Hash+Clone, V>(map: &mut HashMap<K, V>, pred: impl Fn(&K, &V) -> bool) -> Vec<(K, V)>` in a shared app util (collapses the four sweeps and prevents the forget_project double-iteration class of bug), and a private `TerminalManager::require(&self, id) -> Result<Arc<Entry>, Error>` for the not-found lookup. Leave the registries themselves separate.

### Dead public functions: flip_issues_for_conversation and tmux_for_connection
`CONFIRMED` · impact **low** · effort **S** · fartcode-app · runtime
Files: `fartcode-app/src/dispatch.rs:230-232` · `fartcode-app/src/remote_pty.rs:125-127`

Repo-wide grep shows `flip_issues_for_conversation` (dispatch.rs:230, `pub fn flip_issues_for_conversation(app: &App, conversation_id: &str) { settle_conversation(app, conversation_id, None) }`) has zero callers — only its definition and a doc-link from the _observed variant that superseded it in acp_events.rs:144. `RemotePtyRegistry::tmux_for_connection` (remote_pty.rs:125-127) likewise has zero callers anywhere; teardown paths use `remote_tmux_for_task` on the terminal manager instead. Both are `pub` in a lib crate, so rustc's dead_code lint never fires on them.

**Proposal:** Delete both functions (settle_conversation stays — the observed variant uses it). If the identity-less ACP wrapper is being kept deliberately for a future caller, demote it to pub(crate) with a #[allow] and a note; as-is it is an untested second entry point into the settle path.

### GitHub remote-URL prefix parsing duplicated between fartcode-git and core
`CONFIRMED` · impact **low** · effort **S** · satellite crates
Files: `fartcode-git/src/remote.rs:44` · `fartcode-core/src/github/client.rs:24`

remote.rs github_https_url and core's parse_github_slug contain the byte-identical normalization chain: strip_prefix("git@github.com:") .or_else https://github.com/ .or_else http://github.com/ .or_else ssh://git@github.com/, then .trim_end_matches('/').trim_end_matches(".git"). Any future URL shape (port-qualified ssh://git@github.com:22/, GHE hosts) must be added in both or the PR-sync target resolver (which uses parse_github_slug via pr_target.rs:45) and the footer's browse-on-GitHub affordance (github_https_url) silently disagree about what counts as a GitHub remote.

**Proposal:** Reimplement github_https_url on top of the core function: parse_github_slug(remote_url).map(|(owner, repo)| format!("https://github.com/{owner}/{repo}")) — fartcode-git already depends on fartcode-core and pr_target.rs already imports parse_github_slug. One prefix table left, in core, next to the client that consumes the slug.

### fartcode-ssh: dead pty/pty_with_size/resize_pty; channel-open sequence triplicated
`CONFIRMED` · impact **low** · effort **S** · satellite crates
Files: `fartcode-ssh/src/lib.rs:339` · `fartcode-ssh/src/lib.rs:362` · `fartcode-ssh/src/lib.rs:392` · `fartcode-ssh/src/lib.rs:420` · `fartcode-ssh/src/pty.rs:235` · `fartcode-ssh/src/pty.rs:253`

Repo-wide grep: the only PTY entry point ever called is pty_exec (from SshPtyManager::spawn, pty.rs:235). pty() (lib.rs:339-359), pty_with_size() (lib.rs:362-386), and resize_pty() (lib.rs:420-431) have zero callers — resizing goes through channel.window_change directly in the session task (pty.rs:253-255). The three openers are near-identical 20-line bodies (channel_open_session → maybe_forward_agent → request_pty → request_shell/exec) differing only in dimensions and shell-vs-exec, so the dead pair also duplicates the live one's sequence.

**Proposal:** Delete pty, pty_with_size, and resize_pty from SshClient. If an interactive-shell channel is ever needed (none of the terminal paths want one — agents and terminals both go through pty_exec/tmux), reintroduce it as a parameter on one private open_pty_channel(cols, rows, PtyRun::Shell | PtyRun::Exec(&str)) helper that pty_exec calls, so the forward-agent + request_pty sequence exists once.

### fartcode-integrations (11 loc) and fartcode-server (7 loc) are placeholder crates
`CONFIRMED` · impact **low** · effort **S** · satellite crates
Files: `fartcode-integrations/src/lib.rs:1` · `fartcode-server/src/main.rs:1` · `Cargo.toml:11` · `Cargo.toml:13`

fartcode-integrations/src/lib.rs is a doc comment plus a placeholder_compiles test asserting 1+1==2; fartcode-server/src/main.rs is a println placeholder for the Phase-3 workspace daemon. Nothing depends on either (grep matches only PRD.md/ARCHITECTURE.md/AGENTS.md planning references and one ADR mention). They are intentional Phase-0 scaffolding, but they have carried zero content for the project's whole life while every workspace build, test run, and clippy pass touches them, and their names occupy the two spots where readers look for issue-tracker and remote-daemon code that does not exist.

**Proposal:** Remove both from workspace members (a 2-line Cargo.toml diff restores each when its epic actually starts — E8/Phase 2 for integrations, E12-08/Phase 3 for server), or if the team wants the names reserved, collapse each to a lib.rs whose doc comment points at the PRD section and delete the placebo test. The scaffolding argument expired once real Phase 2/3 work (pr_sync, fartcode-ssh, BYOI) landed in other crates instead.

### commit-state has no event wiring; liveness parasitic on changes.ts, comment stale
`CONFIRMED` · impact **low** · effort **S** · frontend · state & IPC
Files: `app-frontend/src/store/commit-state.ts:3` · `app-frontend/src/store/changes.ts:121` · `app-frontend/src/store/changes.ts:128`

commit-state.ts:3-5 says "Refetched on the same git:changed/files:changed debounce as the changes snapshot (wireCommitStateEvents, called from changes.ts)" — but wireCommitStateEvents exists nowhere (grep matches only this comment). The actual refresh is inlined inside wireChangesEvents (changes.ts:128-131), and it is double-gated: the debounce is scheduled only when `workspaceId in useChanges.getState().byWorkspace` (changes.ts:121), so any surface that ensures commit-state without also ensuring changes gets a commit card that never refreshes on git events. Today CommitCard/GitFooter happen to live inside ChangesSidebar which ensures both (ChangesSidebar.tsx:78-79), so the coupling is invisible until someone reuses the card elsewhere.

**Proposal:** Export a real wireCommitStateEvents() from commit-state.ts (or a bus registration once the event-bus lands) that gates on useCommitState's own byWorkspace and shares the coalescer from the debounce helper. Delete the piggyback block in changes.ts and fix the stale comment.

### Dead IPC wrappers: issueDispatch, shareWithTeam, agentAddLineComment
`CONFIRMED` · impact **low** · effort **S** · frontend · state & IPC
Files: `app-frontend/src/lib/tauri.ts:1360` · `app-frontend/src/lib/tauri.ts:1350` · `app-frontend/src/lib/tauri.ts:248` · `app-frontend/src/lib/tauri.ts:1190` · `app-frontend/src/store/dossierConsent.ts:11`

issueDispatch (tauri.ts:1360-1362) and its DispatchOutcomeDto (1350-1356) have zero production call sites — only a vi.fn() mock in CardDetail.test.tsx:16; CardDetail actually dispatches via issueEnterColumn (CardDetail.tsx:382). dossierConsent.ts:11-12 still documents "CardDetail's Dispatch runs `issue_dispatch`, which provisions the worktree" — a rationale anchored to code that no longer exists. shareWithTeam (248-250) and agentAddLineComment (1190-1200, the E4-11 agent-tool surface that runs backend-side) have zero references outside tauri.ts.

**Proposal:** Delete the three wrappers and DispatchOutcomeDto (or fold its shape into StepLaunchInfoDto's doc, which references it at tauri.ts:1540), update the CardDetail test mock, and reword the dossierConsent comment to cite issueEnterColumn. Doing this before the god-module split keeps dead exports from being carried into the new lib/ipc/ modules.

### Editable-target keyboard guard hand-rolled 8x and diverging; canonical one unexported
`PARTIAL` · impact **low** · effort **S** · frontend · components
Files: `app-frontend/src/lib/useCommands.ts:22-29` · `app-frontend/src/components/Modals.tsx:38-42` · `app-frontend/src/components/board/BoardView.tsx:112-117` · `app-frontend/src/components/board/CardDetail.tsx:236-243` · `app-frontend/src/components/ChangesSidebar.tsx:150-151` · `app-frontend/src/components/PullRequestPanel.tsx:111-113`

Eight implementations of "is the key event in a text-entry element". They already disagree: BoardView/TaskPipelineOverlay/CardDetail/useCommands include SELECT, Modals/Drawer/ChangesSidebar/PullRequestPanel do not; only the unexported useCommands.ts version knows the xterm-helper-textarea exception ("a key sink, not a text editor"). On top of this, single-key handling is coordinated by hand across components: ChangesSidebar.tsx:167-178 delegates `r` to PullRequestPanel via an e.defaultPrevented handshake with a comment explaining that PullRequestPanel "preventDefaults without stopping propagation — skip when it already ran so one keypress never syncs twice".

**Proposal:** Export isEditableTarget (the useCommands.ts version, xterm exception included) from lib/registry or a new lib/dom-keys.ts and delete the seven locals. For the surface-scoped single keys (a/s/u/d, r, j/k), add a small useSurfaceKeys(ref, map) helper — or register them as E14 scoped commands — so the r-refresh double-handler handshake disappears.

**Verifier caveat:** All eight sites and the SELECT disagreement verified, as is the r-refresh defaultPrevented handshake at ChangesSidebar:167-178. But the proposal's core move — export the xterm-exception version and delete the seven locals — would regress Drawer.tsx, whose comment explicitly wants keys typed into the script's terminal to stay there (the xterm helper IS the terminal's key sink; treating it as non-editable would make `r` rerun the script mid-typing), and the same logic applies to any single-key surface hosting a terminal. The real shape is two intentional variants (app-chord guard with xterm exception; single-key guard without) plus unintentional SELECT drift — consolidate as one parameterized helper. Impact corrected to low.

### Three coarse relative-time formatters and seven copies of the 30s re-render tick
`PARTIAL` · impact **low** · effort **S** · frontend · components
Files: `app-frontend/src/components/TaskView.tsx:161-172` · `app-frontend/src/components/Nav.tsx:248` · `app-frontend/src/components/board/runState.ts:147-159` · `app-frontend/src/components/PullRequestPanel.tsx:62-69` · `app-frontend/src/components/CommentThread.tsx:41-43` · `app-frontend/src/components/board/BoardView.tsx:286-290`

TaskView.tsx:161 `ago(ts)` is annotated "(mirrors Nav.tsx)" — a literal duplicate of Nav.tsx:248 modulo iso-vs-epoch input; runState.ts:148 `elapsedShort` and PullRequestPanel.tsx:63 `elapsedOf` are two more coarse elapsed formatters with slightly different unit floors. The forcing tick — `const [, setTick] = useState(0)` + setInterval 30_000 — is copied in Nav, CommentThread, TaskView, PullRequestPanel, BoardView, and CardDetail (six components, seven instances counting CardDetail's gated one).

**Proposal:** Add lib/time.ts with useSlowTick(enabled = true) (one shared 30s interval) and agoShort(tsOrIso) as the single coarse formatter; keep elapsedShort as a re-export until callers migrate. Deletes two formatters and six interval effects.

**Verifier caveat:** The literal duplication is ago() ×2 (TaskView:161 self-annotated "mirrors Nav.tsx") and six 30s tick effects (grep finds 6, not 7: Nav, CommentThread, TaskView, PullRequestPanel, BoardView, CardDetail) — useSlowTick plus one shared ago() is a clean win. But the "three formatters" framing overreaches: elapsedShort's "30s" floor and elapsedOf's seconds-precision "38s" are deliberate per-context copy for running steps/checks where ago()'s "now" would be wrong, and unifying them changes rendered strings — which this project routes to design review (DESIGN.md is binding). Scope the dedupe to ago() + the tick.

### Selector-less useSidebar() subscriptions re-render sheet and modal host on every write
`CONFIRMED` · impact **low** · effort **S** · frontend · components
Files: `app-frontend/src/components/ChangesSidebar.tsx:52` · `app-frontend/src/components/Modals.tsx:1017`

ChangesSidebar.tsx:52 `const { projects, tasksByProject, selectedProjectId, selectedTaskId } = useSidebar();` and Modals.tsx:1017 `const { projects, selectedProjectId, deleteProject } = useSidebar();` subscribe to the entire sidebar store — the only two selector-less zustand subscriptions in components/ (grep-verified). Every tasksByProject refetch (fired by task:created/renamed/status events across all projects) re-renders the whole right sheet, including a mounted chat panel or CardDetail, even when the selected task is untouched. Every other component in the codebase already uses narrow selectors, including a documented convention about it (BoardView.tsx:79-81 stable-empty-array note).

**Proposal:** Replace both destructurings with per-field selectors (useSidebar(s => s.selectedTaskId) etc.), and in ChangesSidebar derive `task` with a memoized selector on (selectedProjectId, selectedTaskId) so sheet re-renders track selection, not the task-list cache.

### ADR-0034 CLI-login billing rule (strip API-key env) re-implemented at four sites
`PARTIAL` · impact **low** · effort **S** · cross-cutting duplication
Files: `fartcode-core/src/provider_accounts/mod.rs:278` · `fartcode-core/src/provider_accounts/mod.rs:309` · `fartcode-app/src/acp_runtime.rs:408` · `fartcode-app/src/commands/provider_accounts.rs:116` · `fartcode-app/src/commands/tasks.rs:184`

The rule "a CLI-login (OAuth) account must never see ANTHROPIC_API_KEY or it flips to API billing" lives canonically in resolve_env (mod.rs:278-291) and resolve_removals (mod.rs:309-326), but acp_runtime.rs:408-412 re-checks default_auth_method().kind == CliLogin before calling resolve_env (because resolve_env's empty-vec return is ambiguous with "no env vars", and the process-env fallback must be skipped); commands/provider_accounts.rs:116-127 re-derives is_login with the same Some(id)=>auth_method(id)/None=>default_auth_method() fallback that already appears at mod.rs:262-267 and :284-287; commands/tasks.rs:184 hardcodes .env_remove("ANTHROPIC_API_KEY") for the title summarizer instead of asking the store.

**Proposal:** One ProviderAccountStore::resolve_launch_auth(provider_id) -> AuthResolution { env, removals, is_cli_login } that all launch paths (terminal agent_env_removals, acp_runtime::provider_env, the summarizer) consume; fold the auth_method/default_auth_method fallback into a private effective_auth_method helper. The billing-safety rule then has exactly one implementation.

**Verifier caveat:** Half the sites hold: acp_runtime.rs:408-412's pre-check is genuinely forced by resolve_env's ambiguous empty-vec return (its own doc comment says so), and the Some(id)/None method fallback is duplicated inside provider_accounts/mod.rs itself (262-267 vs 284-287) — resolve_launch_auth with is_cli_login fixes both. But commands/tasks.rs:184 is a documented deliberate blanket rule for the background summarizer ("subscription auth must win"); routing it through the store would change behavior for api-key-account users. And provider_accounts.rs:116-127 is the add-account flow resolving a method from request params (should-we-store-a-secret), not the launch billing rule. The terminal path already delegates cleanly via agent_env_removals→resolve_removals. Impact corrected to low.

### fartcode-runtime (881 LOC dormant ACP worker) and fartcode-scheduler have no dependents
`PARTIAL` · impact **low** · effort **S** · cross-cutting duplication
Files: `Cargo.toml:3` · `fartcode-runtime/src/lib.rs:1` · `fartcode-scheduler/src/lib.rs:1` · `fartcode-app/src/acp_runtime.rs:1`

Grep of all workspace Cargo.tomls shows no crate depends on fartcode-runtime or fartcode-scheduler (477 LOC; the only edge is fartcode-runtime → fartcode-acp); the fartcode-acp-runtime worker binary appears nowhere in fartcode-app source, tauri.conf.json, or scripts. ADR-0030 and MEMORY.md line 1868 document the worker as DORMANT after the in-app AcpRuntime (fartcode-app/src/acp_runtime.rs) won — yet both crates still build in every CI run, fartcode-runtime's worker_integration test is flagged flaky (MEMORY.md:1738), and its protocol.rs env-discard rule ("SessionInput env is ALWAYS discarded") duplicates the in-app rule in acp_runtime.rs ("the renderer contributes nothing") — two copies of a security invariant, one unshipped.

**Proposal:** Delete fartcode-runtime and fartcode-scheduler from the workspace (git history preserves them for the phase that revives them), or at minimum move them to a non-default workspace profile so CI stops building/running the flaky dormant test. If the worker must stay, lift the shared env-discard invariant into fartcode-acp so the two hosts cannot drift.

**Verifier caveat:** The runtime half holds and acknowledges its documented decision: no crate depends on fartcode-runtime, ADR-0030/MEMORY document it DORMANT, MEMORY flags its worker_integration test flaky, and the env-discard invariant genuinely exists in both protocol.rs and the in-app runtime. The scheduler half does not: fartcode-scheduler is the implemented E11-01 cron core (477 LOC with invariants and restart recovery, depending on fartcode-core — the 'only edge is runtime→acp' claim is wrong), forward work for the automations epic the task model already stubs (automation_run_id, 'E11 hooks in later'), so deleting it fights the roadmap rather than removing leftovers. The defensible action shrinks to: quarantine/delete the runtime worker and lift the shared invariant; leave the scheduler.

### fartcode-git hosts a scheduler running raw SQL over core's task/workspace tables
`PARTIAL` · impact **low** · effort **S** · architecture & layering
Files: `fartcode-git/src/pr_sync.rs:112` · `fartcode-git/src/pr_sync.rs:155` · `fartcode-app/src/lib.rs:232`

list_sync_targets (pr_sync.rs:112-127) executes `SELECT DISTINCT t.workspace_id, w.path FROM tasks t JOIN workspaces w ... WHERE t.archived_at IS NULL AND w.path IS NOT NULL` through the re-exported Db handle, and run_scheduler (line 155) is a long-lived async loop spawned by the app shell. ARCHITECTURE.md §1 scopes fartcode-git to "worktrees, git ops, PR"; instead it now encodes domain schema knowledge (archived_at semantics, workspace-path validity) that must be chased whenever core's tasks/workspaces schema changes — the same task→workspace join duplicated in four app-crate sites.

**Proposal:** Move list_sync_targets into fartcode-core (next to PrSyncStore in core::pr_sync, or as part of the proposed workspaces::resolve helpers) and pass the engine a `targets: impl Fn() -> Result<Vec<(String, PathBuf)>>` or the store itself; fartcode-git keeps only PR-target resolution, GitHub fetch, and upsert mechanics. The scheduler loop itself can move to fartcode-app/watchers alongside the other lifecycle wiring.

**Verifier caveat:** The code reads as claimed (pr_sync.rs:112-133 raw tasks/workspaces join with archived_at semantics; run_scheduler spawned at app lib.rs:232), but the layering framing overstates: ARCHITECTURE §1 puts 'PR' inside fartcode-git's scope, §6.4 establishes the fartcode-git → fartcode-core dependency, and app.rs:62-63 documents the 'cache in core / engine in fartcode-git' split as intended — no documented rule is violated. Also the duplication count is wrong: the task→workspace join appears in 2 fartcode-app sites and 5 fartcode-core sites, so this query is not uniquely exotic knowledge. What survives is a modest cohesion point: moving list_sync_targets beside PrSyncStore in core is cheap and localizes schema semantics; the scheduler-loop move is optional.

### Frontend re-implements board placement policy the backend owns since the E18-07 flip
`PARTIAL` · impact **low** · effort **S** · architecture & layering
Files: `app-frontend/src/lib/columnConfig.ts:87` · `app-frontend/src/lib/tauri.ts:1279` · `fartcode-core/src/issues/columns.rs:10` · `fartcode-app/src/app.rs:66`

core columns.rs:10-14 states "Authoritative since the E18-07 flip (#66): issues.column_id owns board placement... issues.lane is a derived display mirror... nothing here (or anywhere) keys behavior off it", and app.rs:66-68 repeats it — but tauri.ts:1279-1282 still documents IssueDto.columnId as "lane stays authoritative until E18-07" and columnIdForIssue (columnConfig.ts:95-105) re-derives placement from lane via seedLane matching plus a landing-column fallback, duplicating the backend's seeding policy (columns.rs seed table, lines 266-334). Migration 0008 backfilled column_id on every row, so the lane fallback is dead-in-practice policy that resurfaces exactly during refetch races, when a card can render in a column its stale lane implies rather than where the backend placed it.

**Proposal:** Make columnId non-nullable in IssueDto at the Rust DTO boundary (the backend already guarantees it post-backfill), shrink columnIdForIssue to a map lookup with a single landing-column fallback for unknown ids, drop the seedLane branch from it and from blockerColumnName, and fix the stale comments in tauri.ts. Longer term, stop shipping lane in the DTO once no consumer reads it.

**Verifier caveat:** The stale comments are verified outright wrong (tauri.ts:1279-1282 and columnConfig.ts:93-94 both still say lane is authoritative until E18-07, contradicting columns.rs:10-14 and app.rs:66-68), and fixing them plus the non-null DTO is right. Weakened because MEMORY #66 records 'columnIdForIssue stays as defensive display resolution' as a deliberate part of the flip's landing — the seedLane fallback surviving is a documented keep the finding treats as pure residue; removing it (and blockerColumnName's lane arm) revises that decision rather than cleaning up an oversight. The refetch-race mis-render also needs the issue's columnId to be absent from the loaded columns list — transient and self-healing — so impact is low.

### ARCHITECTURE.md crate-graph rule is fiction: core depends on two crates plus reqwest
`PARTIAL` · impact **low** · effort **S** · architecture & layering
Files: `ARCHITECTURE.md:41` · `fartcode-core/Cargo.toml:9` · `fartcode-core/Cargo.toml:17` · `fartcode-core/src/github/client.rs:41`

ARCHITECTURE.md:41-42 declares "fartcode-core is the leaf — it depends on nothing except third-party crates", and D8 (line 2189) justifies trait placement by this "leaf rule" — but core's Cargo.toml depends on fartcode-providers (line 9, used by 5 core modules including pty/launcher and conversations) and fartcode-telemetry (line 17, documented as deliberate). Core also gained an async reqwest GitHub REST client (github/client.rs:41-68) while fartcode-integrations — the crate the topology implies for external services — is an 11-line placeholder. Future contributors resolving placement questions against §1 will make wrong calls (D8 already reasoned from the stale rule).

**Proposal:** Rewrite ARCHITECTURE.md §1: the true leaves are fartcode-providers and fartcode-telemetry; core sits above them; document the core→github/reqwest decision (models could stay in core, since pr_sync only imports PrDto, if the client is ever moved). Add a CI guard for the one rule that still matters and is checkable — `tauri` must never appear in fartcode-core's dependency tree (mirroring fartcode-telemetry's tests/no_egress.rs pattern of enforcing rather than remembering).

**Verifier caveat:** Overstated: the §1 diagram immediately above the 'leaf' sentence already draws core → fartcode-providers/fartcode-telemetry ('depends on (Phase 0 subset shown)', lines 25-33), and core's Cargo.toml documents the telemetry edge with a comment citing §1 — so the fiction is one stale, self-contradictory sentence (lines 41-42), not the section, and the 'left or below' rule as diagrammed is satisfied; D8's 'leaf rule' citation reached a placement the diagram also supports. What survives: fix the contradictory sentence, document core's reqwest GitHub client (github/client.rs) against the 11-line fartcode-integrations placeholder, and the no-tauri-in-core CI guard is a genuinely good, cheap addition. Impact low, not medium.

