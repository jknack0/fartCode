# MEMORY.md — fartCode

Project-level working memory. Newest entries first. If a fact here contradicts
AGENTS.md or ARCHITECTURE.md, the docs win — update this file (and the ticket if
one exists).

## E2E scenario catalogue + board fix round (2026-08-09)

`docs/e2e-scenarios.md` (e535a1a): 449 scenarios over 8 journeys, 153
deduped gaps (44 high), authored by reading the implementation not the
specs. Status vocabulary marks unreachable/not-built honestly. USE IT as
the gap backlog and the E2E test spec. Highest-severity findings not yet
ticketed: worktree pool keyed on project NAME (two same-named projects
share a pool; deleting one destroys the other's worktrees), `curl|bash`
agent install with no confirm, delete_project does no process teardown,
task.status never changes so needs-you can never render, unbounded chained
spend (no depth cap/budget on run-mode column chains). No E2E driver
exists for the Tauri app; the doc separates backend-command-drivable
scenarios from ones needing tauri-driver.

E18-07 fix round landed (69262eb) closing all 16 review findings. Notable:
step events now live in an app-lifetime store subscription (store/steps.ts)
because BoardView unmounting on dispatch was eating settle-chained
launches; re-entry PROBES FOR A LIVE AGENT before writing — the fix agent
correctly argued down my backend-guard lean, since `reattached` answers
"did the card re-enter its own column", not "is an agent running", and
TerminalManager is unreachable from &App. Frontend suite: 108 tests.

## E18-06 + E18-07 landed; board renders from config (2026-08-09)

Commits: 5628ab0 (E18-06 entry paths → is_landing + PM prompt from column
config), c340fbd (E18-07 board renders N columns — columnConfigSummary in
lib/columnConfig.ts is THE shared formatter, #67 must reuse it; new
store/columns.ts; consumes step:launch/queued/queue_cleared/settled),
e2d1de1 (PM prompt regression fix), a789600 (E18-06 review fix round),
f4116f1 (ADR amendment). #65 closed.

REVIEW FINDINGS THAT CHANGED THE DESIGN: (1) ADR-0037 item 7 now says a
landing column is NEVER an agent_step — entry paths write rows directly and
never fire on_enter, so a run-mode landing column deposits inert cards, and
routing creation through the engine would make a 50-issue import launch 50
agents. Work dispatches by MOVING onto a step, never by arrival. (2) Delete
guard ownership: the mirror owns a card whenever set; lane mapping covers
only mirrorless pre-E18 rows (was double-counting). (3) PM prompt
ticket-edit example was the exact shape parseTicketEdit rejects;
PM_PROMPT_VERSION now 3.

Still open on the board: E18-07's authority-flip half (column_id
authoritative, BLOCKED_SQL join, delete-guard switch, lift the
seeded-agent-step delete guard, In Review pin degradation) — deliberately
split out of the render round; checklist is on #66.

## v2 WIP committed at last (2026-08-09, 3adb7a1)

The design_handoff_v2 implementation had been sitting UNCOMMITTED (114
files, +16k/−4.5k) while three stash dances rode over it. Now committed as
one commit together with today's ADR-0037/0038 + design brief. `.claude/`
(5.9 GB of agent worktrees, previously untracked-but-not-ignored) added to
.gitignore — never commit it. `fartCode.zip` left untracked deliberately.
Consequence for agents: the UI wave now branches from a base that CONTAINS
the v2 board/task-view/PM-chat work — never rewrite those files from
scratch, always read first.

## E18-04/05 STEP ENGINE LANDED (2026-08-09, 5e8c017) — E18 backend COMPLETE

Squash of three worktree commits (build aa30918 + fix bf9a4a1 + final
8757ab4) cherry-picked onto main; 7-file stash dance, one conflict (app.rs:
engine's steps/Step events + WIP's host_dependencies/SettingChanged — both
kept). Combined tree: core 201 lib + suites green, app lib 42 green, tsc
clean. Migration 0007 pins In Progress advance_to → In Review (0006
untouched — LANDED MIGRATIONS ARE HASH-FROZEN, never edit). Restart
contract: parks/registry in-memory; settle re-parks queue columns after
restart (never advances through an unconfirmed gate). Ticket bookkeeping:
filing error had duplicated E18-03 (#63 dupe of #77) and never created
E18-04 — refiled as #78 (closed); corrected map on epic #60. Closed: #61
#62 #77 #78 #64. Next: #65 (E18-06 landing), then UI wave #66-#68, then E19.

## E18-01/02 LANDED on main (2026-08-09, b1ddde2)

Spike cherry-picked onto main (linear history; worktree branch commit
be04415). Landing dance: main had 102 dirty WIP files, 4 overlapped — stash
push on those 4 → cherry-pick → stash pop; one conflict (app.rs: spike's
`columns` store vs WIP's `host_dependencies` store, both kept), stash
dropped after resolve. Combined tree verified: cargo check fartcode-app +
tsc clean. E18-03 LANDED too (ade8d63, clean pop, #77 closed): BLOCKED_SQL +
dispatch blocker filter key on counts_as_done via seed_lane resolution;
BlockerRef.countsAsDone exposed to the frontend DTO. E18-04/05 step engine
building now in the same worktree — architecture: issue_enter_column
primitive (column_id always, lane synced via reverse seed_lane, unchanged
for non-seeded), on_enter queue = park + step_confirm command, settle reads
current column config (advance→enter(advance_to ?? next), hold→step-settled
event, step-done is DERIVED), reattach-never-respawn preserved, two golden
parity tests (In Progress drag + auto-flip). Adversarial review gate before
landing. REVIEW RESULT (20 agents): 14 CONFIRMED defects in aa30918, 2
refuted (acyclicity concern refuted — do not add validation). Root cause of
~half: settle is task-scoped with NO session identity — stale sessions
bypass the confirm gate (verifier repro: two settles from one session
marched a parked card into Done), walk advance chains, double-launch. Also:
seeded In Progress advance_to must be PINNED to In Review (NULL next-column
reroutes to Done if In Review deleted/reordered); parks leak on issue
delete; confirm_step check-then-act race; reattach discriminator ignores
the seed_lane fallback. Fix round dispatched: in-memory launch registry
(session-scoped settle + tombstones + restart fallback), pinned gate +
temp seeded-agent-step delete guard, park lifecycle, discriminator
alignment. Fix round bf9a4a1 mapped all 14 → tests; 3-agent
verify then found: park atomicity SOUND; two NEW blockers — (1) bf9a4a1
edited landed migration 0006 in place (sha256 startup failure on applied
DBs; pin must ship as 0007) — NEVER edit a landed migration, they are
hash-frozen; (2) restart-state confirm-gate bypass via the no-entry
heuristic on queue+advance columns (fix: heuristic refuses/re-parks on
on_enter=Queue); plus consumed-set lifetime regression breaking the E17
rework loop (fix: clear consumed per column entry). Final fix round
dispatched. Landing BLOCKED until green + my 0006-diff-empty check.

## Handoff v3 accepted — ADR-0037/0038 now BINDING (2026-08-09)

`~/Downloads/design_handoff_v3/` (README + FLOWS §5 + turn-8 frames 8a–8h)
accepted BOTH ADRs at design review; statuses flipped to accepted. Design
gate lifted from #66/#67/#68/#74/#75/#76 (label removed). DESIGN.md gained a
"Pipeline board (handoff v3)" section (step-done dot, header kind sublines,
run-mode sublines at --text-muted #9a9aa1 — existing token, sidecar
unchanged, landing tag never green, counts_as_done drives dimming,
delete-with-issues = disabled label not dialog). ERRATUM resolved by user:
v3's seed line says In Progress on_enter=queue — seed stays RUN
(behavior-identical migration wins); queue is a settings flip. Adopted from
v3: Quick seeds claude·haiku (spike updated). Dashboard placement: settings
→ project → Memory. Frames 2d/2e/4d remain archived non-spec (FLOWS §3.5).

## E18/E19 filed + design brief + schema spike (2026-08-09)

ADR-0037 → epic #60 (E18 configurable pipeline columns): #61 schema/seed,
#62 CRUD, #77 counts_as_done, #63 step engine, #64 settle, #65 landing/PM
prompt; design-gated #66/#67/#68. ADR-0038 → epic #69 (E19 feature
dossiers): #70 dossier birth, #71 skill seed, #72 FTS, #73 telemetry;
design-gated #74/#75/#76. `design-gate` label = held for frames.
`DESIGN_BRIEF_E18_E19.md` (repo root) is the designer punch list. User
explicitly overrode the design gate to start an E18-01/02 schema spike
(worktree `.claude/worktrees/agent-aae7632299c6f64d3`, UNCOMMITTED) — lane
stays authoritative, column_id mirrors. Spike passed an adversarial review
round: 6 confirmed defects fixed in place (0006 edited pre-commit, not
0007). Model change that fell out: `advance_to` target column on on_settle
(ADR-0037 items 1/4 amended — without it Quick advanced into In Progress
and double-dispatched) + `seed_lane` mapping so the delete guard derives
occupancy from the authoritative lane. Tri-state null-clear contract on
column_update (omit=keep, null=clear); step_tools fails CLOSED (corrupt →
empty allowlist, Some([]) ≠ None=unrestricted). Latent twin of the
null-clear bug exists in issues.rs UpdateIssueRequest (chip filed). All
suites green: fartcode-core 192, fartcode-app lib 20, tsc clean.

## ADR-0038 drafted: feature dossiers (2026-08-09)

`decisions/0038-feature-dossiers.md` (status: proposed, companion to 0037) —
per-feature `docs/features/<slug>.md` born with the worktree at first step
entry; app appends event-driven Timeline breadcrumbs, step prompts instruct
agents to append decision sections; convention seeded into managed repos as
`.claude/skills/feature-log/` + AGENTS.md pointer (OPT-IN, provenance-tagged
— never silently write a user's repo); sections indexed into the existing
FTS5 `search_index` as item_type "feature" for ⌘K. Moat decision settled:
repo owns the memory, app owns the intelligence (index/links/dashboard) —
app-owned storage REJECTED as it blinds outside-app agents; value telemetry
(citations, re-ask rate, tokens saved, time-to-land) computed locally in
fartcode-telemetry. Leftover questions settled 2026-08-09: consent asked at
FIRST DISPATCH (reversible via settings switch); dossier born with header
backfilled from issue/PRD/proposal; ⌘K feature hits open the CARD DETAIL
(gains a dossier section); transcript indexing deferred until citation
metrics justify it. In 0037: seeded order Backlog·Ready·Quick·In Progress·
In Review·Done; narrow mode SCROLLS, never caps. Held for DESIGN REVIEW
with 0037.

## ADR-0037 drafted: configurable pipeline columns (2026-08-09)

`decisions/0037-configurable-pipeline-columns.md` (status: proposed) —
columns become per-project data (`kind` shelf/agent_step/human_gate, per-step
prompt/model/tools, `on_enter` run/queue, `on_settle` hold/advance,
`counts_as_done`, `is_landing`); one task+worktree per card with steps as
successive sessions; classic five + a gateless "Quick" express column seeded
(express is a place, not a per-card flag; ⌘N ad-hoc stays the board-free
path; drag-skip stays legal).
Held for DESIGN REVIEW — do not start building against it until the user or
a handoff accepts it. Supersedes ADR-0032 items 2/4 if accepted.

## Rail tile click reopens flyout (2026-08-09)

User-settled interaction addition (not in the left-nav handoff, which only
specifies ⌘\\ to toggle): clicking a project rail tile now also
`setSidebarVisible(true)`, so a collapsed flyout has a mouse path back.
Auditors: not a deviation — do not revert to spec.

## v2 audit + fix round (2026-08-09, same day)

A 9-auditor fidelity audit + build gate ran after the implementation; 76
findings, all closed except the held-open design-review list below. Notable
behavioural fixes: agent-launch now waits for a green auto-run setup
(create_task defers via TerminalManager::wait_for_exit; ⌘T refuses during
setup and opens the drawer after a failed one); lifecycle scripts echo
`$`-prefixed dim command lines and append a red/dim `<type> exited <code> ·
<elapsed>` tail line; the task-header dot reads the LIVE agent terminal
(task.status never changes today — do not derive agent state from it);
`set_default_agent` command + `setting:changed` event landed (settings
Default-agent row is a real picker); the diff view dropped
@codemirror/theme-one-dark (removed from package.json) for a token theme;
board Enter routes to the task on failed cards; lane labels are sentence
case. Legacy CSS is fully dead-checked (scripted top-level-block checker vs
className usage — 0 dead blocks).

## design_handoff_v2 implemented — all 12 surfaces (2026-08-09)

`~/Downloads/design_handoff_v2/` (README + FLOWS + frames) is implemented on
top of the v1 nav. DESIGN.md is REWRITTEN to this system ("The Quiet
Terminal") and formally supersedes the 2026-08-05 emdash-world decision;
`.impeccable/design.json` regenerated to match. What landed:

- **Backend commands added** (thin over existing core): `task_archive`/
  `task_restore` (+ `task:archived`/`task:restored` events), terminal DTO
  `running`/`exitCode`, `project_settings_share`/`project_settings_provenance`
  (keys: preservePatterns|shellSetup|scripts), `host_dependency_list/install/
  update/registry_summary` (HostDependencyStore now in App state). TS
  wrappers in `lib/tauri.ts`.
- **Task view** (5a/5b/7b): `TaskHeader` (46px breadcrumb + script
  launchers + changes toggle), `tv-empty` stopped state, `Drawer.tsx` ⌘J
  bottom sheet hosting lifecycle-script terminals via `store/scripts.ts` —
  the `lifecycle-script` tab kind is GONE from the tab registry.
- **Keymap (FLOWS §3.5 settled)**: ⌘T resume-agent · ⌘⇧T new terminal ·
  ⌘J toggle-drawer · ⌘. stop-agent (SIGINT to the live agent PTY) ·
  toggle-right-panel moved to ⌘⇧. · git fetch/pull/push/publish are
  palette-only commands · archived tasks restore via ⌘K search. No ⌘1–5
  project switching. `chordFromEvent` normalizes shifted punctuation.
- **Surfaces restyled** per frames: PM chat (bubbles/proposal card, panel
  400px), Changes+commit card (single-key s/u/d/a, inline discard confirm —
  ui.discardTarget deleted), PR/checks (failed-first, accent tab underline),
  line comments (lc-* classes; .diff-sel-* kept for CardDetail), board
  (blocked-by meta, dispatch/done confirm overlays, 4a/4b card states,
  j/k/h/l + ⇧ moves), composer ⌥ options unfold, delete confirm itemizes +
  `a` archives, settings 170px nav + provenance `shared` tags + ⇧⌘S share,
  AgentsList (7d) in App settings + onboarding step two.
- **Logo**: fC mark inline SVG in the rail; full Tauri icon set generated
  from `assets/logo/fartcode-icon.svg` via `scripts/gen-icons.sh` (headless
  Chrome + embedded JetBrains Mono; rerun after mark changes).
- **CSS**: per-surface files under `src/styles/` (`taskview/changes/pr/
  comments/modals/settings` + board/project-chat), all `@import`ed from
  styles.css; ~700 lines of dead emdash-era rules deleted; xterm theme now
  reads `--xterm-*` tokens (bg #101012, emerald selection wash).
- **Known gaps (data, not design)**: no numeric task ids (frames' `#392` →
  name/uuid8), no install progress events (installing rows show no %),
  `HostDependencyDto.latest` always null (update ⌄ hidden), no branch-prefix
  command (composer shows `auto · fartCode…`), create_task takes no
  issue-link/provider params (composer issue row omitted, agent row static),
  tmux session name not itemized in the delete confirm, no merge-conflict /
  queue-ordinal / stop-attribution state on tasks (frame 4a's "conflict with
  main", "queued · 2nd of 3", "stopped by you" degrade to what the model
  holds), no would-be branch preview on a first dispatch (confirm footer
  omits the branch until the task exists).
- **Deviations held OPEN for design review (2026-08-09 — do not "fix"
  silently)**: the flyout's Recent group (v1 spec deletes it; kept so ad-hoc
  tasks stay reachable outside ⌘K — user is taking it to design); rail `+`
  = Add project not New task (same review); rail/flyout top padding 28/32px
  clears the macOS traffic lights (platform, not spec); settings renders as
  a floating card over a scrim, not the frame's full-window rail takeover;
  PM file mentions are styled spans, not links (no file surface until E5).

## Styling rules (left-nav redesign, binding for new UI work, 2026-08-08)

Superseded reference: `DESIGN.md` now carries the binding system (v2). The
v1 rules below still hold where they don't conflict.

The app follows `design_handoff_left_nav/` (README.md is the spec; frames
in `fartCode App.dc.html`). When adding/restyling UI:

- Tokens live ONLY as CSS vars in `styles.css` `:root` (`--rail-bg`,
  `--flyout-bg`, `--overlay`, `--hairline`, `--hover-bg`, `--focus-bg`,
  `--text-card`, `--meta`, `--fc-bad`, …). Never hardcode hex in components;
  never introduce a second styling system (no inline-style objects, no CSS-in-JS,
  no utility framework).
- Meaningful colour, and only these: `oklch(.78 .15 155)` = selection/additions
  (the accent); `oklch(.8 .13 80)` = an agent is working (filled) or needs you
  (hollow 1.5px ring); `--fc-bad #c96b6b` = a run ended badly; `--info #7c8fd0`
  = a link out and NOTHING else. `--meta #5f5f66` is the legibility floor —
  nothing informative goes dimmer.
- Cards/rows have no box at rest: hover paints `--hover-bg`, selection/focus is
  `--focus-bg` + a 2px accent left rail. No borders/backgrounds on idle rows.
- System sans for human text, `var(--font-mono)` for machine text (paths,
  chords, IDs, elapsed, counts). Uppercase group labels carry `letter-spacing: .14em`.
- Icons are typographic glyphs (`+`, `⌘`, `‹`, `>_`, `›`) — do NOT add an icon set.
- Motion: only the running-dot pulse (`fc-pulse` 1.8s) and the transcript caret.
  No entrance animations, no transitions on cards/columns.
- The flyout shows IN-FLIGHT work only; the board owns the rest. Do not re-add
  task trees/recents/archive lists to the nav — ⌘K is the jump surface.
- Every action needs a key first, and its button labelled with the key.

## Left-nav redesign: rail + flyout (design_handoff_left_nav, 2026-08-08)

- `components/Sidebar.tsx` is gone; `components/Nav.tsx` renders a 56px
  `LeftRail` (project letter tiles, worst-of agent dot, + new task, ⌘
  settings) plus a 244px `ProjectFlyout` fed IN-FLIGHT tasks only
  (in_progress = Running, review = Needs you). Every other task is
  reachable via ⌘K FTS — pinned/recent/archive tree sections were deleted
  per the design; pin data still drives `visibleTaskOrder` (E2-10).
- `ui.sidebarVisible` now means "flyout open"; ⌘B and ⌘\\ both toggle it
  (command `toggle-sidebar`, relabeled "Toggle project flyout").
- Design tokens live as CSS vars in styles.css `:root` (`--rail-bg`,
  `--flyout-bg`, `--hairline`, `--meta`, `--fc-bad`, …); accent is now
  oklch(.78 .15 155) with DARK `--accent-contrast`, links are `--info`
  #7c8fd0, agent-working amber oklch(.8 .13 80). Board cards are boxless
  (hover bg, selected = accent left-rail); chip row renders as mono meta.
- Skipped from the handoff (no backend surface yet): sessions view/history,
  composer overlay with `>` session switch, ⌘1–5 project switching, 1s
  elapsed tick (flyout uses a 30s tick — display is minute-coarse).

## Create-task dialog: workspace + branch pickers (#59, 2026-08-08)
- Sidebar "+" and ⌘N now open `CreateTaskDialog` (Modals.tsx, driven by
  `ui.createTaskTarget`) instead of instant-creating: workspace select
  (`new-worktree` default / `project-root`) + existing-branch picker fed by
  the new `list_project_branches` command (`BranchRef` now derives Serialize).
- `create_task` gained optional `workspace`/`branch` params; the mapping
  lives in `create_task_params` (now `pub` for tests — same pattern as
  `create_task_from_comment_core`). `project-root` ⇒ `GitSetup::None` +
  `WorkspaceTarget::ProjectRoot` (never touches the live checkout — the
  dogfood mode: agent edits hit `make dev` hot reload immediately).
  Existing branch ⇒ `GitSetup::UseBranch` in a new worktree (fetch + track).
  Comment/dispatch callers pass `None`/`None` — behavior unchanged.
- Core provision paths for both were already tested
  (tasks_operations_integration.rs); the new mapping is covered by
  fartcode-app/tests/create_task_params.rs.

## E4 PR section, PR sync, agent comment tool (#47/#49/#51, 2026-08-07)

- **GitHub client** lives in `fartcode-core/src/github` (token.rs keyring +
  `gh auth token` import; client.rs reqwest REST; models.rs DTOs). Secrets only
  in the OS keyring — never SQLite/logs. Parsers are unit-tested against
  recorded fixtures (`client::fixtures`). Rate-limit aware: 401→GithubAuth,
  403/429+remaining:0→GithubRateLimited(reset_at).
- **PR sync cache** (`pull_requests`, migration 0005): one row per PR URL,
  scalar query columns + full `PrDto` in a versioned-JSON `data` column
  (ADR-0036 — JSON sub-collections, not four normalized sub-tables). Idempotent
  upsert = deserialize-and-compare → skip write+event when byte-identical.
- **Scheduler** in `fartcode-git/src/pr_sync.rs`: periodic `run_scheduler`
  (base 60s, exp backoff on failures capped 1h, jitter), rebuilds targets from
  DB each cycle (restart-safe), `IN_FLIGHT` set dedupes concurrent syncs.
  Cursors in `kv` (`pr_sync:last:*` / `pr_sync:failures:*`). Rate-limit ends the
  cycle early (account-global). The PR tab reads the cache (instant/offline) and
  kicks a background sync; scheduler keeps it warm.
- **Commit-card PR-open guard** is now `CachedPrLookup` (reads the sync cache —
  local, offline-safe) instead of `StubPrLookup`. `PrLookup::pr_url` gained a
  `remote` param.
- **Agent comment tool** (#51): `LineCommentStore::add_agent_comment` validates
  against the task's materialized worktree (path containment, file exists,
  in-range anchor) with typed errors, attributes `created_by = agent:<provider>`.
  Exposed as `agent_add_line_comment`. **Autonomous agent invocation (MCP tool
  registration) is deferred** — no MCP custom-tool infra exists yet; see
  ADR-0035.
- Gotchas: migration count tests assert 6 now (0000–0005). `DOMAIN_TABLES`
  includes `pull_requests` + `issues`. Frontend PR tab is `store/pr.ts` +
  `PullRequestPanel.tsx`; agent comments show a `⚡ <provider>` chip via
  `commentAuthor()`.

## Wrong tab on new task — three root causes fixed (2026-08-07)

The "TTY/Setup script tab on every new task" bug was three stacked defects,
found by reading the real app DB (`~/Library/Application Support/fartCode`):
1. **Auto-run flag ignored:** `run_auto_lifecycle_scripts` never consulted
   `auto_run_enabled` — a configured `scripts.setup` (ade project: `omp`)
   spawned on EVERY task creation with the flag defaulting off. Now gated;
   regression test in tests/task_creation_agent_launch.rs.
2. **Silent failures:** fartcode-app had NO tracing subscriber — every
   best-effort launch error was dropped. `run()` now installs an
   EnvFilter (default `info`, override RUST_LOG) and agent-launch failure
   logs at `warn`.
3. **PATH fragility:** GUI/`make dev` launches can inherit a PATH without
   `~/.local/bin` (where claude lives). `find_on_path` now falls back to
   common user bin dirs (`.local/bin`, `.bun/bin`, `.cargo/bin`,
   homebrew) AFTER the real PATH — mirrors the reference
   remote-shell-profile PATH inclusion.


## Unified top chrome + one agent terminal per task (2026-08-07, ADR-0033)

- The header grid area now ALWAYS renders: `ProjectHeader` (project scope)
  or the new `TaskHeader` (task scope — project/task breadcrumb + script
  launchers + Changes toggle). TaskView's tab bars are pure tab switching;
  `.changes-toggle`/`.tab-bar-trailing`/`.tab-bar-actions` CSS deleted.
- **One agent terminal per task:** `terminal_open_agent` reattaches a live
  agent entry (`TerminalManager::find_running_agent`, lifecycle-dedupe
  pattern) before provider resolution. Frontend: tabs-store `addTab`
  focuses an existing same-id tab; `ensureTabs` surfaces uncovered live
  agent terminals as tabs (dispatch spawns before navigation, so the task
  view must show the hand-off). Switching agents = close the agent tab.
- **No tab bar unless there's something to switch:** `TaskView` renders the
  left pane's `TabBar` only when the task has 2+ tabs or a split. One agent
  terminal (the normal case) now sits directly under the header — the lone
  "TTY claude" chip was the "why is there a tab for the task?" report.
  Verified live on the running app: 1 tab → no bar, ⌘T → bar with both
  chips, close → bar gone.
- Integration test: fartcode-app/tests/agent_terminals_integration.rs.
- **Add Task (left nav) launches the default agent:** `create_task` calls
  `launch_default_agent` (best-effort, same provider resolution as
  dispatch: DEFAULT_AGENT setting → registry binary on PATH). With the
  agent installed, a fresh task opens straight on the agent tab. The
  frontend NEVER auto-spawns a plain shell on task open anymore — the old
  `ensureTabs` terminal fallback is deleted; an empty pane shows ⌘T/⌘D
  summon hints. Test: tests/task_creation_agent_launch.rs.
- **Gotcha (bit in practice):** a "still see the TTY tab" report after
  these changes = the running app is a STALE process. The Rust
  `create_task` launch needs a rebuild+restart (`make dev` / relaunch),
  and store-level frontend changes need a webview reload, not just HMR.
  Check the running pid's start time vs the binary mtime before
  re-diagnosing.


## Issue board design pass (2026-08-07)

- BoardView + CardDetail + board.css rebuilt as ONE ruled surface: a
  hairline-framed plate (`--background-1`) with five lanes divided by 1px
  `--border` rules, shared 32px head row + mono counts; cards are rows
  (title + canonical `.status-dot` + mono chips). Replaces the old
  five-floating-boxes layout. Narrow windows scroll the frame at a 750px
  floor (heads and lanes share `min-width` so they stay registered).
- Cards: linked-task dot uses the CANONICAL `.status-dot` mapping (the
  old board.css had wrong hues: done=green, review=blue — both violate
  the Dots-Are-Data rule). Provider chip is mono passive; gh provenance
  chip opens externalRef via `plugin:shell|open`; blocked chip keeps
  amber + hover popover; acceptance tally "Nac" on the title row.
- CardDetail is now an inspector: lane header with status dot (task
  status wins over lane), agent row with the ONE emerald key —
  Dispatch (backend resolves provider fallback) or Open task when
  linked — meta grid (Source/PRD/Task/Created), empty-state rows for
  acceptance/blockers, hover-only destructive remove keys, sticky
  footer delete confirm. Sheet widens to 420px via
  `.changes-panel.detail-open`.
- Toolbar gained "Add issue" (creates in Backlog, opens its detail) —
  new frontend call to `issue_create`; board empty state teaches the
  GitHub-sync key. All verified in the mocked-backend browser smoke
  (drag/move, blocked-dispatch confirm modal, dispatch → agent write →
  task navigation, gh chip URL open, dirty-save, contrast ≥5.2:1).

## Repo renamed ade → fartCode (2026-08-07)

- User rename, everywhere: 12 crates `ade-*` → `fartcode-*` (dirs + Cargo
  names + `fartcode_*` identifiers), lib crate `fartcode_app_lib`, runtime
  bin `fartcode_acp_runtime`, event channel `fartcode:event` (JS types
  `FartcodeEvent`/`onFartcodeEvent`), env contract `FARTCODE_*`
  (incl. `FARTCODE_PORT`, `fartcode_port`), config file `.fartCode.json`,
  Tauri productName `fartCode` + identifier `dev.fartcode.app`, branch
  prefix setting, all docs/decisions. Product branding is **fartCode**;
  crate/identifier spelling is lowercase `fartcode`.
- GitHub repo `jknack0/ade` → `jknack0/fartCode` (renamed; old URL
  redirects). Full gate green post-rename (fmt/clippy/test + tsc/eslint).

## E17-03 dispatch engine landed (2026-08-06, 5ecacf7) — E17 epic COMPLETE

- **Sheet layout (886bb86, user pick):** at project scope the right surface
  is ONE sheet — Changes on top, PM chat docked at the bottom (flex 42%);
  card click swaps the whole sheet to CardDetail. ⌘⇧2 shows chat AND opens
  the sheet (setChangesOpen(true) in the command); the GitHub icon toggles
  the sheet. Chat/detail mount inside ChangesSidebar, not ProjectView.

- `issue_dispatch` (fartcode-app/src/dispatch.rs): reattach if linked task lives;
  else provider = issue.provider ?? defaultAgent setting, prompt packet
  (`build_dispatch_prompt` in issues module), create_with_provision with
  `linked_issue {provider:"local", identifier:issue_id}` (NO struct change —
  the external-tracker shape absorbs the local variant), link + move.
- **Auto-flip hooks:** terminals.rs pump (agent PTY exit) and
  acp_events.rs transcript_changed (turn settles Done, once-per-turn edge
  detection via flipped_turns map). Both reach App state via
  `app.try_state::<Arc<App>>()` (needs `use tauri::Manager`). Flip = only
  in_progress → in_review.
- Frontend: in_progress drop → dispatch (unlinked) or move+focus (linked);
  agent terminal gets the packet bracket-pasted (Modals.tsx flow).
- AgentStart event is still DEAD (no consumer) — dispatch skips it; the
  frontend launches the terminal explicitly.

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
- **#58 PM chat** (dad40b5): `fartcode_core::issue_proposal` (parse — never
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
- Locked: local-first `issues`/`issue_dependencies` tables (fartCode IS the
  tracker; E7/E8 become sync adapters later); 5 lanes with drag-into-
  In-Progress spawning task+agent; board never kills (re-drag reattaches);
  blocked-by derived at read time + cycle rejection + confirm-on-dispatch;
  auto-flip to In Review on ACP turn-complete / PTY exit; chat writes via
  fenced `fartCode-proposal` block → approval card (no MCP until E10 era); PRDs =
  `docs/prds/*.md` in the repo; dispatch prompt packet by reference.
- Tickets: epic #54; #55 (E17-01 issues module) → #56 board UI / #58 PM chat
  panel → #57 dispatch engine.

## E1-06 lifecycle scripts wired into the app (2026-08-06)

- **The E1-06 runner was unwired**: settings UI + core `LifecycleScriptService`
  existed, but nothing in fartcode-app ever ran a script — "set a script, create a
  task, it just opened the terminal". Now lifecycle scripts are REAL task
  terminals: `terminal_open_lifecycle(task_id, script_type)` spawns
  `sh -c '<script>'` (shellSetup prepended) in the worktree with the FARTCODE_*
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
  tab-registry). Per-configured-script `Run <type>` keys live in the
  task-scope header row (`TaskHeader`, ADR-0033 — moved there from the
  old tab-bar trailing slot), fetched via getProjectSettings per project
  open. ⌘-free.
- **Testing:** `TerminalManager` is now `TerminalManager<R: Runtime = Wry>`
  + `tauri = { features = ["test"] }` in fartcode-app — integration tests drive
  the REAL PTY layer via `tauri::test::mock_app()` (retain/dedupe/kind,
  plain-shell drop, tail survival). Pure fns (`lifecycle_script_text`,
  `auto_run_enabled`) unit-tested in commands/lifecycle.rs. Browser smoke
  (mocked backend): button render + click-through, auto-run discovery
  (tab without spawn), dead-tab drop, double-click focus dedupe.

## Current state (2026-08-06, E2-13 task startup command)

- **Per-project `taskStartupCommand` (#52) shipped.** Project settings gain a
  BASE (non-shareable, DB-only) `taskStartupCommand` — `share_with_team`
  never writes it to `.fartCode.json`. `terminal_open` now does ONE effective
  settings read (tmux flag + startup command), and when the command is set
  spawns `sh -c '<cmd>'` INSTEAD of `$SHELL` (replace-the-shell semantics —
  terminal exits when the command exits, like agent terminals). Both paths
  covered: plain PTY (program+args already flowed) and tmux durability
  (new `build_terminal_session_command_args` in `fartcode-core::pty::tmux` —
  args were previously documented as not passed into sessions; the plain
  `build_terminal_session_command` is unchanged). Pure decision fn
  `terminal_program(&ProjectSettings, shell)` in
  `fartcode-app/src/commands/terminals.rs` (trim, blank→shell). UI: "Task startup
  command" input in ProjectSettings.tsx (placeholder `e.g. omp`), DTO field
  `taskStartupCommand`. Tests: terminal_program unit tests, tmux args
  builder round-trip through real sh (hostile quotes + $HOME), settings
  round-trip incl. not-shareable assert, and a real PTY smoke in
  fartcode-terminal (spawn `sh -c` in task cwd — macOS /private realpath trap
  on cwd compare, canonicalize). Browser-smoke verified save→reopen
  persistence. ⌘⇧O `terminal_open_agent('omp')` unchanged — explicit agent
  tab composes with the default.
- Next: **#47** E4-07 PR section (L, GitHub client) — last E4 frontier
  with #49(⇐47), #51(⇐50).

## Project-level pull (2026-08-06, left nav)

- **Sidebar project rows carry a pull action** — `project_git_pull(project_id)`
  command resolves `app.projects.get(id)` → `fartcode_git::remote::pull` (ff-only,
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
  them with every new migration. Domain: `fartcode_core::line_comments`
  (LineCommentStore CRUD + link_task both-directions in one tx +
  build_comment_prompt pinned EXACTLY to the §14 template; guard
  failures degrade, never fail state reads). Events CommentCreated/
  CommentResolved → `comment:created`/`comment:resolved` envelopes.
  Commands: add_line_comment (takes ONE `request` struct — clippy
  too_many_arguments forced it; frontend wraps `{request: args}`),
  list/resolve/delete_line_comment, create_task_from_comment (core split
  out as `create_task_from_comment_core(&App, ...)` for tests; fartcode-app
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
  `remotes.length === 0`. Backend: new `fartcode_git::remote` module —
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
  Backend: new `fartcode_git::commit` module (free fns like stage.rs — NOT
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
  13 fartcode-git tests (incl. bare-remote upstream fixture, mocked PrLookup
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
  during silent stretches. Postmortem artifacts: `fartcode-app/tests/
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
  New `fartcode-core::files::write_file` (lexical + canonical containment;
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
  `TaskCreationService` (now in App); branch = `fartCode/<slug>-<suffix>` from
  the typed `registry::PROJECT` group (**settings group key is "project"
  SINGULAR** — `get_json("projects")` throws InvalidSettingKey; typed
  `.get()` is a DbSettingsStore inherent method, the trait only has
  get_json). `provision_task` command heals legacy rows; provision's
  config-less worktree fallback mints + persists a default intent
  (regression test `provision_heals_legacy_configless_worktree_row`).
  Changes panel: not-ready state + Provision button (error match needs
  `.includes()` — frontend errors are "Error: <msg>" prefixed). Changes
  toggle moved to TabBar trailing slot (upper right; right pane's bar
  when split). Flaky: `fartcode-runtime worker_integration
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
  disk"). `fartcode-git::stage` — stage/stage_all/unstage (unborn-HEAD →
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
  from seeded view-state. Mock lessons: multiple `fartcode:event` listeners
  need handler ARRAYS; viewState must be seeded IN THE MOCK for reload
  tests (mock re-init wipes persisted state); scope assertions to the
  active `.tab-content` (hidden tabs stay mounted). Deps added:
  codemirror, @codemirror/{merge,language,language-data,state,view,
  theme-one-dark}. Next: **#45 E4-05** (inline-edit unstaged diffs ⌘S) or
  **#48 E4-08** (footer git actions) — both unblocked.

## Current state (2026-08-05, latest++++)

- **#42 E4-02 Git status/diff engine (worktree-scoped):** `fartcode-git` grew
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
  side: "staged"|"unstaged", origPath?)` in fartcode-app/src/commands/git.rs.
  22 new tests (fixture repos: conflict both-lists + :2: fallback, rename,
  spaces-in-paths, binary, oversize both paths, traversal rejection;
  synthetic parser/numstat vectors incl. rename `\0` framing). Next:
  **#43 E4-03** (Changes sidebar UI) or **#44 E4-04** (diff renderer) —
  both unblocked now.

## Current state (2026-08-05, latest+++)

- **#41 E4-01 File+git event watcher → live refresh pipeline:** E4 series
  opened (epic #40, children #41–51, milestone "Phase 1", label phase:1).
  New `fartcode-core::fs_watch`: notify-8 `FsWatchService` — one
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
  symlink trap). Lifecycle in `fartcode-app/src/watchers.rs` (indexer.rs
  pattern): boot backfill (`boot_targets`: non-archived tasks w/ local
  workspace path), TaskProvisioned → `target_for` → register,
  TaskDeleted → unregister; workspaces shared by several tasks
  refcounted. Frontend receives git:changed / files:changed via the
  established `fartcode:event` envelope (ticket's per-name Tauri events
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
  fartCode-frontend-browser-smoke (mock: /tmp/fartCode-mock-33.js pattern): full
  streaming+permission round-trip, task switch+return, cold-restart
  history restore, closed/error states. Next: E2-11 parent #21 can close;
  remaining Phase-2 work per issue list.

## Current state (2026-08-05, latest+)

- **#32 E2-11-5 Commands + conversation-store wiring + provider decision:**
  ACP conversations actually chat. `fartcode-app::acp_runtime::AcpRuntime`
  owns the SessionManager and spawns the adapter binary as a direct child
  per conversation (env server-resolved via keyring `resolve_env` with
  launcher process-env fallback; renderer never supplies env — ADR-0030:
  the E2-11-2 `fartcode-acp-runtime` worker stays DORMANT; the in-app runtime
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
  teardown) + browser smoke. Test seam: `FARTCODE_ACP_ADAPTER` env override.
  Next: **#33 E2-11-6** (transcript renderer + permission prompts).

## Current state (2026-08-05, latest)

- **#31 E2-11-4 Transcript reducer + live models:** `fartcode-acp::transcript`
  owns the full port of the reference reducer — pure
  `(ParserState, ReducerInput) → ParserState` fold (`reducer::reduce`),
  stateful `TranscriptParser` (push/settle_turn/begin_replay/end_replay/
  replay), `SessionUpdate → NormalizedEvent` decoder, reference-format id
  synthesis, and serde-camelCase live models (reduced turns w/ message/
  thinking/tool-lifecycle/plan items, config selectors, usage, title,
  agents, plan). `SessionCell` now owns parser + `RawAcpLog` (50k-entry
  in-memory raw-traffic export); raw `Turn.updates` is GONE — history is
  reduced turns, prompt text = synthetic user-message item. Event seams =
  `SessionEvents` trait fired by the cell; `fartcode-app::acp_events::
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

- **#30 E2-11-3 SessionManager + SessionCell:** `fartcode-acp::session` owns the
  runtime (cell = state machine starting→ready→working/cancelling→closed,
  prompt queue with drain-on-settle, permission broker, rev-guarded draft,
  raw update stream per turn; manager = cells keyed by conversationId,
  routes by ACP sessionId, `start` = session/load-resume w/ fallback to
  session/new + `SessionIdStore` persistence, initial-queue dispatch).
  Persistence is a one-method trait — the real `DbConversationStore`
  adapter wires at #32. Provider decision hook =
  `fartcode_core::conversations::resolve_session_path` (ACP needs config type
  AND `capabilities.acp`; else TUI path, E2-06 launcher untouched).
  Deviations from reference in cell module docs (no quiesce timer, no
  background agents — both arrive with the E2-11-4 reducer). ADR-0027.
  Tests: `fartcode-acp/tests/session_manager_integration.rs` (9 tests vs fake
  adapter incl. restart-resume) + decision regression in
  `fartcode-core/tests/conversations_integration.rs`. Next: **#31 E2-11-4**
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
- **Work tracking is GitHub issues only** (`jknack0/fartCode`) — `tickets-phase0.md`
  was retired 2026-08-04; its Appendix is preserved as `phase0-checklists.md`.
  New work = new issue (`phase:0`/`phase:2` + `size:*` labels, milestone "Phase 0").
- **Terminal-only task view (2026-08-04):** chat surfaces fully removed —
  ⌘T/⌘⇧T open plain terminals; ⌘D splits right with a fresh shell; ⌘⇧O opens
  the OMP agent terminal via new `terminal_open_agent` (provider-registry
  binary resolution through `find_on_path`). Frontend `conversation` tab
  kind, ConversationView, conversations store, palette branch, and backend
  conversation commands/indexing/search are gone; `fartcode_core::conversations`
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
  Work-inside-fartCode path for agents like omp. Next E2-11:
  **#30 E2-11-3** (SessionManager + session-id persistence).
- **HEAD (2827012, 2026-08-04):** E2-11-1 — fartcode-acp is a real ACP v1
  client: stdio JSON-RPC transport + client lifecycle (initialize/new/load/
  prompt/cancel/set_mode/set_config_option) + scoped fs handlers +
  permission surfacing. Wire types from `agent-client-protocol-schema`
  v1.6 (ADR-0024); test fixture `fartcode-acp/src/bin/fake_acp_adapter.rs`;
  8 integration tests in `fartcode-acp/tests/protocol_integration.rs`.
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
  fns return `Result<T, fartcode_core::Error>`, commands map errors to `String` and
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
- GitHub issues — the only work list (`gh issue list -R jknack0/fartCode`).
- `phase0-checklists.md` — cross-cutting Phase 0 process checklists (ex-Appendix).
- `decisions/` — ADRs 0001–0033; record new ones before merge, not after.
