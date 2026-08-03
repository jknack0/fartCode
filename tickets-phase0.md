# Phase 0 Tickets — E1 (Foundation) + E2 (Task Engine)

**Part of:** `PRD.md` v0.2 · **Scope:** Phase 0 — single local project, TUI agent path
**Reference repo:** `reference/emdash/` (clone of `generalaction/emdash`, app v1.1.40) — file paths below are relative to `apps/emdash-desktop/` unless prefixed with `packages/`.

## How to use

- Each ticket is **spawnable as-is** into an issue tracker. IDs match `PRD.md`.
- **Read [`ARCHITECTURE.md`](./ARCHITECTURE.md) first.** It defines the traits, error types, async boundaries, event bus, and code patterns every ticket assumes. A ticket's "Ref:" links are supplemental; the architecture doc is authoritative.
- **Definition of Done (merge gate)** for every ticket: `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` green, plus the ticket's acceptance criteria, plus (where noted) a restart-survival test.
- Size: S <1d · M 2–5d · L 1–2w · XL 2w+.
- "Ref:" lists the reference-implementation files to study while implementing. Read the matching `agents/risky-areas/*.md` page in the reference before touching DB, PTY, SSH, or provider-spawning code.

## Suggested build order (dependency-aware)

```
E0 workspace bootstrap → E1-01 DB/migrations → E1-02 settings+KV
  → E1-03 project model → E1-07 preserve patterns
  → E2-03 naming → E2-02 worktrees → E2-01 task model (E2-01 can start in parallel with E2-03)
  → E2-04 add-task flow → E2-05 conversation supervisor
  → E3-01..04/08 (providers — required by E2-06; see expanded tickets below) → E2-06 agent launch
  → E14-01 keybinding registry (needed by all UI tickets below; can start earlier in parallel with E1-02)
  → E2-08 conversations UI → E2-09 teardown → E2-10 nav
  → E1-04/05/06/08/09 (shell/settings/scripts/onboarding/palette) — can be parallelized
  → E2-07 persistence/resume (needs E2-05/E2-06; E13-01 tmux is optional)
E2-11 ACP path: Phase 2 (after Phase 0/1); listed here for scope visibility.
```

> **Frontend framework:** React (recommended). The reference uses React + MobX; the ecosystem (Monaco/CodeMirror, xterm.js, cmdk) is larger. Svelte is lighter but would add unnecessary risk for a port. Finalize before E1-04.

---

## E0 — Workspace bootstrap (prerequisite)

**Size:** M · **Depends on:** none · **Crate:** all (workspace root)

**Story:** Before any feature work begins, the Rust workspace must be initialized with all crates, the Tauri shell, CI, and frontend build tooling — so every other ticket has a compilation target.

**Ref:**
- PRD §4.2 crate layout (12 crates)
- Tauri 2: https://v2.tauri.app/start/create-project/
- Reference CI: `reference/emdash/.github/workflows/code-consistency-check.yml`

**Subtasks:**
- [ ] `cargo init` workspace root with `[workspace]` members: `emdash-core`, `emdash-git`, `emdash-providers`, `emdash-acp`, `emdash-terminal`, `emdash-ssh`, `emdash-scheduler`, `emdash-integrations`, `emdash-telemetry`, `emdash-server`, `emdash-runtime`, `emdash-app`.
- [ ] `emdash-app`: scaffold via `cargo tauri init` (Tauri 2); configure `tauri.conf.json` with app identifier, window defaults, security CSP, and plugin allowlist (`shell`, `process`, `updater`, `os`).
- [ ] `app-frontend/`: scaffold via `npm create vite@latest` with React + TypeScript; configure Vite for Tauri's `devUrl`/`frontendDist`; add `xterm.js`, CodeMirror 6, Tailwind CSS as dependencies.
- [ ] `Cargo.toml` root: add workspace dependencies for `tokio`, `serde`, `serde_json`, `rusqlite` (with `bundled` feature), `git2`, `portable-pty`, `russh`, `notify`, `keyring`, `croner`, `tracing`, `reqwest`, `sysinfo`, `sha2`, `base64`, `glob`, `ignore`.
- [ ] `.cargo/config.toml`: enable `rustfmt` edition 2024; configure `clippy` lints matching the reference's oxlint rules (correctness + pedantic).
- [ ] CI: GitHub Actions workflow `.github/workflows/ci.yml` — `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on push/PR; matrix macOS + Linux (Windows later). A working file already exists at `.github/workflows/ci.yml` with 3 jobs: `rust` (fmt+clippy+test), `frontend` (typecheck+lint), `db` (migration tests).
- [ ] `justfile` or `Makefile`: `dev` (tauri dev), `build`, `test`, `lint`, `fmt`, `clean` targets — one command to run the app.
- [ ] `.gitignore`: `target/`, `node_modules/`, `dist/`, `*.db`, `.env`, `.emdash/`.
- [ ] `AGENTS.md` (root): project overview, build commands, crate map, conventions (Result<T,E> everywhere, versioned JSON, provider pattern, no ad-hoc shell quoting).

**Acceptance criteria:**
- [ ] `cargo build` compiles all crates with zero errors.
- [ ] `cargo fmt --check` + `cargo clippy -- -D warnings` pass.
- [ ] `cargo test` passes (even if only placeholder tests exist).
- [ ] `make dev` (or `just dev`) launches the Tauri window with the React frontend rendering.
- [ ] CI workflow runs on push and passes.

---

## E1-01 — SQLite init + migration runner + FTS bootstrap

**Size:** L · **Depends on:** none · **Crate:** `emdash-core::db`

**Story:** The app boots into a consistent local database on first run and every upgrade, without losing data or corrupting JSON blobs.

**Ref:**
- `src/main/db/initialize.ts` (init, migration runner, FTS tables)
- `src/main/db/client.ts` (WAL, busy_timeout), `src/main/db/default-path.ts` + `src/main/db/path.ts`, `src/main/db/database-file.ts` (emdash3→4 migration)
- `src/main/db/schema.ts` (all tables), `src/main/db/versioned-column.ts`
- `drizzle/meta/_journal.json` + `drizzle/0000..0019_*.sql` (migration examples)
- `agents/risky-areas/database.md`

**Subtasks:**
- [ ] rusqlite connection singleton: `PRAGMA foreign_keys = ON`, `journal_mode = WAL`, `busy_timeout = 5000`.
- [ ] DB path resolution: `~/Library/Application Support/emdash` (macOS), `%APPDATA%/emdash` (Win), `$XDG_CONFIG_HOME/emdash` (Linux); env override `EMDASH_DB_FILE`.
- [ ] Migration runner: `migrations` table `(id INTEGER PK AUTOINCREMENT, hash TEXT, created_at)`; journal JSON + SQL files embedded at build; apply entries newer than `MAX(created_at)`; record `sha256(sql)` hash; SQL split on `--> statement-breakpoint`.
- [ ] Create all Phase-0 tables from §11 of ARCHITECTURE.md (projects, project_remotes, project_settings, app_settings, tasks, workspaces, conversations, terminals, editor_buffers, kv, automation_runs, ssh_connections, messages, automations, provider_accounts, FTS tables) with the exact columns/defaults shown. Key notes: `conversations.task_id` is NULLABLE (for project-scoped conversations in Phase 1), `conversations.scope` defaults to `'task'`, `tasks.created_by` defaults to `'user'`.
- [ ] FTS5 tables outside migrations, version-gated via `kv` keys: `search_index` (`item_type, item_id UNINDEXED, project_id UNINDEXED, task_id UNINDEXED, title, keywords`, trigram) gated by `fts_version='3'`; `workspace_file_index` + `workspace_file_index_meta` gated by `file_index_version='4'`.
- [ ] Legacy DB migration: if `emdash.db` missing but prior-version DB exists, copy via `VACUUM INTO` (readonly source), then clear secrets table in the copy.
- [ ] Versioned JSON column helper: `parse` never throws — corrupt/future/needs-context values return `None` with warning; writes always serialize latest version.

**Acceptance criteria:**
- [ ] Fresh install: schema created, FTS tables present, no errors.
- [ ] Upgrade path: applying `N` new migrations updates journal; re-running is a no-op.
- [ ] `EMDASH_DB_FILE=/tmp/foo.db` uses that file.
- [ ] Restart: same DB file reused; WAL files present; `PRAGMA foreign_keys` enforced (FK violations raise).
- [ ] A corrupt versioned-JSON cell reads as `None` and never panics.
- [ ] Migration test fixture: applying 0000..0019-equivalent SQLs in order yields expected schema; hand-edit of a numbered migration is not possible by design (embedded + hashed).

**Notes:** Do **not** hand-edit migrations after merge; add new numbered ones. Port the `kv` version gates exactly (`fts_version`, `file_index_version`) — later tickets read them.

---

## E1-02 — Settings store with layered precedence + KV

**Size:** M · **Depends on:** E1-01 · **Crate:** `emdash-core::settings`

**Story:** Users configure the app once per project; teammates share the safe subset via `.emdash.json`; local UI settings always win.

**Ref:** `src/main/core/settings/settings-registry.ts`, `settings/schema.ts`, `settings/settings-service.ts`, `settings/providers/db-project-settings-provider.ts`, `settings/effective-task-settings.ts`, `shared/core/project-settings/project-settings.ts`

**Subtasks:**
- [ ] `app_settings` KV-backed settings service: typed keys with zod-like validation (serde + a validation pass); `update()` computes **delta vs defaults** and deletes the row when delta is empty; reads deep-merge defaults.
- [ ] Registry + defaults (port exactly): `project {pushOnCreate:true, branchPrefix:'emdash', appendRandomBranchSuffix:true, tmuxByDefault:false}` · `tasks {autoGenerateName:true, autoApproveByDefault:false, autoTrustWorktrees:true, createBranchAndWorktree:true, deleteBranchByDefault:false, preserveNameCapitalization:false, includeIssueContextByDefault:true}` · `defaultAgent:'claude'` · `localProject {defaultProjectsDirectory:~/emdash/repositories, defaultWorktreeDirectory:~/emdash/worktrees, writeAgentConfigToGitIgnore:true}` · `terminal {defaultShell:'system', autoCopyOnSelection:false, macOptionIsMeta:false}` · `notifications {enabled:true, sound:true, osNotifications:true, ...}` · `browserPreview {enabled:true}` · `resourceMonitor {enabled:false}`.
- [ ] Project settings storage: two JSON blobs in `project_settings` — `base_project_settings_json` (DB-backed fields) and `shareable_project_settings_json` (`.emdash.json`-shareable fields).
- [ ] Base settings schema: `worktreeDirectory`, `defaultBranch` (string or `{name, remote:true}`), `baseRemote`, `pushRemote`, `githubAccountId`, `tmux`, `autoRunSetupScriptOnTaskCreation`, `autoRunRunScriptOnTaskCreation`, `workspaceProvider {type:'script', provisionCommand, terminateCommand}`.
- [ ] Shareable subset (`.emdash.json`): `preservePatterns`, `shellSetup`, `scripts.{setup,run,teardown}` only. `DEFAULT_PRESERVE_PATTERNS = ['.env','.env.keys','.env.local','.env.*.local','.envrc','docker-compose.override.yml']`; `.emdash.json` filtered out of patterns.
- [ ] Precedence: task `.emdash.json` (when parseable) > project settings > defaults (`effective-task-settings.ts` semantics). "Share with team" moves local values into `.emdash.json` and clears them locally.
- [ ] Seed `baseProjectSettingsJson` + shareable with defaults on project row creation **unless** repo `.emdash.json` already defines them. Defaults: `defaultBranchFallback='main'`, `baseRemote = remoteNameFromQualifiedRef(defaultBranch) ?? 'origin'`, `tmux = appSettings('project').tmuxByDefault`.

**Acceptance criteria:**
- [ ] Updating a setting to its default removes the stored row (delta check).
- [ ] `.emdash.json` in repo is honored; a local UI value overrides it; clearing local value falls back to file.
- [ ] Migration path: v1.1.15-era config with scripts/preserve patterns still works; local-only fields migrate once into base JSON.

---

## E1-03 — Project model: add local / clone GitHub / connect remote

**Size:** L · **Depends on:** E1-01, E1-02 · **Crate:** `emdash-core::projects`

**Story:** User adds a project to Emdash and gets a ready-to-task repo with a resolved default branch and git excluded for Emdash internals.

**Ref:** `src/main/core/projects/operations/createProject.ts`, `create-local-project.ts`, `create-ssh-project.ts`, `create-project-utils.ts` (base-ref resolution), `project-manager.ts`, `project-provider.ts`, `create-project-provider.ts`, `core/project-setup/repository-setup.ts`, `core/git/repository/`

**Subtasks:**
- [ ] `createProject` dispatch: `type='local'` → local flow; `type='ssh'` → SSH flow. Duplicate-path detection (existing project by path) → open existing instead.
- [ ] Local flow: validate directory; `ensureProjectRepository(git, path, initIfMissing)`; resolve base ref via `getDefaultBranch(remoteName)` + `getRefs()` with fallback to detected ref; insert `projects` row (`workspace_provider='local'`, `base_ref`); open via project manager; `ensureRepositoryWorkspace` (non-fatal); emit `project:created`.
- [ ] Clone path: `git clone` into configured dir, then same open flow. New-GitHub-repo path (E8 dependency): create via GitHub API, clone with remote creds — stub the GitHub call behind a trait in Phase 0.
- [ ] SSH flow (Phase 3 detail, stub now): `workspace_provider='ssh'`, `ssh_connection_id`, provider timeout 60s.
- [ ] Provider lifecycle: `openProject`/`closeProject` with timeouts (local 20s, SSH 60s, teardown 60s); `dispose()` teardown mode `tmux ? detach : terminate`; tears down sessions/workspaces/preview servers.
- [ ] `ensureEmdashGitExcludedSafe`: add `.emdash/` to the repo's git excludes on project open.
- [ ] Worktree pool path: local = `join(worktreeDirectory, safePathSegment(name, id))`; SSH = `join(worktreeDirectory, project.name)`.

**Acceptance criteria:**
- [ ] Add local dir → project row created, base ref resolved, `.emdash/` git-excluded, `project:created` emitted, duplicate add opens existing project.
- [ ] Add with "initialize git repository" creates a repo when absent.
- [ ] Close/open cycle restores project state (worktrees re-detected via `git worktree list`).
- [ ] Project removed only when user deletes it; provider teardown respects tmux mode.

---

## E1-04 — Sidebar tree, pinned tasks, create/delete projects

**Size:** M · **Depends on:** E1-03 · **Crate:** `app-frontend` (+ `emdash-core::projects` events)

**Story:** The left sidebar shows projects and their tasks in tree order, with pinning and quick create/delete, and drives task-switching navigation.

**Ref:** `src/renderer/features/sidebar/` (project/task items, pinned tasks, store, virtual list), `features/projects/stores/`, `features/tasks/stores/` (`task-selectors.ts`, `project-selectors.ts`)

**Subtasks:**
- [ ] Sidebar tree: projects → tasks, ordered; collapsed projects/tasks hidden from tree nav; virtualized list.
- [ ] Clicking a project name in the sidebar opens the project view (Phase 0: stub placeholder "Project chat — coming in Phase 1"; Phase 1: project-level agent chat). Clicking a task opens the task's tabs.
- [ ] Pinned tasks (`tasks.is_pinned`): pinned section pinned above others; toggle from context menu.
- [ ] Create project (⌘⇧N) + delete project with confirmation (teardown per E1-03 provider dispose).
- [ ] Task-switch ordering contract: visible tree order, skipping collapsed/hidden (E2-10 depends on this).

**Acceptance criteria:**
- [ ] Tree order matches task-switch navigation order.
- [ ] Pinning a task moves it to the pinned section and persists (`is_pinned`).
- [ ] Deleting a project tears down providers, worktrees, and rows (FK cascade verified).

---

## E1-05 — Project Settings UI

**Size:** L · **Depends on:** E1-02 · **Crate:** `app-frontend` + `emdash-core::projects/settings`

**Story:** Per-project configuration visible and editable in-app, with validation and clear override semantics.

**Ref:** `src/main/core/projects/settings/` (`worktree-directory.ts` validation, `worktree-defaults.ts`), `src/renderer/features/settings/`, `core/settings/providers/db-project-settings-provider.ts`

**Subtasks:**
- [ ] Settings panel fields: GitHub account picker (E8), worktree directory, default branch, base remote, push remote, tmux toggle, workspace provider (provision/terminate commands), preserve patterns, shell setup, lifecycle scripts, auto-run toggles.
- [ ] Worktree directory validation: must be absolute (posix `/`, win drive/UNC); `~`/`~/` expanded via home dir; invalid → `invalid-worktree-directory` error; stored invalid value falls back to default on read.
- [ ] "Share with team" dialog: writes selected shareable fields to `.emdash.json` in working dir, clears them locally.
- [ ] Defaults shown for unset fields (main, origin, `~/emdash/worktrees`, …).
- [ ] Settings changed event → provider re-reads on next task creation.

**Acceptance criteria:**
- [ ] Every field persists and re-reads correctly (DB round-trip).
- [ ] Invalid worktree directory rejected with clear error; `~` expands.
- [ ] Share-with-team writes `.emdash.json` and clears local values; committing that file gives teammates the defaults.

---

## E1-06 — Lifecycle scripts (setup/run/teardown) + env contract + drawer logs

**Size:** L · **Depends on:** E1-02, E2-02 · **Crate:** `emdash-core::terminals` + `emdash-terminal`

**Story:** New tasks run the project's setup/run scripts so the agent starts in a working environment, with output visible in the terminal drawer.

**Ref:** `src/main/core/terminals/runLifecycleScript.ts`, `terminals/lifecycle-script-coordinator.ts`, `workspaces/workspace-lifecycle-service.ts` (`terminalInputForScript`, `OUTPUT_TAIL_CAP = 16*1024`, `watchDevServer`), `workspaces/workspace-env.ts`

**Subtasks:**
- [ ] Script resolution from effective task settings (`scripts.setup|run|teardown` + `shellSetup`); no script → no-op.
- [ ] Execution: type script lines into a PTY (`\r\n → \r`), optional trailing `; exit\r` (posix) / `\rexit\r` (cmd); defaults `waitForExit=false, exit=false, respawnAfterExit=false`; `respawnAfterExit:true, logFailure:true, surfaceFailure:true, continueOnFailure:false` for manual runs.
- [ ] Status events: `running|succeeded|failed|stopped`; success = exit code 0/undefined and no signal; failure throws unless `continueOnFailure`; optional `timeoutMs`.
- [ ] PTY session id = `makePtySessionId(projectId, workspaceId, createLifecycleScriptTerminalId(type))`; dedupe active sessions.
- [ ] Env contract (exact): `EMDASH_TASK_ID`, `EMDASH_TASK_NAME` (slugified, fallback `'task'`), `EMDASH_TASK_PATH`, `EMDASH_ROOT_PATH`, `EMDASH_DEFAULT_BRANCH` (default `'main'`), **`EMDASH_PORT = 50000 + (hash32(portSeed) % 1000) * 10`** with `portSeed = workspace.path` (fallback taskId).
- [ ] Output tail capped at 16 KiB; logs surfaced in terminal drawer (⌘J); `run` scripts additionally feed dev-server detection (E6-02 hook).

**Acceptance criteria:**
- [ ] Setup+run scripts execute in task worktree with correct env; output visible in drawer.
- [ ] Port isolation: two tasks in same project get different `EMDASH_PORT`s; shell arithmetic (`$((EMDASH_PORT + 1))`) works.
- [ ] Failure of setup surfaces as `failed` status and blocks/continues per policy; timeout enforced.

---

## E1-07 — Preserve-pattern file copying into worktrees

**Size:** S · **Depends on:** E1-02, E2-02 · **Crate:** `emdash-core::projects/worktrees`

**Story:** New tasks inherit untracked-but-needed files (`.env`, compose overrides) without copying tracked files or Emdash's own config.

**Ref:** `src/main/core/projects/worktrees/worktree-service.ts` (`copyPreservedFiles` 458-510), `settings/effective-task-settings.ts`

**Subtasks:**
- [ ] Glob `preservePatterns` from effective task settings at `<worktree>/.emdash.json`.
- [ ] Skip git-tracked files (`git ls-files --error-unmatch -- <rel>`); never copy `.emdash.json` itself.
- [ ] Copy untracked matches into worktree (relative paths preserved).

**Acceptance criteria:**
- [ ] `.env`/`.env.local`/`docker-compose.override.yml` appear in new worktrees; tracked files and `.emdash.json` never copied.
- [ ] Patterns from task `.emdash.json` override project defaults.

---

## E1-08 — Onboarding + view-state persistence

**Size:** M · **Depends on:** E1-03 · **Crate:** `emdash-core::view_state` + `app-frontend`

**Story:** First-run experience guides the user from zero to their first task; window/layout state survives restarts.

**Ref:** `src/main/core/view-state/`, `src/renderer/features/onboarding/`, `src/main/index.ts` (boot: `viewStateService.pruneOrphans()`, single-instance lock), `src/main/core/app/`

**Subtasks:**
- [ ] View-state KV: persist per-view UI state (sidebar, panes, open views), prune orphans on boot.
- [ ] Onboarding steps: add project, (optional) install an agent via dependency install, (optional) GitHub/issue sign-in; skip-able, offline-OK.
- [ ] Single-instance lock; second launch focuses existing window.

**Acceptance criteria:**
- [ ] Layout restores after restart; stale view-state rows pruned.
- [ ] Onboarding completes without sign-in and lands on an empty (or first) project.
- [ ] Second launch does not open a second window.

---

## E1-09 — Command palette + FTS search + resource monitor view

**Size:** M · **Depends on:** E1-01, E1-03 · **Crate:** `emdash-core::search` + `app-frontend`

**Story:** ⌘K finds anything (projects, tasks, conversations, commands); resource monitor shows machine health.

**Ref:** `src/main/core/search/` (FTS-backed), `src/renderer/features/command-palette/`, `src/main/core/resource-monitor/`, `src/shared/resource-monitor.ts`

**Subtasks:**
- [ ] `search_index` FTS writes for projects/tasks/conversations/commands; keyword + trigram queries.
- [ ] Command palette UI (cmdk-style): command registry, navigation, actions (create task, open project, run command).
- [ ] Resource monitor: CPU/memory sampling (sysinfo), disabled by default (`resourceMonitor.enabled=false`), shown as palette panel.

**Acceptance criteria:**
- [ ] Typing in ⌘K surfaces matching projects/tasks/conversations + commands; selecting runs the action.
- [ ] Index stays current after create/rename/delete (event-driven updates).
- [ ] Resource monitor toggles on/off from settings and renders live samples.

---

## E2-01 — Task model + lifecycle state machine

**Size:** M · **Depends on:** E1-01 · **Crate:** `emdash-core::tasks`

**Story:** Tasks exist as durable rows with well-defined statuses and lifecycle events that the rest of the app (UI, telemetry, automations) can rely on.

**Ref:** `src/main/core/tasks/` (`task-service.ts`, `operations/createTask.ts`, `operations/deleteTask.ts`), `shared/core/tasks/tasks.ts` (status enum)

**Subtasks:**
- [ ] Task row + status enum: `todo | in_progress | review | done | cancelled | backlog | duplicate | triage`; initial status `taskConfig.initialStatus ?? 'in_progress'`.
- [ ] Fields: `project_id`, `name`, `linked_issue` (versioned JSON), `archived_at`, `is_pinned`, `last_interacted_at`, `status_changed_at`, `workspace_id`, `workspace_intent`, `created_by` (default `'user'`, Phase 1 adds `'agent:<id>'`), `type task|automation-run`, `automation_run_id`.
- [ ] CRUD operations: create (prepare → commit split, with workspace + initial conversation), delete, archive/restore, status change; emit `task:workspace-ready` + `taskProvisionedChannel` on provision; telemetry `task_created|provisioned|archived|status_changed|deleted`.
- [ ] Idempotent provision fast-path via task-session manager.

**Acceptance criteria:**
- [ ] Create writes task + workspace + initial conversation atomically; failure rolls back.
- [ ] Status transitions persist and update `status_changed_at`.
- [ ] Automation-run tasks carry `type='automation-run'` + `automation_run_id` (E11 hooks in later).

---

## E2-02 — Worktree manager

**Size:** L · **Depends on:** E1-03, E2-03 · **Crate:** `emdash-core::projects/worktrees` (`git2` — confirmed: git2 0.21 has full worktree support via `worktree()`, `worktrees()`, `find_worktree()`, `Worktree::prune()`. gix 0.86 does not expose worktree add/list/prune.)

**Story:** Every task gets an isolated worktree on its own branch; the manager creates, validates, reuses, and cleans them safely.

**Ref:** `src/main/core/projects/worktrees/worktree-service.ts` (constructor prune, `getWorktree`, `checkoutBranchWorktree` 265-327, `checkoutExistingBranch` 353-429, `removeWorktree` 435-439, `copyPreservedFiles` 458-510), `core/projects/worktrees/` + `worktree-directory.ts`, `shared/core/workspaces/workspace-setup-spec.ts`

**Subtasks:**
- [ ] On manager init: `git worktree prune`.
- [ ] Serialize all git ops through a queue (single worker); parse `git worktree list --porcelain`; match `branch refs/heads/<name>`.
- [ ] Worktree validity check: `.git` file exists **and** `git -C <path> rev-parse --is-inside-work-tree` == true.
- [ ] Branch-from-remote flow: `git fetch <remote>` when source is remote; create `git branch --no-track <branch> <sourceRef>`; `git worktree add <poolPath>/<branchName> <branchName>`; record `git config branch.<branchName>.base <ref>`.
- [ ] Existing-branch flow: fetch candidates `[baseRemote, origin]`; `git branch --track <branch> <remote>/<branch>`.
- [ ] Task branch name from E2-03 (prefix `emdash/`, random suffix, Linear exception).
- [ ] Remove: recursive rm + `git worktree prune`; **never** remove project root; sibling tasks block removal (`workspaceHasRemainingTasks`); `NON_INTERACTIVE_GIT_ENV` + 5s timeout on cleanup.
- [ ] Workspace rows: `kind worktree|project-root|byoi`, `location local|remote`, `type local|project-ssh|byoi`; worktree-less tasks run in project root.
- [ ] Preserve patterns integration (E1-07) runs as a setup step: `create-branch → set-branch-base → add-worktree → copy-preserved-files → push-branch` (push non-fatal).
- [ ] **Shell escaping helper module** — single shared utility (`emdash-core::shell_escape`); all worktree paths, branch names, and git refs go through it; no ad-hoc quoting anywhere in the codebase. Used by E2-02 (worktree paths), E2-06 (PTY spawn), and E4 (git commands).
- [ ] **Git strategy**: use `git2` for all worktree operations (`worktree()`, `worktrees()`, `find_worktree()`, `Worktree::prune()`). For operations git2 doesn't cover (or that are simpler via CLI), fall back to shelling out to the `git` CLI binary. `git2` is `!Sync` — all git ops must be serialized through a single-threaded queue (use `std::sync::Mutex` on a dedicated `std::thread` or `tokio::task::spawn_blocking`).
- [ ] PR-source setup (Phase 1): fetch `refs/pull/<n>/head` or add fork remote.

**Acceptance criteria:**
- [ ] New task → worktree at `~/emdash/worktrees/<project>/<branch>` (or configured dir) on its own branch, isolated.
- [ ] Disable-worktree setting runs task directly in project root with isolation warning.
- [ ] Deleting task removes worktree; deleting a task whose worktree has siblings does not remove shared workspace.
- [ ] Restart: manager re-prunes and re-detects existing worktrees without duplicate worktrees.

---

## E2-03 — Task name + branch generation

**Size:** S · **Depends on:** E1-02 · **Crate:** `emdash-core::tasks::naming`

**Story:** New tasks get human-friendly unique names and safe branch names automatically.

**Ref:** `src/main/core/tasks/name-generation/generateTaskName.ts`, `src/shared/resolveTaskBranchName.ts`, `src/main/core/tasks/stored-branch.ts`

**Subtasks:**
- [ ] No-title names: human-id style (`separator:'-', capitalize:false`). With title: slug from title.
- [ ] Sanitize to `[a-z0-9-]`, max length 64.
- [ ] Branch name: `rawBranch + '-' + suffix` when `appendRandomBranchSuffix` (default true); prefix `branchPrefix ? branchPrefix/branch : branch` (default prefix `'emdash'`); Linear-issue branch names used verbatim (no suffix).

**Acceptance criteria:**
- [ ] `autoGenerateName=true` produces `emoji-noun-verb`-style names; explicit title slugs correctly.
- [ ] Branch names match `emdash/<name>-<suffix>`; toggling `appendRandomBranchSuffix` off disables suffix.
- [ ] Names/branches are shell-safe and ≤64 chars.

---

## E2-04 — Add Task flow (branch source) + provider/model pickers

**Size:** M · **Depends on:** E2-01, E2-02, E2-03, E1-02 · **Crate:** `emdash-core::tasks` + `app-frontend`

**Story:** One dialog starts a task from a branch, with provider and model chosen, producing a provisioned worktree + seeded conversation.

**Ref:** `src/main/core/tasks/operations/createTask.ts` (`prepareCreateTask` 59-149, `commitCreateTask` 157-191), `src/renderer/features/tasks/`

**Subtasks:**
- [ ] Dialog: name (auto/manual), start source = branch (list refs) / issue (E7) / PR (E4), provider picker, model picker (specific or "Default model"), workspace target: new worktree / existing workspace / project root / BYOI (stub).
- [ ] Initial conversation config: pty type `{version:'1', type:'pty', autoApprove?, initialPrompt?, model?}`; acp type uses `initialQueue: [{text}]` (Phase 2).
- [ ] Create flow: requires open project → workspace intent → create task + workspace + initial conversation rows → emit `conversationCreated` + agent `start` event for pty initial prompt. Task `created_by` set to `'user'`.
- [ ] Trust handling: `autoTrustWorktrees` default true; forced trust when autoApprove.
- [ ] Include issue context by default when task from issue (E7-04).

**Acceptance criteria:**
- [ ] Creating a task provisions worktree, persists rows, and launches the agent (via E2-06) with the initial prompt.
- [ ] Model selector shows provider models or "Default model"; persists into conversation config.
- [ ] Cancel/validation: project-not-found and invalid inputs return typed errors, not panics.

---

## E2-05 — Conversation session supervisor (local vs SSH)

**Size:** L · **Depends on:** E2-01 · **Crate:** `emdash-core::conversations`

**Story:** Conversations are the durable handle for agent sessions; the supervisor owns session ids, resume state, and local/SSH execution.

**Ref:** `src/main/core/conversations/` (`impl/local-conversation.ts`, `impl/ssh-conversation.ts`), `conversations/resolve-agent-session-command.ts`, `conversations/set-session-id.ts`, `core/conversations/hydrate|dehydrate`, `shared/core/conversations/`

**Subtasks:**
- [ ] Conversation CRUD + hydrate/dehydrate; PTY session id = `makePtySessionId(projectId, taskId, conversationId)`.
- [ ] `scope` field: `'task'` (default) or `'project'`. Project-scoped conversations have `task_id = NULL`. Phase 0 creates task-scoped only; Phase 1 adds project-scoped with agent spawning.
- [ ] Local vs SSH execution context trait (`LocalExecutionContext` / `SSHExecutionContext`).
- [ ] Session-id persistence: `UPDATE conversations SET session_id = <trimmed>` returning row; errors `empty-session-id` / `conversation-not-found`.
- [ ] Resume resolution: providers requiring native session id on resume: `{amp, codex, commandcode, droid, goose, oh-my-pi, pi}`; use `conversation.session_id` iff set and `!= conversation.id`, else fall back to `{sessionId: conversation.id, isResuming: false}`.
- [ ] Initial-conversation flag; agent status + seen tracking on row.

**Acceptance criteria:**
- [ ] Creating a conversation yields a stable session id; resume round-trips via `session_id`.
- [ ] Resume-flag logic matches the 7-provider list + fallback exactly.
- [ ] Hydrate after restart restores conversations per task with correct resume state.

---

## E2-06 — Terminal spawn + TUI agent launch

**Size:** L · **Depends on:** E2-05, E3-01..04, E3-08 · **Crate:** `emdash-terminal` + `emdash-core::pty`

**⚠️ Platform note:** On Windows, `portable-pty` requires the ConPTY API (Windows 10 version 1809+). UTF-8 output handling differs from the reference's `node-pty`. Test with both cmd and PowerShell; ensure resize/clamp behavior matches across platforms.

**Story:** The agent CLI starts inside a PTY in the task worktree, with the right env, hooks, and prompt delivery, and respawns when appropriate.

**Ref:** `src/main/core/pty/local-pty.ts`, `pty/pty-env.ts` (`buildAgentEnv` 154-215), `conversations/impl/local-conversation.ts` (start flow 97-274), `packages/core/src/agents/agent-env.ts` (full allowlist), `core/dependencies/host-dependency-store.ts` (executable resolution), `agents/risky-areas/pty.md`

**Subtasks:**
- [ ] PTY layer (portable-pty): spawn with `name:'xterm-256color'`, defaults 80×24, cwd = worktree; resize clamps (cols≥2, rows≥1); exit normalization; kill via terminator.
- [ ] Interactive terminal env: inherit process env; force `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=emdash`; `SHELL` from profile/`$SHELL`/`/bin/zsh`(darwin)/`/bin/bash`; inject `SSH_AUTH_SOCK` via `detectSshAuthSock()` when missing.
- [ ] **Agent env allowlist** (port exactly): `TERM, COLORTERM, TERM_PROGRAM, HOME, USER, PATH, TMPDIR` + `GLOBAL_AGENT_ENV_VARS` (`EDITOR, VISUAL, GIT_EDITOR, HOSTNAME, LANG, TZ`) + `DISPLAY_ENV_VARS` + `SSH_AUTH_SOCK` + `AGENT_ENV_VARS` (~95 keys: `ANTHROPIC_*`, `CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_*`, `GEMINI_*`, `GOOGLE_*`, `OPENAI_*`, `OPENROUTER_API_KEY`, `GITHUB_TOKEN`, `GH_TOKEN`, `AWS_*`, `AZURE_OPENAI_*`, `GOOSE_*`, `QWEN_*`, `PI_*`, `CODEX_HOME`, `COPILOT_CLI_TOKEN`, proxies…); Windows essential set incl. `PATHEXT` default; hook env when hook server up: `EMDASH_HOOK_PORT`, `EMDASH_PTY_ID`, `EMDASH_HOOK_NONCE`, `EMDASH_HOOK_TOKEN`.
- [ ] Launch flow: trust check (forced when autoApprove) → ensure hooks installed → provider override settings → resolve executable (host-dependency store / cached install) → build command (`{cli, extraArgs, autoApprove?, initialPrompt?, sessionId, providerSessionId: conversation.sessionId, isResuming, model}` per provider `behavior.prompt.buildCommand`) → **spill large prompts to temp markdown file** (fresh sessions; cleaned on exit) → spawn with merged env + task env vars.
- [ ] Prompt delivery strategies per provider: argv flag / stdin / keystroke injection (type into TUI after startup) — E3-03.
- [ ] Respawn: 500 ms delay, `MAX_RESPAWNS = 2`; gated by supervisor decision (`respawnResume`); **disabled when tmux enabled**; emit `agentSessionExitedChannel` + telemetry `agent_run_started/finished` (provider + exit code).

**Acceptance criteria:**
- [ ] Agent launches in worktree PTY with correct env; interactive shell works; resize/kill sane.
- [ ] Prompt delivered via provider's strategy; large prompts spill to temp file and are cleaned up.
- [ ] Respawn behavior per policy (incl. no-respawn-under-tmux); env allowlist enforced (no stray vars leaked into agent).
- [ ] Restart-survival: TUI agent resumes where it left off (session-id resume per E2-07).

---

## E2-07 — Terminal state persistence + resume across restarts

**Size:** L · **Depends on:** E2-05, E2-06, E13-01 (tmux — **optional**; non-tmux fallback works without it) · **Crate:** `emdash-core::pty` + `emdash-terminal`

**Story:** Quit and relaunch Emdash; tasks, terminals, and agent sessions come back without losing work.

**Ref:** `src/main/core/conversations/` (`set-session-id.ts`, hydrate), `conversations/resolve-agent-session-command.ts`, `pty/tmux-session-name.ts`, `workspaces/workspace-lifecycle-service.ts`, `src/main/index.ts` boot order (rehydrate after DB init)

**Subtasks:**
- [ ] Persist `conversations.session_id` on every start/resume (E2-05).
- [ ] On boot: rehydrate conversations/terminals per task from DB; recreate PTYs; resume agents via provider resume flags (`--resume <sessionId>` etc.).
- [ ] Tmux durability path: session name = `'emdash-' + base64url(sessionId)`; create-if-missing (`tmux has-session || tmux -u new-session -d -s <name> <cmd>`), `mouse on`, `history-limit 100000`, attach; kill = `tmux kill-session -t <name>`; when tmux enabled, respawn disabled.
- [ ] Non-tmux fallback: terminal scrollback/session survives via rehydration best-effort (documented degradation).
- [ ] Remote rehydrate on SSH reconnect (Phase 3 hook; stub interface now).

**Acceptance criteria:**
- [ ] Kill-restart test: agent conversation resumes with correct history for resume-capable providers.
- [ ] With tmux on: process survives app quit; relaunch reattaches to same `emdash-*` session; manual `tmux ls | grep emdash` shows it.
- [ ] Deleting task/terminal kills its tmux session.

---

## E2-08 — Conversations UI: create, splits, context, prompt insertion

**Size:** M · **Depends on:** E2-05 · **Crate:** `app-frontend`

**Story:** Users chat with agents per task, split conversations, and add context (files, issues, prompts) before sending.

**Ref:** `src/renderer/features/conversations/` (manager, panel, sidebar list), `features/tasks/` (context bar), `core/conversations/` events

**Subtasks:**
- [ ] Conversation list per task; create (⌘T), create in right split (⌘D); close; active conversation highlight.
- [ ] Message input: initial prompt, "Add context" menu (⌘⇧A) for files/issues/prompts, add-and-send (⌘Enter).
- [ ] Prompt-library insertion (E10-01 hook) appends below existing text.
- [ ] Agent status surface: working/awaiting/done from agent hooks (E3-05), `agent_status_seen` tracking.
- [ ] Legacy `messages` table rows rendered read-only (migration path).

**Acceptance criteria:**
- [ ] ⌘T/⌘D create conversations; ⌘⇧A + ⌘Enter work as documented; shortcuts scoped to task view.
- [ ] Context pills visible before send; agent status updates live.

---

## E2-09 — Task deletion/teardown

**Size:** M · **Depends on:** E2-02, E2-06 · **Crate:** `emdash-core::tasks`

**Story:** Deleting a task (⌘Backspace) cleans up processes, sessions, worktrees, and view state — safely and idempotently.

**Ref:** `src/main/core/tasks/operations/deleteTask.ts` (order of operations, options `{deleteWorktree=true, deleteBranch=false}`), `task-lifecycle-utils.ts` (refuse project root, `deleteWorkspaceIfUnused`, `workspaceHasRemainingTasks`)

**Subtasks:**
- [ ] Order: teardown task session (terminate) → delete workspace row if unused (never `kind='project-root'`) → delete task row → delete view-state keys (`task:<id>`, `task:<id>:tabs`) → telemetry `task_deleted` → remove worktree (via project or fs fallback) → delete branch only if `deleteBranch && provisionedBranch != fromBranch.branch`.
- [ ] Guards: never remove project root; sibling tasks block workspace/worktree removal; cleanup runs `git worktree prune` with non-interactive env, 5s timeout.
- [ ] ⌘Backspace deletes selected tasks with confirmation.

**Acceptance criteria:**
- [ ] Delete removes worktree + rows + view state; process killed.
- [ ] Sibling-task scenario: worktree kept; task row removed.
- [ ] Double-delete and delete-during-provision are safe (idempotent, no panic).

---

## E2-10 — Task-switch navigation + tab scoping

**Size:** S · **Depends on:** E1-04, E2-01 · **Crate:** `app-frontend`

**Story:** ⌘⌥↑/↓ moves between tasks in sidebar order; per-task tabs behave consistently.

**Ref:** `src/renderer/features/tabs/` (pane-store, tab-bar, tab-view-factory), `features/tasks/task-tab-registry.tsx`, `src/shared/shortcuts.ts`

**Subtasks:**
- [ ] ⌘⌥↑/↓ task switch following visible tree order, skipping collapsed/hidden; scoped to task view (not while editor focused).
- [ ] Task tabs: ⌘1–9 jump, ⌘W close, ⌘\ split; Ctrl+Tab / Ctrl+Shift+Tab cycle with wrap-around, active inside terminal/editor/browser.
- [ ] Tab registry pattern for new tab kinds (conversations, diff, browser, file editor).

**Acceptance criteria:**
- [ ] Shortcuts match the PRD table; scoping rules verified (editor exemption, wrap-around).

---

## E2-11 — ACP conversation path (structured chat)

**Size:** XL · **Depends on:** E2-05, E3-01 (acp capability), E3-07 · **Phase:** 2 (scope set now) · **Crate:** `emdash-acp` + `emdash-runtime`

**Story:** Providers with ACP capability get structured, transcript-driven chat (permissions, turns, history, agent-managed terminals) instead of raw TUI.

**Ref:** `packages/core/src/acp/api/contract.ts` (procedures 66-183), `api/commands.ts` (input schema), `runtime/acp-agents/runtime/runtime.ts` (start/resume/stop), `runtime/session-manager.ts` (SessionManager surface), `runtime/session/session/cell.ts` (SessionCell), `apps/.../main/core/acp/runtime-process/host.ts` (`withSessionIdPersistence`, `withProviderEnv`), `src/main/core/acp/transport/` (local + SSH process hosts)

**Subtasks:**
- [ ] ACP client: `startSession → {sessionId}`, `resumeSession` (= start + history page, limit 50), `stopSession({conversationId})`, `sendPrompt`, `queuePrompt`, `cancelTurn`, `setModelOption`, `setModeOption`, `resolvePermission`, `getHistory`.
- [ ] Input schema: `{conversationId, projectId, taskId, providerId, workspaceId, cwd, sessionId: string|null, model, initialQueue?: [{text}], env}`; **`env` overwritten server-side from provider settings** (renderer cannot inject env).
- [ ] Out-of-process worker: ACP runtime in child process; attachments dir under app data; process hosts for local + SSH (spawn adapter binaries e.g. claude-agent-acp over stdio).
- [ ] SessionManager keyed by conversationId: `start, prompt, queuePrompt, editQueuedPrompt, removeQueuedPrompt, reorderQueue, cancel, setPromptDraft, stop, resolvePermission, setMode, setConfigOption, isRunning, getChatHistory, getHistory, getSessionState, getTerminals, killAllTerminals`; route `processKey → acpSessionId → conversationId`.
- [ ] SessionCell: state machine + transcript parser + raw-log export; live models (session state/config/usage/plan/agents/activeTurn/draft/terminals, terminalOutput liveLog).
- [ ] Session-id persistence: wrap start/resume → `setSessionId(conversationId, sessionId)`.
- [ ] Provider decision: `acp: supported` capability → ACP path; else TUI fallback (E2-06).
- [ ] Chat UI: transcript renderer + permission prompts; Phase 2.

**Acceptance criteria:**
- [ ] End-to-end ACP conversation with a fake adapter binary (test fixture): start → prompt → turn updates → stop; session id persisted and resumable.
- [ ] Provider env injection blocked from renderer.
- [ ] TUI fallback unchanged for non-ACP providers.

---

## E3-01 — Provider registry + capability descriptors

**Size:** M · **Depends on:** E0 · **Crate:** `emdash-providers` + `emdash-core`

**Story:** All 35 coding-agent CLIs are registered with their capabilities, metadata, and behavioral descriptors — the single source of truth the rest of the app queries for detection, launch, model selection, and feature gating.

**Ref:**
- `packages/plugins/src/agents/registry.ts` (registration + lookup)
- `packages/plugins/src/agents/impl/*` (per-provider definitions)
- `packages/core/src/agents/plugins/` (capability types)
- `src/shared/core/providers/` (metadata DTOs)
- `agents/integrations/providers.md`

**Subtasks:**
- [ ] `ProviderId` enum or string-keyed registry with 35 entries: `claude`, `codex`, `cursor`, `copilot`, `amp`, `commandcode`, `opencode`, `grok`, `devin`, `qwen`, `qoder`, `droid`, `antigravity`, `auggie`, `goose`, `kimi`, `kilocode`, `kiro`, `cline`, `codebuddy`, `continue`, `codebuff`, `freebuff`, `mistral`, `jules`, `junie`, `oh-my-pi`, `pi`, `letta`, `autohand`, `rovo`, `charm`, `hermes`, `mimocode`, `zero`.
- [ ] Capability flags (bitfield or struct): `acp`, `auth`, `autoApprove`, `effort`, `hooks`, `hostDependency`, `mcp`, `models`, `plugins`, `prompt`, `sessions`, `trust`. Port exactly from reference — 22 of 35 are ACP-capable.
- [ ] Per-provider metadata: display name, icon asset path, homepage URL, description, tags, default model, available models list.
- [ ] Per-provider behavior descriptor: `prompt` strategy (argv flag name, stdin support, keystroke injection), session-id format, resume flags, default args, env var prefixes.
- [ ] Provider lookup API: `get(id)`, `list()`, `filter_by_capability(flag)`, `resolve_executable(name)`.
- [ ] Renderer-facing DTO: typed JSON payload without secrets; model list includes "Default model" sentinel.

**Acceptance criteria:**
- [ ] All 35 providers registered; `list()` returns them; `get("claude")` returns correct metadata + capabilities.
- [ ] Capability filter returns correct subsets (e.g., 22 ACP-capable).
- [ ] Adding a new provider requires only one file change (registry entry + optional impl module).

---

## E3-02 — Host dependency detection + install/update/uninstall

**Size:** L · **Depends on:** E0 · **Crate:** `emdash-core::dependencies`

**Story:** Emdash detects which agent CLIs are already installed on the host, and can install, update, or remove them — so users don't need to set up agents manually.

**Ref:**
- `src/main/core/dependencies/` (`host-dependency-store.ts`, `install-runner.ts`, `dependency-managers/`)
- `packages/core/src/agents/plugins/` (dependency descriptors per provider)
- Provider-specific install docs (npm/pip/brew/curl/gh release)

**Subtasks:**
- [ ] `HostDependency` struct: `id`, `provider_id`, `name`, `install_type` (npm, pip, cargo, brew, go, curl+binary, gh-release), `install_args`, `version_command`, `min_version`, `detect_paths` (executable names to search in PATH).
- [ ] Detection: scan `PATH` for each provider's `detect_paths`; run `version_command` if found; store results in `kv` under `host-dep:<provider_id>` with TTL (re-detect on app start or explicit refresh).
- [ ] Install runner: execute install command in a PTY (so user sees output); track progress; handle failure/timeout; cache installed status.
- [ ] Update runner: compare installed version against latest (npm registry, GitHub releases, brew); update if behind.
- [ ] Uninstall: reverse of install; remove from cache.
- [ ] Dependency manager abstraction: `npm`, `pip`, `brew`, `cargo`, `go`, `curl` — each implements `detect/install/update/uninstall`.
- [ ] Onboarding integration: show "Install Claude Code" / "Install Codex" buttons in onboarding flow (E1-08).

**Acceptance criteria:**
- [ ] `claude` detected on PATH → marked installed; `codex` not on PATH → marked not-installed with install instructions.
- [ ] Install runs in a visible PTY; completion updates cache and UI.
- [ ] Detection re-runs on app start and picks up new installs.

---

## E3-03 — Prompt delivery strategies (argv, stdin, keystroke injection)

**Size:** L · **Depends on:** E3-01 · **Crate:** `emdash-core::pty` + `emdash-providers`

**Story:** Every agent gets its initial prompt in the format it expects — some take a CLI flag, some read stdin, some need it typed into their TUI after launch.

**Ref:**
- `src/main/core/pty/local-pty.ts` (spawn + prompt delivery)
- `src/main/core/conversations/impl/local-conversation.ts` (start flow, lines 97-274)
- Per-provider `behavior.prompt` in `packages/plugins/src/agents/impl/`

**Subtasks:**
- [ ] `PromptStrategy` enum: `Argv(flag_name)`, `Stdin`, `KeystrokeInjection { delay_ms, startup_indicator }`.
- [ ] Per-provider mapping: most agents use `Argv("-p")` or `Argv("--prompt")`; Amp uses `Stdin`; no-flag agents (Codex, some others) use `KeystrokeInjection`.
- [ ] Large prompt spill: if prompt exceeds a provider-specific max length (~32KB for most argv), write prompt to a temp `.md` file in the worktree, pass file path instead, clean up file on agent exit.
- [ ] Keystroke injection: after PTY spawn, wait for startup indicator (e.g., "How can I help"), then type prompt characters with inter-character delay to avoid buffer overruns.
- [ ] `buildCommand()` function: `{cli, extraArgs, autoApprove?, initialPrompt?, sessionId, providerSessionId, isResuming, model}` → `Vec<String>` ready for `std::process::Command`.

**Acceptance criteria:**
- [ ] Claude launches with `-p "prompt"`; Amp launches and receives prompt on stdin.
- [ ] A 100KB prompt spills to temp file, is passed as `-p @/tmp/emdash-prompt-xxx.md`, and file is cleaned on exit.
- [ ] Keystroke injection for a no-flag agent delivers full prompt without truncation.

---

## E3-04 — Auto-approve flag plumbing per provider

**Size:** S · **Depends on:** E3-01 · **Crate:** `emdash-core::pty`

**Story:** Providers that support auto-approve get the flag added to their CLI invocation so agents don't block on permission prompts.

**Ref:**
- `src/main/core/pty/local-pty.ts` (autoApprove integration)
- Per-provider `behavior.prompt.autoApprove` flags in `packages/plugins/src/agents/impl/`

**Subtasks:**
- [ ] Per-provider `autoApprove` capability flag; `autoApproveFlag` string (e.g., `--dangerously-skip-permissions` for Claude, `-y` for Codex).
- [ ] `buildCommand()` adds the flag when `autoApprove=true` AND provider has `autoApprove` capability.
- [ ] Forced auto-approve: when task setting `autoApproveByDefault=true`, always pass the flag.
- [ ] Trust gating: when `autoTrustWorktrees=true` (default), auto-approve is implicitly on.

**Acceptance criteria:**
- [ ] Claude launched with `--dangerously-skip-permissions` when autoApprove is on.
- [ ] Non-capable providers silently skip the flag (no error).
- [ ] Setting toggle reflected in next agent launch (not mid-session).

---

## E3-08 — PTY env allowlist + spawn platform layer

**Size:** M · **Depends on:** E3-01 · **Crate:** `emdash-terminal` + `emdash-core::pty`

**Story:** Agent processes inherit only the env vars they need — never secrets, never app internals, never user shell cruft. The allowlist is a single, security-reviewed source of truth.

**Ref:**
- `src/main/core/pty/pty-env.ts` (`buildAgentEnv`, lines 154-215)
- `packages/core/src/agents/agent-env.ts` (full allowlist — ~95 keys)
- `agents/risky-areas/pty.md`

**Subtasks:**
- [ ] Env allowlist module (`emdash-core::pty::env_allowlist`): single canonical list; any addition requires PR review.
- [ ] **Base env**: `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=emdash`, `HOME`, `USER`, `PATH`, `TMPDIR`.
- [ ] **Global agent vars**: `EDITOR`, `VISUAL`, `GIT_EDITOR`, `HOSTNAME`, `LANG`, `TZ`.
- [ ] **Display vars**: `DISPLAY`, `WAYLAND_DISPLAY` (Linux).
- [ ] **SSH agent**: `SSH_AUTH_SOCK` — inject via `detectSshAuthSock()` when present on host but missing in env.
- [ ] **Provider API keys** (~95 keys exactly per reference): `ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL`, `CLAUDE_CONFIG_DIR`, `CLAUDE_CODE_*`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `GITHUB_TOKEN`, `GH_TOKEN`, `GITLAB_TOKEN`, `AWS_*`, `AZURE_OPENAI_*`, `GOOSE_*`, `QWEN_*`, `PI_*`, `CODEX_HOME`, `COPILOT_CLI_TOKEN`, `CURSOR_API_KEY`, `MISTRAL_API_KEY`, `XAI_API_KEY`, `DEEPSEEK_API_KEY`, `GROQ_API_KEY`, `TOGETHER_API_KEY`, `FIREWORKS_API_KEY`, `REPLICATE_API_KEY`, `HUGGINGFACE_API_KEY`, `PERPLEXITY_API_KEY`, `COHERE_API_KEY`, `VOYAGE_API_KEY`, `JINA_API_KEY`, `HYPERBOLIC_API_KEY`, and HTTP proxy vars (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`, plus lowercase variants).
- [ ] **Windows essential set**: `PATHEXT`, `SystemRoot`, `USERPROFILE`, `LOCALAPPDATA`, `APPDATA`, `ALLUSERSPROFILE`, `ProgramFiles`, `ProgramFiles(x86)`, `CommonProgramFiles`.
- [ ] **Hook env** (when E3-05 hook server is running): `EMDASH_HOOK_PORT`, `EMDASH_PTY_ID`, `EMDASH_HOOK_NONCE`, `EMDASH_HOOK_TOKEN`.
- [ ] **Task env contract**: `EMDASH_TASK_ID`, `EMDASH_TASK_NAME`, `EMDASH_TASK_PATH`, `EMDASH_ROOT_PATH`, `EMDASH_DEFAULT_BRANCH`, `EMDASH_PORT` (from E1-06).
- [ ] `buildAgentEnv(provider_id, task_env, hook_env) → HashMap<String, String>`: starts with base env, merges allowlisted vars from process env, overlays task env, overlays hook env. Returns the final map. Never leaks non-allowlisted vars.
- [ ] **Security test**: a non-allowlisted env var (`SECRET_TOKEN=abc`) present in parent process is NOT present in `buildAgentEnv` output.

**Acceptance criteria:**
- [ ] Only allowlisted vars reach the agent process; test that `SECRET_TOKEN=abc` does not leak.
- [ ] Windows `PATHEXT` present on Windows, absent on macOS/Linux.
- [ ] Adding a new env var requires touching only one file (the allowlist) + PR approval.
- [ ] `SSH_AUTH_SOCK` injected when host has it but agent env doesn't.

---

## E14-01 — Keybinding registry + default map + scoping engine

**Size:** L · **Depends on:** E1-02 (settings) · **Crate:** `app-frontend` (+ `emdash-core::settings` for persistence)

**Story:** Every keyboard shortcut in the app works as documented, users can customize them in Settings, and shortcuts only fire in the right context — no accidental ⌘W closing a tab while typing in the editor.

**Ref:**
- `src/renderer/lib/commands/registry.ts` (command registry + keybinding dispatch)
- `src/shared/shortcuts.ts` (default keymap, scoping rules)
- `src/renderer/features/settings/` (shortcuts settings UI)
- `src/renderer/features/tabs/` (tab key handling)

**Subtasks:**
- [ ] **Command registry**: `Command` struct: `id`, `label`, `default_keys: Vec<KeyChord>`, `scope`, `action: Box<dyn Fn>`. Commands registered at startup by each feature module.
- [ ] **`KeyChord` struct**: `modifiers: Modifiers` (⌘/Ctrl, ⌥/Alt, ⇧, Ctrl), `key: Key` (enum: Char, Backspace, Enter, Tab, Escape, ArrowUp/Down/Left/Right, F1–F12). Parse from human-readable format (`"⌘⇧N"`, `"Ctrl+Tab"`).
- [ ] **Default keymap** (port exactly from reference, macOS-first; map Ctrl→⌘ on macOS, use Ctrl on Windows/Linux):

| Shortcut | Command | Scope |
|---|---|---|
| ⌘K | command-palette | global |
| ⌘, | open-settings | global |
| ⌘L | open-library | global |
| ⌘N | add-task | project-view |
| ⌘⇧N | add-project | global |
| ⌘Backspace | delete-task | project-view |
| ⌘O | open-project-in-editor | global |
| ⌘Enter | add-and-send-context | conversation-view |
| ⌘⇧A | add-context-menu | conversation-view |
| ⌘T | new-conversation | task-view |
| ⌘D | new-conversation-right-split | task-view |
| ⌘⇧T | new-terminal | task-view |
| ⌘⇧B | new-browser-tab | task-view |
| ⌘J | toggle-terminal-drawer | task-view |
| ⌘[/] | navigate-back/forward | global |
| ⌘B | toggle-sidebar | global |
| ⌘. | toggle-right-panel | global |
| ⌘⌥↑/⌘⌥↓ | previous-task/next-task | task-view |
| ⌘1–9 | jump-to-tab-N | task-view |
| ⌘W | close-tab | task-view |
| ⌘\ | split-pane | task-view |
| ⌘F | find-in-file | editor |
| ⌘S | save-file | editor |
| ⌘⇧S | save-all-files | editor |
| Ctrl+Tab | next-tab (wrap) | task-view |
| Ctrl+⇧+Tab | previous-tab (wrap) | task-view |
| Esc | close-modal / exit-command-palette | modal |

- [ ] **Scoping engine**: four scope levels — `global` (always active), `app-view` (when app has focus), `task-view` (when viewing a task), `conversation-view` (when a conversation input is focused), `editor` (when editor has focus), `modal` (when a modal is open). Higher-priority scopes consume events first.
- [ ] **Editor exemption**: when editor is focused, single-key and navigation shortcuts (⌘1–9, ⌘W, ⌘⌥↑/↓) are consumed by the editor component, not the global handler. Tab, Enter, Backspace, arrow keys, and printable chars always go to the focused element first.
- [ ] **Persistence**: user-customized keybindings stored in `app_settings` KV; read on startup; merge overrides onto defaults (user overrides win).
- [ ] **Hint rendering**: UI elements (buttons, context menus, toolbar items) read live bindings from the registry and display the current key chord. If a user remaps ⌘N to ⌘⇧T, the "Add Task" button shows ⌘⇧T.
- [ ] **Conflict detection**: registering a key chord already mapped to another command in the same scope logs a warning and rejects the duplicate.

**Acceptance criteria:**
- [ ] Every shortcut in the default map fires the correct command in the correct scope.
- [ ] Typing in the editor does not trigger ⌘W, ⌘1–9, or ⌘⌥↑/↓.
- [ ] Remapping ⌘N → ⌘⌥N in Settings works; "Add Task" button hint updates to ⌘⌥N.
- [ ] Duplicate keybinding in same scope is rejected with a console warning.
- [ ] On Windows/Linux, ⌘ maps to Ctrl and shortcuts work identically.
- [ ] Clear-all-custom-bindings restores the default map and removes hints.

---

## Appendix — Phase 0 cross-cutting checklists

### Restart-survival tests (required for E2-07, sanity for all)
1. Create task → quit app (kill process) → relaunch → task/conversation restored; agent resumes (or documented degradation for non-resume providers).
2. With tmux enabled: process survives quit; relaunch reattaches.
3. Editor buffers (E5-03, later phase) unaffected by Phase 0.

### Security review triggers
- PTY env allowlist changes (E2-06) — review against `packages/core/src/agents/agent-env.ts`.
- Shell quoting/escaping helpers — single shared module; no ad-hoc quoting.
- Worktree path validation (E2-02) — realpath containment, never remove project root.

### Telemetry events to emit (E15 later; stub the client now)
`app_started/app_closed (was_crash)`, `project_added/deleted`, `task_created/provisioned/status_changed/archived/deleted`, `conversation_created/deleted`, `agent_run_started/finished (provider, exit_code)`, `terminal_created/deleted`, `setting_changed`, `sidebar_toggled`, `error/$exception`.
