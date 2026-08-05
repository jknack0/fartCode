# MEMORY.md — ade

Project-level working memory. Newest entries first. If a fact here contradicts
AGENTS.md or ARCHITECTURE.md, the docs win — update this file (and the ticket if
one exists).

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
