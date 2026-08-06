# MEMORY.md — ade

Project-level working memory. Newest entries first. If a fact here contradicts
AGENTS.md or ARCHITECTURE.md, the docs win — update this file (and the ticket if
one exists).

## E17-02 + E17-04 landed (2026-08-06)

- **Dogfood fixes (6532b9b):** AcpRuntime::resolve_cwd hard-errored on
  project-scoped conversations ("no workspace yet") — now resolves to the
  project root (regression tests in acp_runtime.rs). Project view header:
  project name + GitHub icon (`project_github_url` command — base remote
  normalized scp/ssh/https → https; non-GitHub hides the icon) + chat
  toggle; PM panel has a minimize button (⌘⇧2 toggles back).

- **#56 board UI** (f47b3e6): 5-lane board with native HTML5 DnD →
  `issue_move` (midpoint drop index, within-lane reorder correction),
  blocked→In-Progress confirm modal, provider/linked-task badges, blocked
  hover popover, CardDetail in the project view's right region (edits via
  `issue_update`, edge add/remove, two-click delete). Card detail takes the
  right region over the PM chat via `ui.boardDetailIssueId`.
- **#58 PM chat** (dad40b5): `ade_core::issue_proposal` (parse — never
  panics; apply — all-or-nothing with compensating rollback) +
  `issue_parse_proposal`/`issue_apply_proposal` commands; frontend
  `ProposalCard` in the transcript (rename rewrites blockedBy refs; parse
  failure renders raw text); `PM_PROMPT` as hiddenContext on PM sends.
- **Seams commit 2e00b8e** (pre-landed): project-scoped conversations
  (store scope lift + `get_or_create_project_conversation`), issue command
  wrappers/events, `ProjectView` shell, owner-key conversation store
  (`project:<id>` keys), `toggle-project-chat` ⌘⇧2.
- **Mock-recipe traps hit:** Tauri listen callbacks receive
  `{event, payload}` (emit `payload` or listeners get undefined); mock
  eventHandlers must be ARRAYS fanned out (last-writer-wins silently
  un-wires earlier subscribers); programmatic `blur()` needs `focus()` first
  or React onBlur never fires.
- Remaining: #57 dispatch engine (needs both, now unblocked).

## E17 project board & PM chat — design locked (2026-08-06)

- Re-grilled the §13 project-chat design; it was **stale** (predated the #39
  terminal-only pivot and the E2-11 ACP landing). Full re-design recorded in
  ARCHITECTURE.md §13 (rewritten) + `decisions/0032-project-board-pm-chat.md`.
- Locked: local-first `issues`/`issue_dependencies` tables (ade IS the
  tracker; E7/E8 become sync adapters later); 5 lanes with drag-into-
  In-Progress spawning task+agent; board never kills (re-drag reattaches);
  blocked-by derived at read time + cycle rejection + confirm-on-dispatch;
  auto-flip to In Review on ACP turn-complete / PTY exit; chat writes via
  fenced `ade-proposal` block → approval card (no MCP until E10 era); PRDs =
  `docs/prds/*.md` in the repo; dispatch prompt packet by reference.
- Tickets: epic #54; #55 (E17-01 issues module) → #56 board UI / #58 PM chat
  panel → #57 dispatch engine.

## E1-06 lifecycle scripts wired into the app (2026-08-06)

- **The E1-06 runner was unwired**: settings UI + core `LifecycleScriptService`
  existed, but nothing in ade-app ever ran a script — "set a script, create a
  task, it just opened the terminal". Now lifecycle scripts are REAL task
  terminals: `terminal_open_lifecycle(task_id, script_type)` spawns
  `sh -c '<script>'` (shellSetup prepended) in the worktree with the ADE_*
  env contract (port seed = worktree path), via TerminalManager so output
  streams to the tab like any shell.
- **Retention:** lifecycle entries are RETAINED after exit (pump sets
  `Entry.exited` and skips the map removal only for lifecycle terminals) —
  the finished run's tab reattaches and replays the tail (64 KiB). Plain
  shells/agents keep drop-on-exit. Dedupe: `find_running_lifecycle(task,
  type)` — a rerun while one is in flight reattaches.
- **Auto-run:** `create_task` + `create_task_from_comment_core` call
  `run_auto_lifecycle_scripts` (best-effort) when
  `autoRunSetupScriptOnTaskCreation`/`autoRunRunScriptOnTaskCreation` +
  a non-blank script are set; the task view surfaces backend lifecycle
  terminals as `lifecycle-script` tabs on open (ensureTabs discovery from
  `terminal_list_for_task` kind/scriptType fields). Dead lifecycle tabs in
  persisted view-state are DROPPED on restore (never respawn as a shell).
- **UI:** TabKind `lifecycle-script` (glyph SCR, TerminalView), titles
  "Setup script"/"Run script"/"Teardown script" (`scriptTabTitle` in
  tab-registry), per-configured-script `Run <type>` buttons in the TaskView
  tab-bar trailing slot (next to the Changes toggle), fetched via
  getProjectSettings per project open. ⌘-free; the tab bar is the surface.
- **Testing:** `TerminalManager` is now `TerminalManager<R: Runtime = Wry>`
  + `tauri = { features = ["test"] }` in ade-app — integration tests drive
  the REAL PTY layer via `tauri::test::mock_app()` (retain/dedupe/kind,
  plain-shell drop, tail survival). Pure fns (`lifecycle_script_text`,
  `auto_run_enabled`) unit-tested in commands/lifecycle.rs. Browser smoke
  (mocked backend): button render + click-through, auto-run discovery
  (tab without spawn), dead-tab drop, double-click focus dedupe.

## Current state (2026-08-06, E2-13 task startup command)

- **Per-project `taskStartupCommand` (#52) shipped.** Project settings gain a
  BASE (non-shareable, DB-only) `taskStartupCommand` — `share_with_team`
  never writes it to `.ade.json`. `terminal_open` now does ONE effective
  settings read (tmux flag + startup command), and when the command is set
  spawns `sh -c '<cmd>'` INSTEAD of `$SHELL` (replace-the-shell semantics —
  terminal exits when the command exits, like agent terminals). Both paths
  covered: plain PTY (program+args already flowed) and tmux durability
  (new `build_terminal_session_command_args` in `ade-core::pty::tmux` —
  args were previously documented as not passed into sessions; the plain
  `build_terminal_session_command` is unchanged). Pure decision fn
  `terminal_program(&ProjectSettings, shell)` in
  `ade-app/src/commands/terminals.rs` (trim, blank→shell). UI: "Task startup
  command" input in ProjectSettings.tsx (placeholder `e.g. omp`), DTO field
  `taskStartupCommand`. Tests: terminal_program unit tests, tmux args
  builder round-trip through real sh (hostile quotes + $HOME), settings
  round-trip incl. not-shareable assert, and a real PTY smoke in
  ade-terminal (spawn `sh -c` in task cwd — macOS /private realpath trap
  on cwd compare, canonicalize). Browser-smoke verified save→reopen
  persistence. ⌘⇧O `terminal_open_agent('omp')` unchanged — explicit agent
  tab composes with the default.
- Next: **#47** E4-07 PR section (L, GitHub client) — last E4 frontier
  with #49(⇐47), #51(⇐50).

## Project-level pull (2026-08-06, left nav)

- **Sidebar project rows carry a pull action** — `project_git_pull(project_id)`
  command resolves `app.projects.get(id)` → `ade_git::remote::pull` (ff-only,
  same contract as the E4-08 footer) at the project ROOT checkout. Motivation:
  after a worktree branch lands on origin's default branch, the project
  checkout (often the branch the app itself runs from) had no in-app way to
  catch up. UI: hover-revealed `IconPull` button on `.project-row` (reuses
  `.add-task-btn` styling; `:disabled` = in-flight pulse), errors inline under
  the row via `.project-pull-error` (no toast system). Verified via mocked
  Tauri browser smoke (success / non-ff error / retry-clears).

## Current state (2026-08-06, E4-10 line comments)

- **E4-10 Line comments (#50) shipped — ARCHITECTURE §14 end-to-end.**
  Migration 0001_line_comments (journal when=1800000000001 + sql_for_tag
  arm; ALTERs: source_side/line_end/linked_task_id/resolved/resolved_at/
  created_by + tasks.source_comment_id). Migration-count tests
  (db_integration + migrations.rs) hardcode the journal length — bump
  them with every new migration. Domain: `ade_core::line_comments`
  (LineCommentStore CRUD + link_task both-directions in one tx +
  build_comment_prompt pinned EXACTLY to the §14 template; guard
  failures degrade, never fail state reads). Events CommentCreated/
  CommentResolved → `comment:created`/`comment:resolved` envelopes.
  Commands: add_line_comment (takes ONE `request` struct — clippy
  too_many_arguments forced it; frontend wraps `{request: args}`),
  list/resolve/delete_line_comment, create_task_from_comment (core split
  out as `create_task_from_comment_core(&App, ...)` for tests; ade-app
  lib now exposes `pub mod app; pub mod commands;`). create_task's param
  building extracted to `create_task_params` (shared with the comment
  flow, which layers an InitialConversationConfig whose initial_prompt =
  §14 template → conversations.config carries it raw, NOT
  versioned-enveloped). Worktree pool comes from app-level
  `localProject.defaultWorktreeDirectory` — per-project
  worktree_directory is NOT consulted by worktree_pool_path.
  Frontend: store/line-comments.ts (byTask, `__lineCommentsStore` seam,
  wireLineCommentEvents in App.tsx); DiffSelectionPopover FAB renamed
  "+ Comment", actions now Add Note / Create Task ⚡ / Send to agent —
  both comment paths go THROUGH the store (markLinked needs the row in
  byTask); QuickTaskDialog (ui.quickTaskTarget) prefills name/provider,
  calls create_task_from_comment then terminal_open_agent + bracketed-
  paste of the prompt; DiffView comment gutters per side (before→a,
  after→b, unified→after only) in Compartments reconfigured by a
  comments effect — markers survive rebuilds via markerMountsRef;
  CommentThread panel (resolve ✓ manual per §14, linked-task badge reads
  live status from sidebar tasksByProject, click → switchToTask).
  Browser-smoke lessons: CM6 ignores re-selecting the SAME range
  (collapse elsewhere first); syntax highlighting splits text nodes
  (select whole .cm-line via TreeWalker); `.diff-sel-actions
  button:nth-child(3)` counts the destination span — use `$$(...
  button)[i]`.
- Next: **#47** E4-07 PR section (L, GitHub client) — last E4 frontier
  with #49(⇐47), #51(⇐50 done now).

## Current state (2026-08-06, E4-08 footer git actions)

- **E4-08 Footer git actions (#48) shipped:** GitFooter under the commit
  card in the Changes sidebar — branch label + ↑ahead/↓behind badges +
  Fetch / Pull / Push / Publish, and an inline add-remote mini-form when
  `remotes.length === 0`. Backend: new `ade_git::remote` module —
  `fetch` (-q), `pull` **--ff-only** (deliberate reference deviation per
  ticket; diverged history surfaces git's stderr, never a hidden merge),
  `publish` (push -u, refuses when upstream already set — that's
  commit.rs::push's path), `add_remote` (name charset + dup + empty-url
  validation). `CommitState` extended with upstream/ahead/behind/remotes
  — ONE DTO now feeds both card and footer (git_commit_state is the
  single repo-state read). Commands: git_fetch/git_pull/git_publish/
  git_add_remote. Frontend: store actions refetch state after every
  success so the footer flips immediately (publish → push/pull
  affordances, acceptance); errors inline (.git-footer-error, role=alert,
  cleared on next success — repo has no toast system). Disabled matrix:
  fetch needs hasRemote; pull needs upstream && behind>0; push needs
  remote+branch+(upstream||published); Publish visible only when
  branch+remote && !upstream. Browser smoke: 4 scenarios by workspace id
  (synced ↑2↓1, no-remote+add-form, unpublished publish-flip, diverged
  pull error). Rust tests: bare-remote clone fixture for real
  ahead/behind + ff-pull + diverged-pull + rebase recovery.
- Next: **#47** E4-07 PR section (needs the #49 sync engine's storage —
  check its body) or **#50** E4-10 line comments; #49(⇐#46 done) and #51.

## Current state (2026-08-05, E4-06 commit card)

- **E4-06 Commit card (#46) shipped:** bottom-of-Changes-sidebar card —
  message input + Commit / Commit & Push / Commit & Create PR.
  Backend: new `ade_git::commit` module (free fns like stage.rs — NOT
  GitOps trait methods; commit/push are UI mutations, stage.rs precedent).
  `commit()` = `commit -m` + rev-parse HEAD, empty msg rejected pre-spawn;
  `push()` = upstream-configured → plain `git push`, else `push -u
  <remote> <branch>` (reference publishBranch), returns combined
  stdout+stderr (PR URLs live on stderr); `state()` = CommitState DTO
  (branch/remote/hasRemote/published/prOpen/canCreatePr) with pushRemote
  resolved workspace→task→project settings `effective_push_remote()`;
  `create_pr()` = Phase 0 stub-level integration: guard → push-if-
  unpublished → GitHub compare URL (`/compare/<branch>?expand=1`, ssh +
  https remotes) opened in the browser via @tauri-apps/plugin-shell
  (JS dep added; Rust plugin + `shell:allow-open` capability already
  registered). **PrLookup trait + StubPrLookup** = the PR-open guard seam
  (always None until E4-07/E8); guard failures degrade to "proceed",
  never fail the state read. Commands: git_commit_state/git_commit/
  git_push/git_create_pr.
  Frontend: `store/commit-state.ts` (per-workspace, `__commitStateStore`
  seam; refetch rides the changes.ts 150ms event debounce — ONE
  subscription, timer body refreshes both stores), `CommitCard.tsx` in
  ChangesSidebar (rendered only when workspaceId && snapshot). Disabled
  matrix: empty msg | nothing staged | detached HEAD disable Commit;
  push additionally needs hasRemote; PR-open → Create PR button replaced
  by "PR already open — push instead" note. Errors inline
  (.commit-error), message kept for retry, cleared on success.
  Deliberate reference deviations: no autoStage (card commits exactly
  the staged set; Stage all lives in panel header), no description
  field, explicit buttons instead of split button + remembered action.
  Browser smoke (mocked backend, per-workspace state scenarios by id):
  all 4 matrix rows + commit/push happy path + error surfacing verified.
  13 ade-git tests (incl. bare-remote upstream fixture, mocked PrLookup
  guard, offline-safe create_pr).
- Next: **#48** E4-08 footer git actions (fetch/pull/push/publish/
  add-remote — reuses commit.rs `push()` + `state()` patterns) or **#50**
  E4-10 line comments; then #47(⇐#46) → #49 PR chain.

## Current state (2026-08-05, selection → agent)

- **Terminal reattach on frontend reload (fd5956c):** vite HMR/webview
  reloads used to RESPAWN every terminal tab (fresh shell!) while the
  live agent PTY stayed orphaned in the backend — "my sessions don't
  show". ensureTabs reconcile now reattaches persisted tabs whose id is
  live in `terminal_list_for_task` (title + agent preserved), respawning
  only dead ids (app-restart path unchanged). Scrollback: TerminalManager
  keeps a 64KB output tail per entry, replayed via `terminal_tail` into a
  fresh xterm (subscribe-first buffering so the fetch race can't lose
  chunks). tmux shells benefit equally (no fresh attach ⇒ no tmux redraw
  ⇒ tail is the only content source, no duplication). Mock lesson again:
  EVERYTHING in __MOCK re-seeds on reload — flip cross-reload state via
  localStorage overrides.
- **Selection prompt routes to the LIVE AGENT TERMINAL first (68939da,
  user-directed):** opening a parallel ACP chat "while the work happens
  elsewhere" was wrong. TerminalSpec/Entry now carry `agent: Option<provider>`
  (set by terminal_open_agent; shells are None) and
  `terminal_list_for_task(taskId)` exposes it. Popover submit: agent
  terminal → `terminal_write(id, ESC[200~ + prompt + ESC[201~ + \r)`
  (bracketed paste so multi-line lands as one block) + focus that tab;
  ACP conversation is the FALLBACK (no agent terminal). The popover shows
  the destination on open ("→ omp terminal" / "→ Agent chat"). Smoke:
  both routes verified (write to term-omp with paste markers + no ACP
  call; shells-only → acp prompt + Agent tab). Provider AGENCY for the
  ACP path is still "first ACP-capable registry entry" (claude); the
  `defaultAgent` setting exists but is still unread — if provider choice
  becomes a thing, wire that + a popover picker.
- **Selection → agent WORKS end-to-end with the real adapter (verified
  live):** the "I don't see anything" report was a SLOW, SILENT turn, not
  a hang — tools-first edits (Bash/Read of a 560-line file before any
  text) leave the UI showing just the user card with no strong working
  signal for 20-60s. UX gap to close in the conversation view: an
  unmistakable working indicator (elapsed time + latest tool activity)
  during silent stretches. Postmortem artifacts: `ade-app/tests/
  acp_real_adapter_probe.rs` (ignored live probe — start → edit prompt →
  turn settle → file edited; run with --ignored) and a standalone stdio
  driver pattern (/tmp/acp-probe.mjs style: initialize/session/new/
  session/prompt over newline-delimited JSON-RPC). The claude adapter
  auto-approves fs edits without session/request_permission when the
  client declares fs read+write caps; zero CPU on the adapter does NOT
  distinguish hung-vs-fast-completed turns (node is sub-second per turn).
- **ACP adapter resolution (7a0b16e):** `default_adapter_resolver`'s
  `<id>-acp` format names binaries that don't exist in the wild —
  claude's real adapter is `claude-agent-acp` from
  `@agentclientprotocol/claude-agent-acp` (installed globally on drfart's
  machine, v0.65.0). Resolver now has a per-provider table with npm
  install hints in the error; the table's long-term home is the provider
  descriptor's adapter metadata (Phase 2 plugin machinery). Claude spawn
  sets CLAUDE_CODE_EXECUTABLE to the host binary (reference behavior,
  avoids the SDK's ~50MB auto-download). Codex's ACP is a SUBCOMMAND
  (`codex acp`, not a binary) — the path resolver can't express
  command+args yet; known limitation when codex ACP gets exercised.
- **Diff selection → agent prompt (de6c9eb, user-directed reshape of #50's
  popover):** select text in ANY diff editor (split a/b, unified, single-
  doc) → floating "Ask agent" button at selection end → popover textarea
  (Enter sends, ⇧Enter breaks, Esc closes) → `<path> lines X–Y[(baseline)]:`
  + fenced code + prompt → task's ACP conversation via shared
  `lib/acp-conversation.ts` (`ensureAcpConversation` find-or-create +
  `focusConversationTab`, extracted from the ⌘⇧A command) → conversation
  tab focused. Selection lives in diffs store (`selectionByTab`, capped
  4K chars). #50 (line comments) now inherits this popover — its
  remaining scope is Add Note / Create Task actions + persistence +
  comment-task linking, not popover mechanics. Mock lesson (recurring):
  ALWAYS close+kill the browser before re-registering an init script —
  duplicate init scripts share one scope and the second dies on
  const-redeclaration; mock is now IIFE-wrapped for idempotent
  registration. Also: no backticks in `git commit -m` double-quoted
  strings (shell ate a code span + 'syntax error at end of input').

## Current state (2026-08-05, E4-05)

- **#45 E4-05 Inline editing of unstaged diffs (c42dd17):** worktree side
  of unstaged diffs is editable (split b-editor, unified view, Added
  single-doc); staged + baselines read-only. ⌘S bound via CM keymap IN
  the editable editor (no global-registry entry — E5 keeps its own path).
  New `ade-core::files::write_file` (lexical + canonical containment;
  `Error::PathEscape`) behind `write_workspace_file` (commands/files.rs;
  `workspace_path` in commands/git.rs is now pub(crate)). Refresh rules:
  content-identical payload (save echo) skips rebuild (cursor/scroll/undo
  survive) — BUT the skip requires the view KIND to match the requested
  mode (mode flip bug found in smoke); external change rebuilds with
  scroll+selection preserved; refresh while dirty deferred (edit wins).
  Dirty dot in TabBar + header badge; saveError chip. Live CM handles in
  `lib/diff-views.ts` map (non-serializable, never in zustand);
  `window.__tabsStore` seam added to store/tabs.ts (HMR resets zustand
  stores on module reload — smoke calls into a re-created store silently
  no-op until ensureTabs rehydrates; wait ~2s after HMR). E4 is 5/11;
  next: **#48 E4-08** footer git actions or **#46 E4-06** commit card.

## Current state (2026-08-05, dogfood fix)

- **create_task never provisioned (pre-existing gap, 393abee):** the
  command used bare `DbTaskStore::create` — E2-04's `create_with_provision`
  was dead code with zero callers, so tasks got config-less `worktree` rows
  with `path=NULL` and terminals silently fell back to the project path
  (terminals.rs COALESCE). E4-03's `git_status` exposed it as "workspace
  has no local path". Fix: `create_task` routes through
  `TaskCreationService` (now in App); branch = `ade/<slug>-<suffix>` from
  the typed `registry::PROJECT` group (**settings group key is "project"
  SINGULAR** — `get_json("projects")` throws InvalidSettingKey; typed
  `.get()` is a DbSettingsStore inherent method, the trait only has
  get_json). `provision_task` command heals legacy rows; provision's
  config-less worktree fallback mints + persists a default intent
  (regression test `provision_heals_legacy_configless_worktree_row`).
  Changes panel: not-ready state + Provision button (error match needs
  `.includes()` — frontend errors are "Error: <msg>" prefixed). Changes
  toggle moved to TabBar trailing slot (upper right; right pane's bar
  when split). Flaky: `ade-runtime worker_integration
  renderer_env_discarded_and_server_env_reaches_adapter` failed once,
  passed on rerun — timing-sensitive, watch it.

## Current state (2026-08-05, latest+++++)

- **#43 E4-03 + #44 E4-04 Changes sidebar + diff renderer (one commit):**
  Right-side Changes panel (`.changes-panel`, `.shell` grid now
  `264px 1fr auto` with explicit `grid-column: 3`) toggled by sidebar-header
  branch icon or ⌘⇧1 (`toggle-changes` command; ui store `changesOpen` —
  NOT persisted, matches resourceOpen). `store/changes.ts`: snapshot per
  workspace, actions refetch immediately post-invoke, `wireChangesEvents`
  = 150 ms coalesced refetch on git:changed/files:changed for TRACKED
  workspaces only — no polling (smoke-verified flat call count). Discard
  confirm modal via ui `discardTarget` (untracked warns "deletes from
  disk"). `ade-git::stage` — stage/stage_all/unstage (unborn-HEAD →
  `git rm --cached -r` fallback)/discard (tracked→restore, untracked→fs
  delete, missing→error); commands git_stage/git_stage_all/git_unstage/
  git_discard. Row click → `openDiffTab` (lib/diff-tabs.ts): single =
  preview (one preview per pane, next preview REPLACES it), double =
  persistent; same file re-open activates (no dupe); opening preview's
  file with preview:false flips it persistent. Tab id =
  `diff:<side>:<workspaceId>:<path>` — restored tabs re-parse params from
  the id (no sidecar state); preview-ness lives in store/diffs.ts,
  restored tabs are persistent. `components/DiffView.tsx`:
  @codemirror/merge — MergeView (split) / EditorView+unifiedMergeView
  (unified), oneDark, language-data grammars, read-only; ONLY builds while
  `active` (display:none zero-measure trap), guards: binary / tooLarge /
  Added / Deleted single-doc states with badges. Mode toggle persists
  `view-state:app:diff-mode`. Browser smoke proved: panel rows/glyphs/
  rename orig→new, stage/stage-all/unstage/discard flows, event refresh,
  preview replace + persistence, unified↔split + mode persistence across
  reload, notices, diff content refresh on git:changed, restart restore
  from seeded view-state. Mock lessons: multiple `ade:event` listeners
  need handler ARRAYS; viewState must be seeded IN THE MOCK for reload
  tests (mock re-init wipes persisted state); scope assertions to the
  active `.tab-content` (hidden tabs stay mounted). Deps added:
  codemirror, @codemirror/{merge,language,language-data,state,view,
  theme-one-dark}. Next: **#45 E4-05** (inline-edit unstaged diffs ⌘S) or
  **#48 E4-08** (footer git actions) — both unblocked.

## Current state (2026-08-05, latest++++)

- **#42 E4-02 Git status/diff engine (worktree-scoped):** `ade-git` grew
  `status.rs` + `diff.rs` (crate doc already claimed status/diff — now
  true). Status: one `git --no-optional-locks status --porcelain=v2 -z
  -uall` (no-optional-locks so status never writes the index → no E4-01
  watcher feedback loop) + staged/unstaged `diff --numstat -z`;
  `StatusSnapshot { staged, unstaged, stagedAdditions/Deletions,
  truncated }` (camelCase serde, returned by commands directly); reference
  split semantics: X column → staged, Y/untracked → unstaged, conflicts
  (`u` records, AA/DD) appear in BOTH lists; renames carry `origPath`;
  untracked additions = capped line count; >10k entries → truncated=true
  w/ empty lists. Diff: NOT hunks — two-sided content payloads (`FileDiff`
  old/new content+size+exists, binary, tooLarge) because @codemirror/merge
  computes hunks from documents (reference getFileAtRef design). Sides:
  staged = HEAD:{origPath|path} ↔ :0:path; unstaged = :0: (fallback :2:
  ours during conflict, then HEAD:) ↔ worktree file. Guards: 512 KiB/side
  cap (size-checked via cat-file -s BEFORE reading — oversize blobs never
  materialize), NUL-in-8KiB binary sniff; guarded payloads keep sizes,
  drop contents. Path inputs validated lexically (no abs, no `..`).
  Commands `git_status(workspaceId)` / `git_file_diff(workspaceId, path,
  side: "staged"|"unstaged", origPath?)` in ade-app/src/commands/git.rs.
  22 new tests (fixture repos: conflict both-lists + :2: fallback, rename,
  spaces-in-paths, binary, oversize both paths, traversal rejection;
  synthetic parser/numstat vectors incl. rename `\0` framing). Next:
  **#43 E4-03** (Changes sidebar UI) or **#44 E4-04** (diff renderer) —
  both unblocked now.

## Current state (2026-08-05, latest+++)

- **#41 E4-01 File+git event watcher → live refresh pipeline:** E4 series
  opened (epic #40, children #41–51, milestone "Phase 1", label phase:1).
  New `ade-core::fs_watch`: notify-8 `FsWatchService` — one
  RecommendedWatcher, refcounted **canonical** watch roots (worktree +
  shared git common dir when it lives outside the worktree), std-thread
  dispatcher debouncing raw events into 100 ms batches → pure
  `classifier` (reference port: common-dir HEAD/refs/heads/packed-refs →
  conservative fan-out to every workspace sharing that common dir;
  superset deviation: refs/remotes + config fan out too, for ahead/behind
  freshness; per-worktree gitdir HEAD/index routed to the owning
  workspace only; worktree files excl. `.git`; objects/logs = noise) →
  bus: new `FilesChanged { workspace_id, paths (rel, ≤128) }` + existing
  `GitChanged`. `layout.rs` resolves gitdir/commondir by pure fs (no git
  binary; canonicalize everything — FSEvents reports realpaths, /tmp
  symlink trap). Lifecycle in `ade-app/src/watchers.rs` (indexer.rs
  pattern): boot backfill (`boot_targets`: non-archived tasks w/ local
  workspace path), TaskProvisioned → `target_for` → register,
  TaskDeleted → unregister; workspaces shared by several tasks
  refcounted. Frontend receives git:changed / files:changed via the
  established `ade:event` envelope (ticket's per-name Tauri events
  adjusted to the envelope convention). Service mutexes are parking_lot
  (rs-parking-lot rule; Db's std Mutex contract untouched). 19 fs_watch
  tests incl. real-FSEvents integration: burst→one batch, linked-worktree
  fan-out with index staying scoped, unregister stops events, refcounts,
  DB helper queries. Next: **#42 E4-02** (git status/diff engine).

## Current state (2026-08-05, latest++)

- **#33 E2-11-6 Chat UI — transcript renderer + permission prompts:**
  E2-11 is now 6/6. New `conversation` tab kind (tab id = conversation id;
  ⌘⇧A `open-conversation` creates/focuses with the first ACP-capable
  provider). `ConversationView` + `TranscriptItems` render the reduced
  transcript two-tier: `SettledTurn` = React.memo on (id, items.length,
  outcome.kind) — sound because committed turns are immutable — so
  streaming snapshots re-render only the active turn (verified: settled
  DOM nodes identity-stable). Permission prompts dock at the composer
  (allow*→primary / reject*→danger → `acp_resolve_permission`); transcript
  rows show a blue awaiting glyph on the gated toolCallId. Plan = docked
  strip above composer (session slice, not a transcript item). Composer:
  native textarea (editor scope — no conversation-view scope), Enter
  sends / Shift+Enter breaks, Stop→`acp_cancel`, send-while-working
  queues. States: hero, starting, closed-notice, stop-reason notices
  (max_turn_requests/max_tokens/refusal), send-error banner, conversation-
  deleted. Restore: tabs-store `reconcile` now branches per kind
  (conversation tabs restore as-is; view hydrates via `acp_history` with
  in-flight guard). tauri.ts types tightened to the exact models.rs
  discriminated unions. No Rust changes. ADR-0031. Verified per
  ade-frontend-browser-smoke (mock: /tmp/ade-mock-33.js pattern): full
  streaming+permission round-trip, task switch+return, cold-restart
  history restore, closed/error states. Next: E2-11 parent #21 can close;
  remaining Phase-2 work per issue list.

## Current state (2026-08-05, latest+)

- **#32 E2-11-5 Commands + conversation-store wiring + provider decision:**
  ACP conversations actually chat. `ade-app::acp_runtime::AcpRuntime`
  owns the SessionManager and spawns the adapter binary as a direct child
  per conversation (env server-resolved via keyring `resolve_env` with
  launcher process-env fallback; renderer never supplies env — ADR-0030:
  the E2-11-2 `ade-acp-runtime` worker stays DORMANT; the in-app runtime
  won, keeping all E2-11-4 wiring live). Commands: `create_conversation`
  (runtime type decided SERVER-SIDE from capabilities.acp — renderer never
  picks it), `list_conversations` (DTO carries derived `runtime` field,
  no DB column), `acp_start` / `acp_send_prompt` / `acp_cancel` /
  `acp_resolve_permission` / `acp_stop` / `acp_history`. Provider decision
  gate = `resolve_session_path` in exactly 2 places (create + start).
  Teardown: `delete_task` calls `AcpRuntime::stop_task` BEFORE the FK
  cascade. Frontend: `store/conversations.ts` (runtime field + `acp:*`
  subscription + `window.__conversationsStore` browser-test seam), ⌘Enter
  `send-context` command routes only when runtime==='acp' (TUI untouched).
  Boot ACP rehydration NOT wired (PTY stays byte-identical; follow-up with
  #33 chat UI). Tests: 3 E2E (fake adapter e2e, gate non-regression,
  teardown) + browser smoke. Test seam: `ADE_ACP_ADAPTER` env override.
  Next: **#33 E2-11-6** (transcript renderer + permission prompts).

## Current state (2026-08-05, latest)

- **#31 E2-11-4 Transcript reducer + live models:** `ade-acp::transcript`
  owns the full port of the reference reducer — pure
  `(ParserState, ReducerInput) → ParserState` fold (`reducer::reduce`),
  stateful `TranscriptParser` (push/settle_turn/begin_replay/end_replay/
  replay), `SessionUpdate → NormalizedEvent` decoder, reference-format id
  synthesis, and serde-camelCase live models (reduced turns w/ message/
  thinking/tool-lifecycle/plan items, config selectors, usage, title,
  agents, plan). `SessionCell` now owns parser + `RawAcpLog` (50k-entry
  in-memory raw-traffic export); raw `Turn.updates` is GONE — history is
  reduced turns, prompt text = synthetic user-message item. Event seams =
  `SessionEvents` trait fired by the cell; `ade-app::acp_events::
  TauriAcpEvents` emits `acp:update` / `acp:transcript` (full LiveModels
  snapshot) / `acp:permission_request` keyed by conversationId —
  bypassing the internal bus (terminal:output precedent). ADR-0029.
  Scoped down: no EnrichHook → no subagent/search/mcp/web-fetch event
  kinds; terminal live models stay empty until the Phase-4 `terminal`
  capability. `StartInput` gained `provider_id` + `events` (replaces
  `update_sink`). Fake adapter has a `rich` prompt behavior exercising
  every slice. Tests: 6 reducer goldens + 5 browser-free event/integration
  tests. Next: **#32 E2-11-5** (commands + conversation-store wiring).

## Current state (2026-08-05, later)

- **UI redesign — Signal → "emdash world" (impeccable new-work, seed e3c1a90f):**
  full replacement of `app-frontend`'s visual world. Neutral charcoal chassis
  (`#111111` bg ramp from emdash `.emdark`), emerald primary action
  (`--accent: #00a67b`), blue selection; status hues: in_progress = amber
  `--status-in-progress`, in_review = green `--status-in-review`,
  cancelled/destructive = red. Type: Inter Variable (UI voice) + JetBrains
  Mono Variable (machine voice) via `@fontsource-variable/*`. Old
  `--bg*`/`--amber` Signal tokens are gone — don't reintroduce;
  `styles.css` `:root` is the token source (`--background*`/`--foreground*`/
  `--border*`/`--accent*`/`--status-*`/`--xterm-*`), recorded in DESIGN.md
  + `.impeccable/design.json`. Icons are drawn SVG in `components/icons.tsx`
  (no unicode glyphs). xterm theme in `lib/terminals.ts` syncs with
  `--xterm-*`. Direction contract comment lives in `index.html` body
  (survives build). Supersedes the intermediate INSTRUMENT concept
  (seed 0a35d91b, Barlow fonts) — never landed. Reviewer disposition: ship.

## Current state (2026-08-05)

- **Terminal lifecycle fix (ADR-0028):** reopen now shows every surviving
  tmux terminal automatically, and closing a tab KILLS the session (no more
  detach-survivors accumulating — a real task had grown to slots 0–10).
  Mechanics: `close` runs `kill-session` + frees the slot; `pick_slot` →
  `choose_terminal_slot` reuses the smallest live DETACHED session (never
  double-attaches); window close = `detach_all` (PTYs die, sessions live);
  restore calls new `terminal_surviving` and opens extra tabs for survivors
  beyond the persisted tabs. Real-tmux integration test
  `list_by_prefix_reports_survivors_with_attach_state` pins the listing.

- **#30 E2-11-3 SessionManager + SessionCell:** `ade-acp::session` owns the
  runtime (cell = state machine starting→ready→working/cancelling→closed,
  prompt queue with drain-on-settle, permission broker, rev-guarded draft,
  raw update stream per turn; manager = cells keyed by conversationId,
  routes by ACP sessionId, `start` = session/load-resume w/ fallback to
  session/new + `SessionIdStore` persistence, initial-queue dispatch).
  Persistence is a one-method trait — the real `DbConversationStore`
  adapter wires at #32. Provider decision hook =
  `ade_core::conversations::resolve_session_path` (ACP needs config type
  AND `capabilities.acp`; else TUI path, E2-06 launcher untouched).
  Deviations from reference in cell module docs (no quiesce timer, no
  background agents — both arrive with the E2-11-4 reducer). ADR-0027.
  Tests: `ade-acp/tests/session_manager_integration.rs` (9 tests vs fake
  adapter incl. restart-resume) + decision regression in
  `ade-core/tests/conversations_integration.rs`. Next: **#31 E2-11-4**
  (transcript reducer + live models + `acp:*` events).

## Current state (2026-08-04)

- **#36 durable terminals (ADR-0025):** with the project `tmux` setting on,
  E2-12 terminals run under tmux (`{project}:{task}:terminal:{slot}` sessions)
  — app crash/restart leaves the shell alive and the next open REATTACHES
  slot 0. Close-tab = detach; task-delete sweeps the prefix (orphans included).
  tmux binary resolved with Dock-PATH fallback; setting off/binary absent →
  plain shell unchanged. `tmux_by_default` stays false. Agent terminals
  (⌘⇧O) are always plain PTYs — slot durability is for shell terminals.
- **"Signal" UI design system (#38):** full restyle of `app-frontend`. Tokens
  live in `styles.css` `:root` (`--bg0..3`, `--line`, `--text/--muted/--faint`,
  `--amber` reserved for the ONE active signal: selected task row bar, focused
  pane's active tab, primary actions). Type: Space Grotesk = UI voice,
  JetBrains Mono = data voice (tasks, chords, terminals, meters) — bundled via
  `@fontsource/*` (imported in `main.tsx`; no CDN, Tauri stays offline-safe).
  Tab kinds carry a `glyph` in `lib/tab-registry.tsx` (terminal = `$`).
  xterm theme re-tinted in `lib/terminals.ts` (bg `#0b0d10`, cursor amber).
  Old `--navy*` AND #39's signal-box `--board/--ivory/--aspect-*` tokens are
  gone — don't reintroduce.
- **Work tracking is GitHub issues only** (`jknack0/ade`) — `tickets-phase0.md`
  was retired 2026-08-04; its Appendix is preserved as `phase0-checklists.md`.
  New work = new issue (`phase:0`/`phase:2` + `size:*` labels, milestone "Phase 0").
- **Terminal-only task view (2026-08-04):** chat surfaces fully removed —
  ⌘T/⌘⇧T open plain terminals; ⌘D splits right with a fresh shell; ⌘⇧O opens
  the OMP agent terminal via new `terminal_open_agent` (provider-registry
  binary resolution through `find_on_path`). Frontend `conversation` tab
  kind, ConversationView, conversations store, palette branch, and backend
  conversation commands/indexing/search are gone; `ade_core::conversations`
  stays (PTY launcher + boot rehydration depend on it). Scope precedence is
  now modal > editor > task-view > project-view > app-view > global.
- **Terminal lifecycle (#37) kept under the terminal-only refactor:** xterm
  sessions live outside React keyed by PTY id (`lib/terminals.ts`); PTY
  ownership is in the tab store (only ⌘W's last reference / split collapse /
  task delete kills); terminal tabs persist and respawn a fresh shell on
  restore (scrollback restart survival = future tmux work). Panes ALSO keep
  all tabs mounted (hidden, not unmounted) so tab flips never even detach.
- **Signal-box theme:** dark green-grey diagram board, ivory track lines,
  multi-aspect state colors (proceed/caution/stop/shunt), Libre Franklin +
  IBM Plex Mono (@fontsource). Terminal theme matches --inset/--ivory/
  --aspect-proceed.
- **Phase 0 is fully closed** (2026-08-04). **Phase 2 in progress:** E2-11
  broken into #28–#33; #28 (2827012), #34 (9041aad), #29 (2ca862a) done.
  **#35 E2-12 interactive task terminal done (713dfbd) + terminal-first
  default (5ea481d) + lifecycle fix (#37) + terminal-only refactor.**
  Work-inside-ade path for agents like omp. Next E2-11:
  **#30 E2-11-3** (SessionManager + session-id persistence).
- **HEAD (2827012, 2026-08-04):** E2-11-1 — ade-acp is a real ACP v1
  client: stdio JSON-RPC transport + client lifecycle (initialize/new/load/
  prompt/cancel/set_mode/set_config_option) + scoped fs handlers +
  permission surfacing. Wire types from `agent-client-protocol-schema`
  v1.6 (ADR-0024); test fixture `ade-acp/src/bin/fake_acp_adapter.rs`;
  8 integration tests in `ade-acp/tests/protocol_integration.rs`.
- **E14-01 (16b8e8f):** keybinding registry — scope precedence
  modal > editor > task-view > project-view > app-view > global
  (conversation-view scope removed with the chat surfaces),
  user overrides in `view-state:app:keybindings`. E2-10's
  `lib/shortcuts.ts` was superseded and deleted.
- **E2-08 removed the standalone conversation list** — conversations now live
  under tasks (create-task command + sidebar).
- **E2-07 shipped terminal persistence/resume** — boot rehydration orchestration,
  tmux kill, remote hook, dirty-check on worktree open (ADR-0022 for the
  sync-command decision).

## Key decisions (see decisions/ for full ADRs, 0001–0027)

- **Git strategy:** `git2` v0.21 for worktree lifecycle (add/list/prune); shell
  out to `git` CLI for everything else. `gix` rejected (no worktree ops as of 0.86).
- **git2::Repository is `!Sync`** — all git operations go through the serialized
  GitOps impl (ADR-0003).
- **ACP wire types** come from the official `agent-client-protocol-schema`
  crate; transport/client/SessionManager are ours (ADR-0024, PRD §10.1
  resolved). Workspace `rust-version` = 1.88 because of it.
- **keyring v3 needs a backend feature** (`apple-native` on macOS,
  `sync-secret-service` on Linux) — without one it silently uses a mock
  store and secrets vanish across calls. Secrets never cross a Tauri
  command boundary (maskedSecret DTOs only).
- **SQLite migrations are append-only** — never hand-edit an applied migration
  (ADR-0001).
- **Settings** use layered precedence + KV store (ADR-0002).
- **Tauri commands are thin and synchronous** where possible (ADR-0022); domain
  fns return `Result<T, ade_core::Error>`, commands map errors to `String` and
  return DTOs.
- **Terminal persistence** via tmux; resume across restarts (ADR-0021).
- **Task deletion teardown** semantics in ADR-0023.

## Conventions that bite

- `cargo` lives at `~/.cargo/bin` (rustup) — export PATH before cargo commands.
- Frontend UI verification: drive `vite` dev in a headless browser with a
  mocked Tauri backend (`window.__TAURI_INTERNALS__`, seeded via
  `evaluateOnNewDocument`) — the frontend has no test runner; restart
  survival is checked by re-seeding persisted view-state and reloading.
- `make frontend` is **required before `cargo build`** — the app embeds
  `app-frontend/dist`.
- Icons are **placeholder-generated** (amber bar on navy) — fine for dev, must be
  regenerated before first bundling (E16).
- No ad-hoc shell quoting — use the shared `shell_escape` module.
- Worktree paths validated by realpath containment; never delete the project root.
- Versioned JSON (`read_versioned`/`write_versioned`) for all JSON DB columns.
- Tests use `tempfile` / `:memory:` — never touch real app data paths.
- **Terminal session lifecycle:** React effect cleanups run on task switches
  and tab flips — never kill/cleanup shared resources there. Interactive
  terminals keep their xterm instance in a module-level registry keyed by PTY
  id; the TAB owns the PTY (tab store kills on close/split-collapse/delete),
  the VIEW only attaches/detaches the DOM node. Also: one PTY drives one
  xterm surface — splitting a terminal spawns a fresh shell. (E2-12 fix #37.)
- **PTY integration tests: never gate readiness on echoed output** — the PTY
  echoes the typed command, so a sentinel inside the command self-matches
  before it runs (tmux_durability flake). Gate on files the shell writes.
- Before touching DB, PTY, SSH, or provider-spawning code, read the matching
  `reference/emdash/agents/risky-areas/*.md` page (reference impl is a clone of
  `generalaction/emdash`, Electron + TS).

## Docs map

- `AGENTS.md` — onboarding + merge gate (fmt, clippy -D warnings, cargo test).
- `ARCHITECTURE.md` — authoritative reference: traits, error type, async
  boundaries, event bus, DB schema. Ticket contradicting it → ticket loses.
- `PRD.md` — product spec + epic inventory.
- GitHub issues — the only work list (`gh issue list -R jknack0/ade`).
- `phase0-checklists.md` — cross-cutting Phase 0 process checklists (ex-Appendix).
- `decisions/` — ADRs 0001–0023; record new ones before merge, not after.
