# fartCode end-to-end scenario catalogue

449 scenarios across eight journey areas, written against the code as it stands on `main` at
f942288 plus the uncommitted v3 working tree — not against the docs. Where the code and
`DESIGN.md` / the ADRs / `design_handoff_v2` / `design_handoff_v3` / `FLOWS.md` disagree, the code
wins and the delta is recorded as a finding.

---

## How to use this document

This catalogue has two jobs, and every scenario serves both.

**1 · It is the backlog for end-to-end tests.** Each scenario is a falsifiable Given/When/Then
with an `Automation` line naming the seam that would drive it. Scenarios marked `implemented` are
the regression net worth building first; the rest are executable specifications for work that does
not exist yet.

**2 · It is a UI-flow gap survey.** A scenario is written for the intended journey, then checked
against the code. Where the two diverge the scenario says so in the `Then` — `**Intended:** …
**Actual:** …` — and the divergence is raised as a numbered finding in the
[Gap register](#gap-register). Nothing was dropped to make the picture look better; several
scenarios exist only to document that a designed affordance has no code behind it, or that
working code has no affordance in front of it.

### Scenario format

```
#### <AREA>-<NN> — <one-line title>
- **Given:**   the precondition, concrete enough to build a fixture from
- **When:**    the single user gesture (or system event) under test
- **Then:**    the observable outcome. Where intent and reality diverge, both are stated.
- **Covers:**  the ADR / handoff frame / PRD line the scenario exists to defend
- **Automation:** the seam that could drive it — RTL, a Rust command test, a static assertion,
                  or the honest admission "needs a driver we lack"
- **Status:**  one of the four values below
```

### Status vocabulary

| Status | Meaning |
|---|---|
| `implemented` | The `Then` happens today. Parenthetical notes such as *(as designed)*, *(untestable here)* or *(unspecified by design)* do not change the bucket. |
| `partial` | Some of the `Then` happens; the rest deviates from the design, is under-communicated, silently fails, or is missing its confirmation step. |
| `not-built` | The intended behaviour does not exist. |
| `unreachable` | The capability exists in code — a command, a store action, a branch, a DTO field — but no user gesture reaches it. |

Section authors used a few compound tokens. They are preserved verbatim in the scenario bodies and
normalised as follows for every count in this document, so the tallies stay auditable:

- `unreachable-entirely`, `unreachable-by-mouse`, `unreachable-by-mouse-only` → **unreachable**
- any compound token whose second half records a deviation, a missing gate, or an unbuilt half —
  `implemented-but-wrong`, `implemented-contradicting-spec`, `implemented without a confirm`,
  `implemented without error surfacing`, `implemented (the kill), gap (the missing confirm)`,
  `implemented (PR tab) / partial (board import silence)`, `partial / misplaced`,
  `partial / unreachable-by-mouse-only` → **partial**
- parentheticals that qualify only the *test*, not the behaviour — `implemented (untestable here)`,
  `implemented (as designed)`, `implemented (unspecified by design)`, `implemented — but …` — stay
  **implemented**; the "but" clause is carried as a finding in the register instead
- `implemented-as-specified, but the spec has no failure branch` (CROSS-26) → **implemented**;
  the missing failure branch is GAP-42
- `not-built (index leak) / unreachable-by-mouse` (LIFE-42) → **not-built** (its primary claim)

### Two facts that shape every `Automation` line

1. **There is no UI-level E2E driver in this repo.** `make test` is `vitest run` +
   `cargo test --workspace`. No Playwright, no `tauri-driver`, no WebDriver. Anything that needs
   real layout, a real window, xterm, CodeMirror, drag physics or a text selection is marked
   *needs a driver we lack*. See [Recommended test harness](#recommended-test-harness).
2. **`FARTCODE_DB_FILE` points `App::init` at a temp DB**, so the backend half of most scenarios
   is drivable today from `cargo test` with event-bus assertions, even when the gesture half is not.

---

## Coverage map

| # | Journey area | Scenarios | implemented | partial | not-built | unreachable | Findings raised |
|---|---|---:|---:|---:|---:|---:|---:|
| 1 | First run, projects, and workspace setup | 67 | 39 | 13 | 8 | 7 | 27 |
| 2 | Board and pipeline columns | 48 | 26 | 13 | 8 | 1 | 20 |
| 3 | PM chat, PRDs, and issue proposals | 53 | 30 | 18 | 3 | 2 | 22 |
| 4 | Task view, agent sessions, and terminals | 50 | 32 | 13 | 3 | 2 | 18 |
| 5 | Changes, commit, PR, and checks | 65 | 41 | 9 | 13 | 2 | 25 |
| 6 | Task end states: delete, archive, restore, teardown | 49 | 24 | 14 | 10 | 1 | 22 |
| 7 | Navigation, search, keyboard, and layout | 63 | 40 | 10 | 10 | 3 | 32 |
| 8 | Cross-cutting: persistence, failure, concurrency, consent | 54 | 35 | 7 | 10 | 2 | 21 |
| | **Total** | **449** | **267** | **97** | **65** | **20** | **187 raw** |

59% of the catalogue describes behaviour that exists. The remaining 41% splits into 97 flows that
work but mislead, under-report or skip a gate; 65 that were designed and never built; and 20 where
working code sits behind no affordance at all.

The 187 raw findings deduplicate to **153 distinct gaps** — 44 high, 67 medium, 42 low — because
independent authors reached the same defect from different surfaces. Every merge is recorded in the
register's *Source* column; nothing was dropped and no severity was lowered.

---

## 1 · First run, projects, and workspace setup

Everything between launching fartCode with an empty database and having a project that can actually run an agent: the onboarding modal (`components/Onboarding.tsx`, view-state gated per ADR-0017), adding a project (`Modals.tsx::CreateProjectDialog` → `create_project` → `DbProjectStore::create_local`), the agent-CLI detection/install list (`AgentsList.tsx` + `store/dependencies.ts` + `commands/dependencies.rs`), the per-project settings pane and its share-to-`.fartCode.json` provenance (`ProjectSettings.tsx`, `settings/service.rs`), the seeded six-column board that arrives with every new project (ADR-0037 item 8, `issues/columns.rs::SEED_COLUMNS`), the ⌘J lifecycle-script drawer (`Drawer.tsx` + `commands/lifecycle.rs`), and deleting a project (`Nav.tsx` right-click → `ConfirmDelete` → `DbProjectStore::delete`).

Scenarios below are written against the code as it stands on `main` at f942288 plus the uncommitted v3 working tree. Two structural facts shape almost every "Automation" line: there is **no UI-level E2E driver in this repo** (no Playwright, no `tauri-driver`, no WebDriver — `make test` is `vitest run` + `cargo test --workspace`), and `FARTCODE_DB_FILE` lets a Rust integration test point `App::init` at a temp DB, so backend halves are drivable while the gesture halves mostly are not.

---

### A · First launch and the onboarding modal

#### FIRST-01 — Show the welcome step on a virgin database
- **Given:** a fresh `FARTCODE_DB_FILE` with no `view-state:app:onboarding` kv row and no projects.
- **When:** the app window finishes loading.
- **Then:** a `.modal-backdrop` with a `.fc-onboard` card appears reading "Welcome to fartCode" over the copy "fartCode runs coding agents in isolated worktrees. Three quick steps — all optional, all skippable.", with two actions: `skip` and `↵ get started`. Behind it the main region shows the `fartCode` brand placeholder and "Add a project to get started — press ⌘⇧N or the + button."
- **Covers:** ADR-0017 §3; FLOWS.md F1; design_handoff_v2 7d card grammar.
- **Automation:** RTL component test — mock `getViewState` → `null`, render `<Onboarding />`, assert the title text.
- **Status:** implemented

#### FIRST-02 — Advance the welcome step with ↵
- **Given:** onboarding open on the welcome step, no text input focused.
- **When:** press `Enter`.
- **Then:** the card swaps to "Add a project" with a focused `/path/to/repo` input and a `browse…` button; `↵ add project` is disabled while the input is empty.
- **Covers:** ADR-0017 §3 (skip-able stepped flow).
- **Automation:** RTL — `userEvent.keyboard("{Enter}")` on the document, assert step-two title and the disabled button.
- **Status:** implemented

#### FIRST-03 — Add the first project from inside onboarding
- **Given:** onboarding on the "Add a project" step; `/tmp/e2e-repo` is a real git repo.
- **When:** type `/tmp/e2e-repo` into the input and press `Enter`.
- **Then:** the card advances to "Agents on this machine"; the rail behind the modal now shows a tile with the repo's first letter, and the tile is `.active`.
- **Covers:** PRD E1-03/E1-08; FLOWS.md F1 step 1 (local folder).
- **Automation:** RTL for the step transition (mock `createProject`); the real create is a Rust test on `create_local`.
- **Status:** implemented

#### FIRST-04 — Surface a bad path without losing the step
- **Given:** onboarding on the "Add a project" step.
- **When:** type `/does/not/exist` and press `Enter`.
- **Then:** a `.fc-set-error` paragraph appears inside the card carrying the backend string (`project path not found: /does/not/exist`); the step stays on "Add a project" and the input keeps its text.
- **Covers:** E1-03 validation (`create_local` canonicalize failure).
- **Automation:** RTL with `createProject` rejecting; assert the error node.
- **Status:** implemented

#### FIRST-05 — Refuse a non-git directory, with no way to initialize one
- **Given:** `/tmp/plain-dir` exists and is not a git repository.
- **When:** enter that path in onboarding (or in the ⌘⇧N dialog) and confirm.
- **Then:** the inline error reads `not a git repository: /tmp/plain-dir`. **Intended:** the dialog should offer "initialize a repository here" (the backend already takes `init_if_missing`).
- **Covers:** `ProjectStore::create_local(path, init_if_missing)`; FLOWS.md F2 "Pick/Clone/New".
- **Automation:** Rust test on `create_local(dir, false)` → `Err`; the missing affordance is a UI gap, not testable.
- **Status:** partial (error path implemented; the `init_if_missing: true` branch has no caller — `commands/projects.rs:21` hardcodes `false`)

#### FIRST-06 — Show the agent-detection list as onboarding step two
- **Given:** onboarding advanced to the agent step.
- **When:** the step renders.
- **Then:** a "Detected" group lists one row per registered agent CLI — installed rows read `<version> · <bin dir>` (home-anchored paths tildified), missing rows read `not found · install` — and a tail line reads `+ N more in the registry · M acp`. A forced re-detect runs on every mount (`load(true)`), so an agent installed outside the app between visits shows up.
- **Covers:** design_handoff_v2 7d; ADR-0011; FLOWS.md F1 gap "no surface anywhere for agent CLI detected/missing → install".
- **Automation:** RTL over `<AgentsList />` with a mocked `hostDependencyList`; plus a Rust test asserting `host_dependency_registry_summary()` totals.
- **Status:** implemented

#### FIRST-07 — The GitHub step is inert
- **Given:** onboarding advanced to the "Connect GitHub?" step.
- **When:** press `↵ done` (or `skip`).
- **Then:** both buttons do exactly the same thing — write `{done:true}` to `view-state:app:onboarding` and close. No sign-in is offered, nothing is stored, no GitHub token flow starts.
- **Covers:** ADR-0017 §3 ("GitHub sign-in is a Phase-0 stub").
- **Automation:** RTL — assert `setViewState` called with the key and `{done:true}` from both buttons.
- **Status:** partial (step exists, does nothing; `github_token_*` commands exist but are not wired here)

#### FIRST-08 — Onboarding shows exactly once across restarts
- **Given:** onboarding was completed or skipped.
- **When:** quit and relaunch the app.
- **Then:** no onboarding backdrop appears; the app lands directly on the restored project (or the brand placeholder if none).
- **Covers:** ADR-0017 §3 (completion recorded in view-state).
- **Automation:** Rust — `set_view_state("view-state:app:onboarding", {done:true})` then `get_view_state` after a re-init of `App` against the same `FARTCODE_DB_FILE`; frontend half via RTL with `getViewState` → `{done:true}` asserting `render` produces nothing.
- **Status:** implemented

#### FIRST-09 — Esc does not close onboarding
- **Given:** onboarding open on any step.
- **When:** press `Escape`.
- **Then (intended):** the flow closes and records completion, exactly like `skip`. **Actual:** nothing happens — `modalOpen()` counts `onboardingOpen` so the `modal` scope is active and `close-modal` fires, but `closeTopModal()` has no `onboardingOpen` branch (`store/ui.ts:145-156`), so the key is swallowed with no effect.
- **Covers:** E14-01 modal scope contract ("Esc closes the topmost modal").
- **Automation:** RTL — render `<Onboarding />` + `useCommands()`, press Escape, assert the card is still in the document.
- **Status:** not-built

#### FIRST-10 — Global chords fire underneath the onboarding backdrop
- **Given:** onboarding open on the welcome step.
- **When:** press `⌘⇧N`.
- **Then (intended):** nothing, or the onboarding "add project" step takes it. **Actual:** the Add-project dialog opens stacked on top of the onboarding card (global-scope commands stay active while a modal is open — `registry.ts:216-228` only suspends the *view* scopes). `⌘K` and `⌘,` stack the same way. `Escape` then peels the top dialog and leaves onboarding.
- **Covers:** E14-01 scope precedence.
- **Automation:** RTL — render `<App />`-equivalent with onboarding forced open, press `⌘⇧N`, assert both `[aria-label="Add project"]` and `.fc-onboard` are present.
- **Status:** partial (behaviour is reachable and recoverable, but unspecified by the design)

#### FIRST-11 — No way back into onboarding after skipping
- **Given:** the user skipped onboarding on the welcome step and has zero projects.
- **When:** they look for a way to re-run the guided flow (command palette `⌘K`, settings `⌘,`).
- **Then (intended):** a "Run first-run setup" command exists. **Actual:** `onboardingOpen` is only ever set from the boot-time view-state read; there is no command, button, or reset. The only recovery is the brand placeholder's `⌘⇧N` hint.
- **Covers:** ADR-0017 §3.
- **Automation:** grep-level assertion (`setOnboardingOpen` has exactly one non-store caller) or a registry test asserting no command matches /onboard/.
- **Status:** unreachable-entirely

---

### B · Adding a project

#### FIRST-12 — Open the add-project dialog from the rail
- **Given:** any state, onboarding closed.
- **When:** click the `+` tile in the left rail (dashed-bordered when there are zero projects) or press `⌘⇧N`.
- **Then:** a `.fc-overlay-card.fc-composer` dialog labelled "Add project" opens with a focused mono input, a `browse…` action, `esc cancel` and a disabled `↵ add project`.
- **Covers:** design_handoff_left_nav rail; commands.ts `new-project`.
- **Automation:** RTL over `<Nav />` + `<Modals />` with the ui store.
- **Status:** implemented

#### FIRST-13 — Pick a folder with the native dialog
- **Given:** the add-project dialog open.
- **When:** click `browse…` and choose a directory in the OS picker.
- **Then:** the chosen absolute path fills the input; `↵ add project` becomes enabled. If the plugin call throws, `Dialog failed: <error>` renders in the card instead.
- **Covers:** FLOWS.md F2 "Open a folder".
- **Automation:** RTL with `@tauri-apps/plugin-dialog`'s `open` mocked both ways.
- **Status:** implemented

#### FIRST-14 — Add a project and see it everywhere at once
- **Given:** the dialog open with a valid repo path.
- **When:** press `Enter`.
- **Then:** the dialog closes; a rail tile appears with the directory's first letter; the flyout opens on that project showing `…/parent/name · <baseRef>` and "nothing running"; the main region swaps from the brand placeholder to the board; and the project gains a row in the ⌘, settings left nav.
- **Covers:** E1-03/E1-04; ADR-0004 open lifecycle.
- **Automation:** backend half — Rust: `create_local` then assert an `InternalEvent::ProjectAdded{id,name,path}` on `event_bus.subscribe()`. Frontend half — RTL over `<Nav />` after seeding `useSidebar`.
- **Status:** implemented

#### FIRST-15 — Adding an already-added path duplicates the rail tile
- **Given:** project `/tmp/e2e-repo` already exists in the rail.
- **When:** add the exact same path again (either dialog).
- **Then (intended):** the app notices the duplicate and just selects the existing project. **Actual:** `create_local` returns the *existing* row **before** `finish_create`, so no `project:added` event fires and nothing reconciles — but `useSidebar.createProject` appends the returned project unconditionally (`store/sidebar.ts:148-155`), so the rail shows the same project twice (two tiles, same id) until the next full `load()` or restart. The dialog closes with no message either way.
- **Covers:** `create_local` "Duplicate path → open existing".
- **Automation:** store unit test — `createProject` twice with a mocked `createProject` returning the same DTO; assert `projects.length === 2` (currently) / `=== 1` (fixed).
- **Status:** partial (backend correct, frontend duplicates)

#### FIRST-16 — Add a project by cloning a git URL
- **Given:** no project for `https://github.com/org/repo.git`.
- **When:** the user chooses "Clone from GitHub" and supplies the URL.
- **Then (intended):** the repo clones into `localProject.defaultProjectsDirectory/<repo>`, clone progress is shown (FLOWS.md F2), and the project opens like a local add; an existing clone target errors with `clone target already exists: <path>`.
- **Covers:** FLOWS.md F1 step 1 / F2; `ProjectStore::create_clone`.
- **Automation:** the core path is testable (`create_clone` against a local bare repo URL) — but there is **no Tauri command** invoking it, so nothing in the app can reach it.
- **Status:** unreachable-entirely (backend `create_clone` exists at `fartcode-core/src/projects/mod.rs:376`; no `#[tauri::command]` and no `lib.rs` registration)

#### FIRST-17 — Connect a remote SSH host as a project source
- **Given:** the add-project surface.
- **When:** the user looks for the third source the design calls for.
- **Then (intended):** an SSH-host option sits beside "open folder" and "clone" (design_handoff_v2 7d: "First run also gains a third row when E12 lands: Connect an SSH host ⌘⇧O").
- **Covers:** FLOWS.md F1 gap; E12-04.
- **Automation:** none — the field exists (`projects.ssh_connection_id`, `repository_workspace_key` hashes `ssh:<conn>:<path>`) but nothing writes it (`finish_create` is always called with `None`).
- **Status:** not-built (explicitly deferred to E12)

#### FIRST-18 — Create a brand-new GitHub repository
- **Given:** the add-project surface.
- **When:** the user chooses "New" (FLOWS.md F2 "Pick/Clone/New").
- **Then (intended):** fartCode creates the remote repo and clones it.
- **Covers:** ADR-0004 ("GitHub 'new repo' creation is stubbed behind `RepoHostProvider` (E8)").
- **Automation:** `StubRepoHost::create_repository` returns `Err("... arrives with E8 — stubbed in Phase 0")`; assertable in Rust, unreachable from the UI.
- **Status:** not-built

#### FIRST-19 — Duplicate rail letters are indistinguishable
- **Given:** two projects whose names begin with the same letter (`ade` and `app-frontend`).
- **When:** look at the rail.
- **Then:** both tiles render the same glyph; the only disambiguation is hover (`title="<name> — right-click to delete"`) and tile order (project `created_at`).
- **Covers:** design_handoff_left_nav rail tiles.
- **Automation:** RTL — two projects, assert two buttons with identical text content and distinct `aria-label`s.
- **Status:** implemented (as designed; noted as a legibility risk, not a defect)

#### FIRST-20 — Auto-pull the project root on selection
- **Given:** two projects in the rail; the second was last selected more than 30 s ago.
- **When:** click the first project's rail tile.
- **Then:** the flyout reopens on that project and a background `project_git_pull` runs (`--ff-only`); a second click inside 30 s does not re-pull (in-memory cooldown, reset by restart). Failures are console-warned only — no toast, no error surface.
- **Covers:** `store/sidebar.ts:44-54` "ponytail" cooldown.
- **Automation:** store unit test with a mocked `projectGitPull` and a fake clock; assert one call for two rapid selects.
- **Status:** partial (silent failure — a pull error is invisible to the user)

---

### C · What arrives with a new project

#### FIRST-21 — A new project is seeded with the six default columns
- **Given:** a project just created from a repo with no prior fartCode state.
- **When:** the board renders (or `column_list(projectId)` is called).
- **Then:** exactly six columns come back in position order: **Backlog · Ready · Quick · In Progress · In Review · Done**. Backlog is the landing column; Quick and In Progress are `agent_step` with `on_enter: run`; In Review is `human_gate`; Done has `counts_as_done`.
- **Covers:** ADR-0037 item 8; `issues/columns.rs::SEED_COLUMNS`; `insert_row` seeds inside the create transaction.
- **Automation:** Rust — `create_local` then `column_list`, assert names/kinds/flags in order. Strong, cheap, falsifiable.
- **Status:** implemented

#### FIRST-22 — Seeded step columns carry their agent pins and advance targets
- **Given:** the freshly seeded board from FIRST-21.
- **When:** read the column config (board header sublines, or `column_list`).
- **Then:** `Quick` is pinned to `claude · haiku` and its `advance_to` points at **Done**; `In Progress` has no provider pin (renders the project default agent) and its `advance_to` is pinned at **In Review**, never "next by position".
- **Covers:** ADR-0037 item 4 + the fix-round note in `SEED_COLUMNS`; migration 0007.
- **Automation:** Rust — assert `step_provider/step_model` and resolved `advance_to` ids on the seeded rows.
- **Status:** implemented

#### FIRST-23 — The new project's board reads as empty, not broken
- **Given:** a project with the seeded columns and zero issues.
- **When:** the board finishes loading.
- **Then:** a `.board-empty` block reads "The board is empty." plus "Pull work onto it — the GitHub key above imports every open issue, or add a card by hand. Dragging one into Quick dispatches an agent in its own worktree." with an `a add issue` button. Before both fetches resolve the region reads "Reading the board…".
- **Covers:** ADR-0032; `BoardView.tsx:802-820`.
- **Automation:** RTL over `<BoardView />` with mocked `issueList`/`columnList`.
- **Status:** implemented

#### FIRST-24 — `.fartCode/` is excluded from the repo on open, without touching tracked files
- **Given:** a repo whose `.git/info/exclude` has no fartCode entry.
- **When:** the project is added (or reopened).
- **Then:** `.git/info/exclude` gains a `.fartCode/` line; `.gitignore` is untouched; `git status` in the repo stays clean. Re-adding is idempotent (no duplicate line).
- **Covers:** ADR-0004; `provider.rs::ensure_fartcode_git_excluded`.
- **Automation:** Rust — create a temp repo, `create_local`, read `.git/info/exclude`, run twice.
- **Status:** implemented

---

### D · Agent CLI detection and the install flow (7d)

#### FIRST-25 — Detection rows render the three states
- **Given:** the App pane of ⌘, settings (or onboarding step two), with at least one agent installed and one missing.
- **When:** the list renders.
- **Then:** installed rows show `<name>` (plus a green `default` tag on the default agent) and `<version> · <~/bin dir>`; missing rows carry class `missing` and read `not found · install`; while an install is in flight the row reads `installing` with no version and no controls.
- **Covers:** design_handoff_v2 7d; ADR-0011.
- **Automation:** RTL over `<AgentsList />` with three fabricated `HostDependencyDto` rows.
- **Status:** implemented

#### FIRST-26 — Install a missing agent CLI
- **Given:** an agent row reading `not found · install` whose registry entry has an install plan.
- **When:** click `install`.
- **Then:** the row flips to `installing`; when the backend settles, the row is replaced by the re-detected DTO — `<version> · <dir>` on success. The dialog stays open and usable throughout (the install runs on the blocking pool).
- **Covers:** ADR-0011; `commands/dependencies.rs::host_dependency_install`.
- **Automation:** RTL for the row transitions with a deferred mock; the real install is not CI-safe (it shells out to npm/curl).
- **Status:** implemented

#### FIRST-27 — Install with no visible progress or consent moment
- **Given:** a not-found agent whose install plan is `InstallType::Curl` (a `curl … | bash` one-liner).
- **When:** click `install`.
- **Then (intended):** 7d's `installing · 62%` row with a 2px accent progress bar, and — per the ADR-0011 security note — a confirmation naming the exact command before unvetted remote code runs with the user's privileges. **Actual:** one click runs `sh -c "<vendor one-liner>"` immediately with no confirm; the row shows a bare `installing` label because `ProcessInstallRunner` buffers output and flushes once at the end (no progress events exist).
- **Covers:** ADR-0011 "SECURITY NOTE … Surfaced in the install UI (E1-08) before running"; design_handoff_v2 7d.
- **Automation:** RTL asserting no confirm dialog appears; the missing progress feed is structural (documented at `commands/dependencies.rs:9`).
- **Status:** not-built (missing confirm on a code-execution action; progress state unreachable)

#### FIRST-28 — Report an install failure inline
- **Given:** an agent whose install plan is absent, or whose installer exits non-zero.
- **When:** click `install`.
- **Then:** the `installing` state clears and a `.fc-set-error` line appears above the list carrying `no installer for <id>` / `install failed for <id> (exit != 0)`. The row returns to `not found · install`.
- **Covers:** `dependencies/mod.rs::install`; `store/dependencies.ts:52-66`.
- **Automation:** RTL with `hostDependencyInstall` rejecting; Rust test on `install()` for a plan-less provider.
- **Status:** implemented

#### FIRST-29 — The "update available" affordance can never appear
- **Given:** an installed agent CLI with a newer published version.
- **When:** open the agents list.
- **Then (intended):** the row reads `0.48.0 · update ⌄` and clicking runs the manager's update command. **Actual:** `latest` is always `None` (`HostDependencyStore::latest_version` is a Phase-0 stub returning `None`), so `updateAvailable` is never true and the branch at `AgentsList.tsx:62-72` is dead. `host_dependency_update` is registered but has no reachable caller.
- **Covers:** ADR-0011 "Network update checks stubbed … until E3-05"; design_handoff_v2 7d.
- **Automation:** Rust — assert `latest_version(any)` is `None`; RTL — feed a row with `latest !== version` to prove the branch renders when fed (i.e. only the data is missing).
- **Status:** unreachable-by-mouse

#### FIRST-30 — Uninstall an agent CLI
- **Given:** an installed agent the user wants removed.
- **When:** they look for an uninstall control.
- **Then (intended):** an uninstall action on the row, with a confirm.
- **Covers:** ADR-0011 (uninstall is part of the ticket).
- **Automation:** `HostDependencyStore::uninstall` is testable in Rust; there is no Tauri command and no UI.
- **Status:** unreachable-entirely

#### FIRST-31 — Pick the default agent, and see it everywhere
- **Given:** two or more installed agents; ⌘, → a project row → the **Agent** group.
- **When:** click the `Default agent · model` row and choose a different agent.
- **Then:** the chosen row gains the green `default` tag; the `setting:changed` event with key `defaultAgent` fires; the 7d agents list under App settings moves its `default` tag to the same provider; and the board's step-column subline for unpinned steps now names the new agent.
- **Covers:** `commands/settings.rs::set_default_agent` (validated against the provider registry, emits `SettingChanged`).
- **Automation:** Rust — `set_default_agent_core(&app, "codex")`, assert the stored setting and the emitted event (this test already exists at `commands/settings.rs:144`). Frontend — RTL on the event listener in `ProjectSettingsPane`.
- **Status:** implemented

#### FIRST-32 — The per-project "Default agent" row is not per-project
- **Given:** two projects, A and B.
- **When:** open ⌘, → project **A** → Agent → pick `codex`.
- **Then (intended, per 7c's "project pane"):** only project A's new tasks use codex. **Actual:** the row writes the app-wide `defaultAgent` setting, so project B changes too. The row's value string is also `<agent> · default` — a literal, not a model picker, so 7c's "default agent · model ⌄" has no model half.
- **Covers:** design_handoff_v2 7c Agent group.
- **Automation:** Rust — set the default from project A's command path, read effective settings for project B.
- **Status:** partial (scope mismatch between the pane and the setting)

#### FIRST-33 — No installed agents, seen from the project pane
- **Given:** a machine with zero detected agent CLIs.
- **When:** open the `Default agent · model` row.
- **Then:** the menu body reads "no installed agents — install one under App settings"; the row's collapsed value still reads `claude · default` (the `defaultAgentName` fallback), which names an agent that is not present.
- **Covers:** `ProjectSettings.tsx:529-533`; `store/dependencies.ts:86-88`.
- **Automation:** RTL with an empty `deps` array.
- **Status:** partial (the fallback label contradicts the menu copy)

---

### E · Project settings

#### FIRST-34 — Reach a project's settings
- **Given:** at least one project.
- **When:** press `⌘,` (or click the `⌘` rail tile) and click the project's row in the 170px left nav.
- **Then:** the pane title becomes the project name and three groups render — **Repo** (Default branch, Base remote · push remote, GitHub account), **Workspaces** (Worktree directory, tmux terminals, Preserve into new worktrees, Shell setup, Task startup command, Provision · terminate commands), **Agent** (Default agent · model, Scripts, two auto-run toggles) — over a footer legend and `⇧⌘s share local values`.
- **Covers:** design_handoff_v2 7c full field map.
- **Automation:** RTL over `<SettingsModal initialSection={`project:${id}`} />` with `getProjectSettings`/`projectSettingsProvenance` mocked.
- **Status:** implemented

#### FIRST-35 — The "sidebar gear" project-settings path is dead
- **Given:** any project.
- **When:** look for a per-project gear that opens settings scoped to that project.
- **Then (intended):** `projectSettingsOpen` opens `SettingsModal` at `project:<id>` (that wiring exists in `Modals.tsx:771-779`). **Actual:** `setProjectSettingsOpen(true)` has zero callers — the old `ProjectHeader.tsx` that held the gear is deleted and nothing replaced it, and `ProjectView.tsx`'s own header comment still refers to it. The only route is ⌘, plus a click.
- **Covers:** design_handoff_v2 7c; ADR-0033 unified header.
- **Automation:** grep assertion / registry test (no command opens project settings).
- **Status:** unreachable-entirely

#### FIRST-36 — Edit a row inline and have it persist immediately
- **Given:** the project pane, `Default branch` reading `main`.
- **When:** click the row, type `develop`, press `Enter`.
- **Then:** the editor closes, the row value reads `develop`, and a reopen of the modal (or a `get_project_settings` call) still returns `develop`. `Escape` inside the editor closes only the editor — the settings modal stays open.
- **Covers:** 7c "edits save immediately through updateProjectSettings"; `InlineInput` stops Escape propagation.
- **Automation:** RTL for the gesture; Rust round-trip on `update_project_settings` + `get_project_settings`.
- **Status:** implemented

#### FIRST-37 — Reject an invalid worktree directory, but leave the bad value on screen
- **Given:** the `Worktree directory` row.
- **When:** enter `relative/path` and press `Enter`.
- **Then:** a `.fc-set-error` line appears with the typed `invalid-worktree-directory` message. **But** the row still displays `relative/path`, because `commit()` sets optimistic state and never rolls it back on rejection (`ProjectSettings.tsx:227-239`) — reopening the modal reveals the old value was kept. `~/x` expands and is accepted; a blank value clears the field.
- **Covers:** ADR-0016; `commands/settings.rs:45-63`.
- **Automation:** RTL — reject `updateProjectSettings`, assert both the error node and the stale row text.
- **Status:** partial (validation correct; the UI lies about what was saved)

#### FIRST-38 — The per-project worktree directory has no effect on worktrees
- **Given:** project settings with `Worktree directory` set to `/tmp/custom-worktrees`.
- **When:** create a task in that project.
- **Then (intended):** the worktree lands under `/tmp/custom-worktrees/<project>/<branch>`. **Actual:** it lands under the *app-level* `localProject.defaultWorktreeDirectory` (`~/fartCode/worktrees`) — `worktree_pool_path` reads only the app setting (`projects/provider.rs:169-177`), and `ProjectSettings.worktree_directory` is read/validated but consumed by nothing. The row displays `~/fartCode/worktrees` as its placeholder default, which makes it look authoritative.
- **Covers:** design_handoff_v2 7c Workspaces; FLOWS.md F2 worktree-dir note; ADR-0016.
- **Automation:** Rust — set the project setting, create a worktree, assert the resulting path ignores it. Directly falsifiable.
- **Status:** not-built

#### FIRST-39 — App-level workspace defaults are unreachable
- **Given:** a user who wants worktrees somewhere other than `~/fartCode/worktrees`, or clones somewhere other than `~/fartCode/repositories`.
- **When:** they open ⌘, → App.
- **Then (intended):** the `localProject` group is editable. **Actual:** the App pane renders only `<AgentsList />` and `<ProviderAccounts />`; no `get_app_setting`/`set_app_setting`/`settings_reset` Tauri commands are registered at all (`lib.rs:134-234`), so the only setting the app can write is `defaultAgent` (plus resource-monitor enable). `SettingsStore::reset` has no caller.
- **Covers:** ADR-0002 settings-store architecture.
- **Automation:** grep/registration assertion over the `invoke_handler` list.
- **Status:** unreachable-entirely

#### FIRST-40 — Base and push remote save on blur
- **Given:** the `Base remote · push remote` row showing `origin · origin`.
- **When:** open it, set base to `upstream`, tab out; set push to `fork`, click elsewhere.
- **Then:** the collapsed row reads `upstream · fork`; leaving push blank makes it fall back to base in the display (`push = s.pushRemote || base`) while storing `null`. `Escape` closes the editor without saving the field being typed.
- **Covers:** 7c Repo group.
- **Automation:** RTL blur simulation + a Rust round-trip.
- **Status:** implemented

#### FIRST-41 — The GitHub account row is a free-text id that nothing reads
- **Given:** the `GitHub account` row reading `—`.
- **When:** type any string and save.
- **Then:** the row displays it and it round-trips through `get_project_settings`. **But** 7c specifies a `⌄` picker, and `github_account_id` has no consumer anywhere in `fartcode-core`, `fartcode-app`, or `fartcode-git` — PR/issue calls do not read it.
- **Covers:** design_handoff_v2 7c Repo group.
- **Automation:** Rust round-trip proves storage; a grep proves no consumer.
- **Status:** partial

#### FIRST-42 — Preserve patterns copy real files into a new worktree
- **Given:** the project root contains an untracked `.env`; `Preserve into new worktrees` lists `.env`.
- **When:** create a task (new worktree).
- **Then:** the worktree contains a copy of `.env`; `.fartCode.json` is never copied even if a pattern would match it; unsafe patterns (absolute / `..`) are skipped silently.
- **Covers:** `projects/worktrees.rs::copy_preserved_files`; `DEFAULT_PRESERVE_PATTERNS`.
- **Automation:** Rust — temp repo + `.env`, create a worktree, assert the file is present.
- **Status:** implemented

#### FIRST-43 — Shell setup is prepended to lifecycle scripts only
- **Given:** `Shell setup` = `source .envrc`, `Scripts → setup` = `npm ci`.
- **When:** the setup script runs (⌘J drawer, or auto-run).
- **Then:** the PTY runs `sh -c` on `source .envrc\nnpm ci` (with the command-echo prefix). A blank/whitespace shell setup is not prepended. **Note:** shell setup is *not* applied to a plain `⌘⇧T` terminal or to the agent terminal — only to lifecycle scripts.
- **Covers:** `commands/lifecycle.rs::lifecycle_script_text` (+ its two existing unit tests at `:282` and `:298`).
- **Automation:** the Rust unit tests already exist and are the falsification; the "not applied to plain terminals" half needs a terminal-spawn test.
- **Status:** partial (scope of `shellSetup` is narrower than the label "Shell setup" implies)

#### FIRST-44 — Share local values into `.fartCode.json` with ⇧⌘S
- **Given:** the project pane with `Preserve into new worktrees` edited locally (no `shared` tag) and the repo having no `.fartCode.json`.
- **When:** press `⇧⌘S`.
- **Then:** `.fartCode.json` is created at the repo root containing `preservePatterns`; the DB's shareable blob is cleared; the label gains the green `shared` tag; a notice reads "local values moved into .fartCode.json" for 4 s. The file write is atomic (temp + rename).
- **Covers:** E1-02; design_handoff_v2 7c footer; `settings/service.rs::share_with_team`.
- **Automation:** Rust — the round-trip has a test at `settings/service.rs:921`; extend it to assert the on-disk file. RTL for the chord + tag flip.
- **Status:** implemented

#### FIRST-45 — ⇧⌘S with nothing local still claims success
- **Given:** the project pane with no local shareable overrides.
- **When:** press `⇧⌘S`.
- **Then (intended):** a neutral "nothing to share" notice. **Actual:** `share_with_team` returns `false`, but `project_settings_share` maps it to `()`, so the pane flashes "local values moved into .fartCode.json" and no file is written.
- **Covers:** `commands/settings.rs:83-89`.
- **Automation:** Rust — assert `share_with_team` returns `false` and no file exists; RTL — assert the notice text still appears.
- **Status:** partial

#### FIRST-46 — ⇧⌘S fails loudly on a malformed `.fartCode.json`
- **Given:** the repo contains a `.fartCode.json` that is not valid JSON, and a local shareable override exists.
- **When:** press `⇧⌘S`.
- **Then:** a `.fc-set-error` line reads `cannot share settings: <path> is not valid JSON (<parse error>)`; the file is left byte-identical; the local values stay local.
- **Covers:** `settings/service.rs:612-626` (parse failure → error, never overwrite).
- **Automation:** Rust — write garbage, call `share_with_team`, assert `Err` and unchanged file bytes.
- **Status:** implemented

#### FIRST-47 — Provenance tags distinguish default / shared / local
- **Given:** a repo whose `.fartCode.json` sets `shellSetup`, and a project where the user then overrides `shellSetup` locally.
- **When:** open the project pane.
- **Then:** immediately after project creation all three shareable keys read `default` (the seeded default `preservePatterns` materialized into the DB must **not** read as local); with only a file value the label carries the green `shared` tag; after a local edit the tag disappears; clearing the local value restores `shared`.
- **Covers:** `settings/service.rs::shareable_provenance` (tests at `:876` and `:885`).
- **Automation:** Rust — the two existing tests plus a clear-and-restore case; RTL for tag rendering.
- **Status:** implemented

#### FIRST-48 — Toggle rows commit with no confirmation and no undo
- **Given:** `tmux terminals`, `Auto-run setup on new tasks`, `Auto-run run script on new tasks`.
- **When:** click any of them once.
- **Then:** the value flips `off`↔`on` and persists immediately; there is no confirm and no undo. Turning `Auto-run run script` on means the *next* task creation spawns the run script's PTY without further consent.
- **Covers:** 7c Agent group; `commands/lifecycle.rs::run_auto_lifecycle_scripts`.
- **Automation:** RTL for the toggle; Rust for `auto_run_enabled` gating the spawn.
- **Status:** implemented (flagged: an auto-run toggle is a "spend/execute" switch with no confirm)

---

### F · Lifecycle scripts and the ⌘J drawer

#### FIRST-49 — Open the drawer and run a configured script
- **Given:** a task open, project `scripts.setup` configured, setup never run.
- **When:** press `⌘J`.
- **Then:** a 210px bottom sheet appears with three tabs (`setup`/`run`/`teardown`), `r rerun · ⌘j close` on the right, and the body reads "not run yet · r runs it" until a run exists. Pressing `r` while the drawer chrome holds focus spawns the script's PTY and the body swaps to a live terminal whose first lines are the dim `$ `-prefixed script commands.
- **Covers:** design_handoff_v2 7b; ADR-0014; `Drawer.tsx`.
- **Automation:** RTL for chrome/tabs/`r`; Rust for `terminal_open_lifecycle` producing a terminal entry with `kind: "lifecycle"`.
- **Status:** implemented

#### FIRST-50 — A failed script labels its own tab
- **Given:** `scripts.setup` = `exit 1`; setup has run once.
- **When:** look at the drawer strip.
- **Then:** the setup tab reads `setup ✗ exit 1` with the `failed` class (red underline); the task view's empty pane refuses to start the agent and `⌘T` opens the drawer on setup instead of spawning.
- **Covers:** 7b "a failed setup blocks agent start"; `commands.ts::resumeAgentTab`.
- **Automation:** RTL — seed `useScripts.byTask[taskId].setup = {exitCode:1}` and assert both the tab label and that `resumeAgentTab` does not call `terminalOpenAgent`.
- **Status:** implemented

#### FIRST-51 — Running an unconfigured script errors invisibly
- **Given:** a project with no `teardown` script.
- **When:** switch the drawer to the `teardown` tab and press `r`.
- **Then (intended):** the drawer says the script is not configured. **Actual:** `terminal_open_lifecycle` rejects with `no teardown script configured for this project`, `Drawer.rerun` catches it into `console.error`, and the body keeps reading "not run yet · r runs it" — the user gets no feedback at all.
- **Covers:** `Drawer.tsx:33-38`; `commands/lifecycle.rs:104-109`.
- **Automation:** RTL — reject `terminalOpenLifecycle`, assert no error node appears anywhere in the drawer.
- **Status:** partial

#### FIRST-52 — Teardown never runs on its own
- **Given:** a project with a `teardown` script configured, and a task with a worktree.
- **When:** delete the task (⌘⌫ from the delete confirm).
- **Then (intended):** the teardown script runs before the worktree is removed. **Actual:** teardown has no auto-run setting (`auto_run_enabled` returns `Some(false)` for it) and task deletion never invokes it — ADR-0023 explicitly notes "Phase 0 lacks teardown SCRIPTS". The only way to run teardown is the drawer's `r`, and only while the task still exists.
- **Covers:** ADR-0023; ADR-0014.
- **Automation:** Rust — configure teardown, delete a task, assert no lifecycle terminal of type `teardown` was created.
- **Status:** not-built

#### FIRST-53 — The drawer is task-scoped only
- **Given:** a project selected with no task open (board view).
- **When:** press `⌘J`.
- **Then:** nothing happens — `toggle-drawer` is `task-view` scope and `Drawer` only renders inside `TaskView`. There is no project-scoped place for lifecycle output.
- **Covers:** `commands.ts:300-308`; `TaskView.tsx:73`.
- **Automation:** RTL — no task selected, press `⌘J`, assert `.drawer` is absent.
- **Status:** implemented (as designed)

---

### G · Deleting / forgetting a project

#### FIRST-54 — Right-click a rail tile to delete a project
- **Given:** a project in the rail.
- **When:** right-click (context menu) its tile.
- **Then:** a confirm card opens: "Delete <name>?" over "Tasks, worktrees, and rows are torn down. The repository on disk is left untouched.", with `esc cancel` and `↵ delete`.
- **Covers:** `Nav.tsx:98-101`; `Modals.tsx:780-789`.
- **Automation:** RTL — `fireEvent.contextMenu` on the tile, assert the dialog.
- **Status:** implemented

#### FIRST-55 — Deleting a project has no keyboard or palette route
- **Given:** a project selected.
- **When:** try `⌘K` → "delete project", or any chord.
- **Then (intended):** the palette lists it. **Actual:** no `delete-project` command is registered; the only affordance is right-click on the rail tile (discoverable solely through the tile's `title` tooltip).
- **Covers:** E14-01 registry; design_handoff_left_nav.
- **Automation:** registry test — assert no command id matches /delete-project/.
- **Status:** unreachable-by-mouse-only (keyboard/palette path absent)

#### FIRST-56 — Delete tears down rows and the worktree pool, not the repo
- **Given:** a project with two tasks, each with a worktree under `~/fartCode/worktrees/<name>/`.
- **When:** confirm the delete.
- **Then:** the project row, its tasks, conversations, issues, board columns and workspace rows are gone; `~/fartCode/worktrees/<name>/` is removed from disk; the repository at the project path is untouched; a `project:deleted` event fires and the rail selects the first remaining project (or falls back to the brand placeholder).
- **Covers:** E1-04 teardown; `DbProjectStore::delete`.
- **Automation:** Rust — create project + tasks in a temp `FARTCODE_DB_FILE`, delete, assert rows gone, pool dir gone, repo intact, `ProjectDeleted` on the bus. Fully falsifiable.
- **Status:** implemented

#### FIRST-57 — Deleting a project does not stop its running agents
- **Given:** a project with a task whose agent terminal is live (and possibly an ACP session).
- **When:** delete the project.
- **Then (intended):** the confirm itemizes "kills N running agents" — the way `DeleteTaskConfirm` does — and the delete stops ACP sessions and closes/reaps the task terminals. **Actual:** `delete_project` calls `projects.delete()` + `step_engine::on_project_deleted()` only; unlike `delete_task` (which calls `acp.stop_task` and `terminals.close_task`, `commands/tasks.rs:302,312`), nothing touches terminals or ACP. Orphaned PTYs — and orphaned tmux sessions when the project had `tmux` on — survive the delete, and the confirm never mentions a running agent.
- **Covers:** ADR-0023 (task-side teardown contract) vs `commands/projects.rs:26-34`; design_handoff_v2 7a confirm itemization.
- **Automation:** Rust — spawn a lifecycle/agent terminal for a task, delete the project, assert the terminal manager still lists a live session.
- **Status:** not-built

#### FIRST-58 — Two same-named projects share one worktree pool
- **Given:** `~/work/ade` and `~/archive/ade` both added as projects (same directory *name*, different paths); both have worktrees.
- **When:** delete either one.
- **Then (intended):** only that project's worktrees are removed. **Actual:** the pool segment is `safe_path_segment(project.name)`, so both projects live in `~/fartCode/worktrees/ade/`, and deleting one `remove_dir_all`s the other project's on-disk worktrees. The surviving project's tasks keep DB rows pointing at deleted directories. The confirm says nothing about this.
- **Covers:** the documented limitation at `projects/mod.rs:320-326` (ADR-0015).
- **Automation:** Rust — two projects with the same basename, create a worktree in each, delete one, assert the other's worktree directory is gone. Directly falsifiable.
- **Status:** not-built (known data-loss limitation, no guard, no warning)

#### FIRST-59 — A failed delete keeps the dialog and shows why
- **Given:** the delete confirm open and the backend rejecting (e.g. project id no longer present).
- **When:** press `↵`.
- **Then:** the dialog stays open, the buttons re-enable, and a `role="alert"` paragraph carries the backend error. No silent delete.
- **Covers:** `ConfirmDelete` ("Awaited: the modal stays open and shows the failure inline").
- **Automation:** RTL with `deleteProject` rejecting.
- **Status:** implemented

#### FIRST-60 — Re-adding a deleted project's path starts clean
- **Given:** a project was deleted; its repo still exists on disk with `.fartCode/` excluded and possibly a `.fartCode.json`.
- **When:** add the same path again.
- **Then:** a new project id is minted, settings are re-seeded, the six default columns are seeded again, `.fartCode.json` values re-read as `shared`, and the columns store's `project:deleted` eviction guarantees no stale column cache is served for the new id.
- **Covers:** `store/columns.ts` eviction note; `seed_project_settings` idempotence.
- **Automation:** Rust — create, delete, create, assert six fresh columns and re-seeded settings.
- **Status:** implemented

---

### H · Restart, concurrency, and narrow layouts

#### FIRST-61 — Selection and flyout state survive a relaunch
- **Given:** project B selected with task T open and the flyout collapsed (`⌘\`).
- **When:** quit and relaunch.
- **Then:** project B is selected, task T is reopened (only if the task row still exists — a stale id falls back to no task), and the flyout is still collapsed. If the saved project id is gone, the app falls back to the first project.
- **Covers:** ADR-0017 §1–2; `store/sidebar.ts::load`; `store/ui.ts` `fc:sidebarVisible` in localStorage.
- **Automation:** store unit test with mocked `getViewState` returning stale/valid ids; the localStorage half is a `store/ui` test.
- **Status:** implemented

#### FIRST-62 — Stale view-state rows are pruned at boot
- **Given:** kv rows `view-state:task:<deleted-id>` and `view-state:project:<deleted-id>` left behind.
- **When:** the app boots.
- **Then:** those rows are gone; `view-state:app:onboarding`, `view-state:app:sidebar` and `view-state:app:keybindings` survive (the prune only targets `task:`/`project:` scopes).
- **Covers:** ADR-0017 §1; `view_state::prune_orphans`.
- **Automation:** Rust — insert orphans, run `prune_orphans`, assert the app-level keys remain.
- **Status:** implemented

#### FIRST-63 — A second launch focuses the existing window
- **Given:** fartCode already running.
- **When:** launch the app again from Finder/CLI.
- **Then:** no second window opens; the existing window is unminimized and focused.
- **Covers:** ADR-0017 §4; `tauri_plugin_single_instance` at `lib.rs:40`.
- **Automation:** needs a driver we lack (requires launching two real app processes).
- **Status:** implemented (untestable here)

#### FIRST-64 — Two projects run agents at once without cross-talk
- **Given:** project A with a running agent and project B with a running agent.
- **When:** switch between rail tiles.
- **Then:** each rail tile carries its own status dot (filled amber for a running task, hollow amber for review, computed per project from that project's tasks); the flyout shows only the selected project's Needs-you/Running/Recent groups; neither switch interrupts the other project's terminals.
- **Covers:** `Nav.tsx::agentState`; ADR-0033 one agent terminal per task.
- **Automation:** RTL — seed two projects with differing task statuses, assert two `.tile-dot` elements with the right modifier classes.
- **Status:** implemented

#### FIRST-65 — The rail never disappears; the flyout is the only collapsible half
- **Given:** any project selected.
- **When:** press `⌘\` (or `⌘B`), then click a rail tile.
- **Then:** the 244px flyout hides and the 56px rail stays; clicking any rail tile both selects that project and forces the flyout back open (the mouse-only path back from a collapsed flyout). The collapsed state persists across relaunch.
- **Covers:** design_handoff_left_nav; `Nav.tsx:92-97`.
- **Automation:** RTL — toggle, assert `.flyout` absent and `.rail` present, click a tile, assert it returns.
- **Status:** implemented

#### FIRST-66 — Settings and onboarding at a small window width
- **Given:** the window narrowed to ~900px.
- **When:** open ⌘, settings, then onboarding (fresh DB).
- **Then:** the settings card clamps to `calc(100vw - 80px)` with the 170px nav column intact and the pane scrolling vertically; the onboarding card clamps the same way at `max-height: 85vh` with its own scroll. Neither surface introduces a horizontal page scrollbar.
- **Covers:** `styles/settings.css:10-13` and `:668-672`.
- **Automation:** RTL cannot assert layout; needs a driver we lack (or a visual/snapshot harness).
- **Status:** implemented (unverifiable in the current test setup)

#### FIRST-67 — There are no responsive breakpoints outside the board
- **Given:** the window narrowed below ~900px with a project open.
- **When:** compare the board and the rest of the shell.
- **Then:** the board collapses to its single-column + strip mode (measured with a `ResizeObserver` against `NARROW_PX`, not a media query). Everything else — rail, flyout, changes sheet, settings nav — keeps fixed pixel widths; the only `@media` rules in the whole stylesheet set are two `prefers-reduced-motion` blocks. **Intended:** a laptop-width layout contract for the shell (what collapses first when rail + flyout + board + changes sheet no longer fit).
- **Covers:** `BoardView.tsx:305-316`; `styles.css` (no layout media queries).
- **Automation:** grep assertion over the CSS; the shell behaviour needs a driver we lack.
- **Status:** partial (board only; shell unspecified)

---

## 2 · Board and pipeline columns

The board (`app-frontend/src/components/board/BoardView.tsx`) is the project view's only surface: one plate of N hairline-ruled columns rendered from `board_columns` in `position` order, where every semantic — header subline, run-vs-queue on drop, hold-vs-advance on settle, done-dimming, landing — is column data, not a name test. Every cross-column move goes through the one backend primitive `issue_enter_column` → `step_engine::enter_column_from_command`, which runs, parks, or does nothing and hands back a launch payload; within-column reorder deliberately stays on the legacy `issue_move`. Card run-state derives from the live agent terminal (`runState.ts` + `store/scripts.ts`), never from the card's column, and the board never kills an agent.

Scope note: the E18-07 authority flip is still open (`issues.lane` is authoritative, `issues.column_id` is a maintained mirror), the settings Columns editor (#67, handoff §8d) is unbuilt, and a fix round for known board defects is in flight — scenarios below describe intended behavior and mark status honestly.

---

### Rendering from config

#### BOARD-01 — Render every configured column in position order
- **Given:** a freshly added project (migration 0006 / `seed_default_columns` seeded Backlog · Ready · Quick · In Progress · In Review · Done) with no cards.
- **When:** the user selects the project in the rail so `ProjectView` mounts, in a pane ≥900px wide, and adds one card so the empty state clears.
- **Then:** six column heads render left-to-right in exactly that order, each with its name, a mono count, and a kind subline; the grid is `repeat(6, minmax(0,1fr))` (the `--column-count` custom property equals 6, never a hardcoded 5).
- **Covers:** ADR-0037 items 1/8; handoff v3 §8a.
- **Automation:** backend `column_list` + RTL render of `BoardView` with a mocked `columnList` returning 6 rows; assert head order and `--column-count`.
- **Status:** implemented

#### BOARD-02 — Say what each column does in its subline, and brighten confirm-free spend
- **Given:** the seeded board, with the app's default agent set to `claude`.
- **When:** the user looks at the six column heads.
- **Then:** Backlog/Ready read `shelf`, In Review reads `human gate`, Done reads `counts as done`, Quick reads `claude · haiku — run → Done`, In Progress reads `<default agent> — run → In Review`; the two `run` sublines render at `#9a9aa1` (`[data-tone="run"]`) while non-step sublines render `--meta-dim`. A queue-mode step would render `--meta` (`[data-tone="queue"]`).
- **Covers:** ADR-0037 items 3/4 + "Cost surface"; handoff v3 §8a "Confirm-free spend is brighter"; DESIGN.md "Pipeline board".
- **Automation:** unit test `columnConfigSummary`/`columnSublineTone` in `lib/columnConfig.ts` (pure) + RTL assertion on `[data-tone]`.
- **Status:** implemented

#### BOARD-03 — Dim a terminal column from the flag, never the name
- **Given:** the seeded board plus a second user-created column "Shipped" with `counts_as_done: true` (created via the `column_create` command — there is no UI, see BOARD-46), one card in Done and one in Shipped.
- **When:** the user views the board.
- **Then:** both Done and Shipped heads carry `data-done`, their names render `#6e6e75`, and cards inside both render at 50% opacity — identically. Renaming Done to "Archive" changes nothing about the dimming.
- **Covers:** ADR-0037 item 6; handoff v3 §8a "counts_as_done = dimmed".
- **Automation:** RTL with a mocked `columnList` carrying two `countsAsDone` columns; assert `[data-done]` on both `.board-lane-head`/`.board-lane`.
- **Status:** implemented

#### BOARD-04 — Tag the landing column and hang the add key off it
- **Given:** the seeded board (Backlog is `is_landing`).
- **When:** the user views the column heads.
- **Then:** Backlog's name row shows a mono `landing` tag in `--meta` (never green) and a `+` button labelled "Add issue to Backlog"; no other head has either. Moving the flag to Ready via `column_update` and reloading moves both the tag and the `+` to Ready.
- **Covers:** ADR-0037 item 7; handoff v3 §8a "Landing tag".
- **Automation:** RTL on mocked columns; backend `column_update {isLanding:true}` + `column_list` assert exactly one holder.
- **Status:** implemented

#### BOARD-05 — Keep the per-column count truthful across a move
- **Given:** Backlog holds 3 cards, Ready holds 0.
- **When:** the user drags one Backlog card into Ready.
- **Then:** the Backlog count reads 2 and Ready reads 1 without a manual refresh (the `issue:updated` event drives the refetch).
- **Covers:** ADR-0037 item 1; BoardView.tsx:232-245 reconcile-on-event.
- **Automation:** backend `issue_enter_column` + assert the `issue:updated` envelope, then `issue_list` grouping.
- **Status:** implemented

---

### Empty, first-run and failure states

#### BOARD-06 — Teach the empty board how work gets on it
- **Given:** a project with seeded columns and zero issues.
- **When:** the board renders.
- **Then:** no columns are drawn at all; the pane shows "The board is empty.", a paragraph naming the first `agent_step` column ("Dragging one into Quick dispatches an agent in its own worktree."), and an `a add issue` key button. The paragraph also says "the GitHub key above imports every open issue" — **there is no such key**: `ProjectHeader.tsx` was deleted and `App.tsx` renders no header at project scope.
- **Covers:** ADR-0032 board onboarding; handoff v3 migration notes ("copy becomes template instances").
- **Automation:** RTL with `issueList` → `[]`; assert copy and that no `.board-lane-head` renders.
- **Status:** partial (dead copy reference; empty board also hides the column structure the user is being told to drag into)

#### BOARD-07 — Say so when the column read fails
- **Given:** `column_list` fails (e.g. DB mutex poisoned / project row gone).
- **When:** the user opens the project.
- **Then:** the error string renders in `.board-error` at the top of the pane, and the board must not sit on "Reading the board…" forever. Today it additionally renders "This project has no columns." underneath, which is a second, false claim.
- **Covers:** BoardView.tsx:770-776 ("A column read that failed must SAY so").
- **Automation:** RTL with `columnList` rejecting; assert `.board-error` text and the absence of the "no columns" copy.
- **Status:** partial

#### BOARD-08 — Say so when the issue read fails
- **Given:** `issue_list` fails while `column_list` succeeds.
- **When:** the user opens the project.
- **Then:** intended — the error renders and the board shows a read-failed state with a retry. Today it renders the error *and* "The board is empty." with an add-issue key, which invites the user to add cards on top of an unknown card set.
- **Covers:** —
- **Automation:** RTL with `issueList` rejecting, `columnList` resolving.
- **Status:** not-built

#### BOARD-09 — Import open GitHub issues on board entry, once per minute
- **Given:** a project whose checkout has a GitHub remote and 3 open issues, none imported; `gh` installed and authed.
- **When:** the user opens the project view.
- **Then:** three cards appear in the landing column within a few seconds, each titled `#N …`, each carrying a `gh` chip in its meta line that opens the issue URL; re-opening the project within 60s imports nothing again. With `gh` missing or unauthed nothing appears and **no error is surfaced anywhere** (console warning only).
- **Covers:** BoardView.tsx:87-107 + 224-231; `issue_import_github` dedupe by `external_ref`.
- **Automation:** backend `project_github_issues` + `issue_import_github` + assert `issue:created`; the cooldown needs an RTL test with a mocked clock.
- **Status:** partial (silent failure; also no consent or opt-out for the automatic import)

---

### Drag mechanics

#### BOARD-10 — Drag a card between two shelves
- **Given:** card A in Backlog, Ready holding two cards.
- **When:** the user drags A over Ready and releases between the two cards.
- **Then:** a 1px accent insertion line (never a ghost box) shows between them during the drag; on release A renders in Ready at that index and the drop line disappears.
- **Covers:** handoff v2 frame 4b drag physics; ADR-0037 item 10 (`move_to` stays permissive — no ordered traversal required).
- **Automation:** RTL `dragOver`/`drop` on `.board-lane-cards` with a stubbed `getBoundingClientRect`; backend `issue_enter_column` with a position.
- **Status:** implemented

#### BOARD-11 — Reorder inside one column
- **Given:** Backlog holds cards A, B, C (positions 0, 1, 2).
- **When:** the user drags A below C.
- **Then:** the column renders B, C, A and the order survives a reload. The move must go through `issue_move` (position only), never the enter primitive, so a step column's session is not re-entered by a reorder.
- **Covers:** BoardView.tsx:434-438 + 652-660; step_engine.rs:872-896 (`on_lane_move` same-lane early return).
- **Automation:** backend `issue_move` + `issue_list` order assertion.
- **Status:** partial — the write sets one card's `position` without shifting its siblings (`issues/mod.rs:598-623`, `:669-737`), so positions collide and the resulting order is decided by the `created_at` tiebreak in `list_for_project`. Dropping two cards at the same index makes the rendered order unpredictable.

#### BOARD-12 — Drop into an empty column
- **Given:** Ready holds zero cards.
- **When:** the user drags a card anywhere over Ready's (blank) body.
- **Then:** the drop registers — `.board-lane-placeholder` keeps a minimum 96px live drop target — the insertion line shows, and the card lands in Ready.
- **Covers:** `columnConfig.groupByColumn` gives every column an entry; board.css `.board-lane-placeholder`.
- **Automation:** RTL drop on an empty `.board-lane-cards`.
- **Status:** implemented

#### BOARD-13 — Drag into a run-mode step: dispatch with no confirm
- **Given:** an unlinked card in Ready; In Progress is `agent_step`, `on_enter: run`; the default agent binary is on PATH.
- **When:** the user drags the card onto In Progress.
- **Then:** no overlay appears; a worktree + task are provisioned, the card's meta line gains `running` with a filled amber dot, the agent terminal opens with the dispatch packet bracket-pasted, and the app navigates into the task view. `step:launch` fires exactly once for that issue+column.
- **Covers:** ADR-0037 items 2/3; ADR-0032 item 3; step_engine.rs:583-605.
- **Automation:** backend `issue_enter_column` on a run column + assert `EnterOutcome.step == "launched"` and one `step:launch`; the terminal open + paste is a frontend seam (`terminalOpenAgent`/`terminalWrite` mocks).
- **Status:** implemented

#### BOARD-14 — Drag into a queue-mode step: confirm first
- **Given:** In Progress switched to `on_enter: queue` via `column_update` (the ADR's "settings flip"); an unlinked card in Ready.
- **When:** the user drags the card onto In Progress.
- **Then:** the card **has already moved** into In Progress and renders queued (dashed-ring dot, dimmed row, `queued` label); an overlay reads "In Progress runs `<provider · model · effort — trigger>` on `<#id or title>`. Dispatch?" with footer `esc keep in <source column>` / `↵ dispatch[ on <branch>]`. No agent has started.
- **Covers:** ADR-0037 item 3; handoff v3 §8c queue confirm.
- **Automation:** backend `issue_enter_column` on a queue column + assert `step == "queued"` and a `step:queued` event; RTL for the overlay copy.
- **Status:** implemented

#### BOARD-15 — Dismiss the queue confirm
- **Given:** the BOARD-14 overlay is open.
- **When:** the user presses `esc` (or clicks the backdrop, or "esc keep in …").
- **Then:** intended — the overlay closes, the card stays in the queue column and keeps a visible pending-step affordance with a way to fire it. Today the overlay closes, the queued dot disappears, and the backend park survives with no UI: the only route back to the confirm is dragging the card out and back in.
- **Covers:** BoardView.tsx:477-485; step_engine.rs park lifecycle.
- **Automation:** RTL: fire `step:queued`, press Escape, assert the card renders no queued state; then backend `step_confirm` still succeeds — proving the invisible park.
- **Status:** partial (dead end: an invisible, un-fireable parked step)

#### BOARD-16 — Fire the queued step with ↵
- **Given:** the BOARD-14 overlay is open.
- **When:** the user presses `↵` (or clicks the `↵ dispatch` button).
- **Then:** the overlay closes, the queued state clears, the agent launches (worktree provisioned on a first step), the prompt is pasted, and the app navigates into the task. A second `step_confirm` for the same card errors with "no parked step" rather than double-launching.
- **Covers:** ADR-0037 item 3; step_engine.rs:639-677 (atomic park take).
- **Automation:** backend `step_confirm` twice; assert `launched` then the typed `NoParkedStep` error.
- **Status:** implemented

#### BOARD-17 — A later drag supersedes a parked step
- **Given:** a card parked in a queue-mode In Progress with the confirm open.
- **When:** the user presses `esc` and then drags the card into Ready.
- **Then:** the park is dropped, `step:queue_cleared` fires for the old column, and the card sits in Ready with no queued state and no agent ever started.
- **Covers:** ADR-0037 item 3 / step_engine.rs:544-552.
- **Automation:** backend: enter queue column, then `issue_enter_column` on a shelf; assert `step:queue_cleared`.
- **Status:** implemented

---

### The Quick express lane and settle

#### BOARD-18 — Quick end to end: run on drop, advance to Done on settle
- **Given:** the seeded board; an unlinked card in Backlog; `claude` installed.
- **When:** the user drags the card onto Quick and, after the agent finishes, exits the agent CLI (the PTY exits).
- **Then:** (a) no overlay — the agent launches immediately in a fresh worktree; (b) when the PTY exits, the card moves itself into **Done** (not In Progress — Quick's `advance_to` is pinned) and renders dimmed at 50%; (c) any card blocked by it loses its `blocked by` badge on the same refresh.
- **Covers:** ADR-0037 items 4/10; migration 0006 seed; handoff v3 §5.
- **Automation:** backend: `issue_enter_column(quick)` → `settle_issues_for_task(task, Some("pty:x"))` → assert the issue's `column_id`/`lane` is Done and the dependent's `blocked` flipped false.
- **Status:** implemented

#### BOARD-19 — Launch the step's agent with the column's model and effort
- **Given:** Quick pinned to `claude · haiku` (and, say, `step_effort: high` plus a `step_tools` allowlist).
- **When:** a card is dropped onto Quick and the session opens.
- **Then:** intended — the opened agent session runs haiku at the pinned effort with only the allowlisted tools. Today only `provider` is used: `terminal_open_agent(task_id, agent, rows, cols)` takes no model/effort/tools, so the header subline advertises `claude · haiku` while the session runs the provider's default model with unrestricted tools.
- **Covers:** ADR-0037 item 1 (per-step model/effort/tool allowlist); handoff v3 §8a subline contract.
- **Automation:** needs a driver we lack (no command carries model/effort into a PTY agent launch).
- **Status:** not-built

#### BOARD-20 — A hold column leaves the step-done dot for a human drag
- **Given:** a user-created `agent_step` column "Plan" with `on_settle: hold`, and a card whose Plan agent has just exited, with the board view on screen.
- **When:** the settle trigger fires.
- **Then:** the card stays in Plan and gains the accent-filled step-done dot (`run-step-done`), its run label clears, and — where the column declares an artifact — a `↵ read <artifact> · drag on` hint appears.
- **Covers:** ADR-0037 item 4; handoff v3 §8a step-done dot; DESIGN.md "Pipeline board".
- **Automation:** RTL: emit `step:settled` for the card's current column, assert `.run-step-done`. The artifact hint cannot be driven: `stepArtifact()` reads a field no column carries (`lib/columnConfig.ts:82-85`) and always returns null.
- **Status:** partial (dot works while mounted; the artifact hint is not-built)

#### BOARD-21 — The step-done dot survives leaving the board
- **Given:** a card showing the step-done dot in a hold column.
- **When:** the user opens a task (or another project) and comes back to the board, or reloads the webview.
- **Then:** intended — the dot is still there; "this step finished and is waiting for you" is the card's state, not a session artifact. Today `steps` is component state reset on every mount/project switch (`BoardView.tsx:210-216`), so the dot is gone and the card is indistinguishable from one that never ran.
- **Covers:** ADR-0037 item 4 ("step-done is DERIVED state — nothing stored").
- **Automation:** RTL: emit `step:settled`, unmount/remount `BoardView`, assert the dot. Fails today.
- **Status:** not-built

#### BOARD-22 — A settle-chained launch opens its session even when the board is not on screen
- **Given:** a chain — column A (`agent_step`, `on_settle: advance`) advancing into column B (`agent_step`, `on_enter: run`); a card running in A. The user is in the task view (the normal state: every dispatch navigates there).
- **When:** A's agent exits and the engine advances the card into B and emits `step:launch` for B.
- **Then:** intended — B's agent session opens and receives B's prompt. Today `BoardView` is the **only** subscriber to `step:*` and it is unmounted, so nothing opens: the card sits in B with no session, and the launch stays "undelivered" until someone re-enters the column by hand.
- **Covers:** ADR-0037 item 4 ("chains are legal"); step_engine.rs:60-64 (the directive contract).
- **Automation:** RTL: unmount `BoardView`, emit `step:launch`, assert `terminalOpenAgent` was called — needs the listener to live above the board.
- **Status:** not-built

---

### Blocked derivation and the blocked confirm

#### BOARD-23 — Dragging a blocked card onto a step asks, never refuses
- **Given:** card A blocked by card B; B sits in Ready; In Progress is an `agent_step`.
- **When:** the user drags A onto In Progress.
- **Then:** A does **not** move; an overlay reads "`#a` is blocked by `#b`, still in progress. Send to In Progress anyway?" with `esc keep in Ready` / `↵ dispatch <agent>[ on <branch>]`. `esc` leaves A in Ready untouched; `↵` dispatches. Handoff §8c asks for the blocker's **column name** here — the copy hardcodes "still in progress".
- **Covers:** ADR-0032 (confirm, never a hard stop); ADR-0037 item 6; handoff v3 §8c.
- **Automation:** RTL on `ConfirmOverlay` with a blocked `IssueDto`; backend has no gate to assert (any transition is permitted).
- **Status:** partial (copy deviation)

#### BOARD-24 — A blocker reaching Done unblocks its dependents with no writes
- **Given:** A blocked by B; both on the seeded board; B in In Review.
- **When:** the user drags B into Done.
- **Then:** on the next refresh A's meta line loses `blocked by #b` and A's title stops dimming — with no write to A. Dragging B back out of Done re-blocks A.
- **Covers:** ADR-0037 item 6; `issues/mod.rs:250-256` (`BLOCKED_SQL` keys on `counts_as_done`).
- **Automation:** backend `issue_enter_column(done)` then `issue_list`; assert `blocked` flipped on the dependent.
- **Status:** implemented

#### BOARD-25 — A blocker reaching *any* counts-as-done column unblocks its dependents
- **Given:** A blocked by B; a user-created terminal column "Shipped" with `counts_as_done: true` (no `seed_lane`).
- **When:** the user drags B into Shipped.
- **Then:** intended — A unblocks, exactly as it does for Done ("multiple terminal columns are legal"). Today it does not: `BLOCKED_SQL` resolves the blocker's column through `c.seed_lane = b.lane`, and entering a non-seeded column leaves `lane` untouched (`issues/mod.rs:712-728`), so B still resolves to its old lane and A stays blocked forever.
- **Covers:** ADR-0037 item 6 ("any future terminal lane keys off the flag"); MEMORY.md open item "E18-07 authority-flip half".
- **Automation:** backend: create a `counts_as_done` column, `issue_enter_column(blocker → it)`, assert the dependent's `blocked == false`. Fails today.
- **Status:** not-built

#### BOARD-26 — Clicking a blocker reference opens that card
- **Given:** card A shows `blocked by #b` in its meta line.
- **When:** the user clicks `#b`.
- **Then:** the right sheet swaps to card B's detail; the click does not also open A's detail and does not start a drag.
- **Covers:** BoardView.tsx:1074-1097.
- **Automation:** RTL click on `.board-blocked-ref`; assert `ui.boardDetailIssueId === b.id`.
- **Status:** implemented

---

### The board never kills; reattach and the rework loop

#### BOARD-27 — Moving a live card into a terminal column confirms, then moves, and leaves the agent alone
- **Given:** a card in In Progress with a live agent terminal.
- **When:** the user drags it into Done.
- **Then:** an overlay reads "`<card>` has a live agent. Move to Done anyway?" with `esc keep in In Progress` / `move to Done`; on confirm the card renders in Done (dimmed 50%) while its run dot still reads `running`, the task's agent terminal is still alive, and no SIGINT/kill was issued. Handoff §8c also asks the copy to say "The agent keeps running — stopping is ⌘." — it does not.
- **Covers:** ADR-0037 item 11; ADR-0032 item 3; handoff v3 §8c.
- **Automation:** RTL for the overlay; backend assertion that no terminal command is issued on any enter (grep-level/no-call assertion).
- **Status:** partial (copy omits the reassurance the design specifies)

#### BOARD-28 — A live card moved into another agent step gets no warning at all
- **Given:** a card in In Progress with a **live** agent mid-turn.
- **When:** the user drags it onto Quick (`agent_step`, `on_enter: run`).
- **Then:** intended — the user is told a second step is about to run against a task whose agent is still working, and gets a chance to keep it where it is. Today: no confirm (the live-agent gate only fires for `countsAsDone` columns), the engine launches a "new session", and `terminal_open_agent` reattaches to the **already-running** agent (`terminals.rs:519-527`), so Quick's prompt is bracket-pasted into the running agent's stdin mid-turn — under In Progress's provider, not Quick's.
- **Covers:** ADR-0037 item 2 ("a NEW agent session in the same task/worktree"); BoardView.tsx:452-457.
- **Automation:** backend `issue_enter_column` while a live agent terminal exists for the task; assert the returned `launch.reattached == false` yet `terminal_open_agent` returns the existing id.
- **Status:** partial (confirmed defect: cross-step prompt injection into a live session, no confirm)

#### BOARD-29 — The rework loop: drag back from In Review, keep the worktree
- **Given:** a card in In Review whose agent already ran and exited; its linked task and worktree exist.
- **When:** the user drags it back onto In Progress.
- **Then:** no second worktree and no second task are created — the same `linked_task_id` is reused; a fresh agent session opens in that worktree and receives the step's prompt; when it exits, the card advances to In Review again (the settle epoch was reset by the user gesture, so the same task can settle twice).
- **Covers:** ADR-0037 item 2; ADR-0032 item 3; step_engine.rs:286-292 (settle epochs), :687-711.
- **Automation:** backend: enter In Progress, settle, enter In Progress again, settle again; assert one task id throughout and two settles.
- **Status:** implemented — but the re-dispatch prompt is the *original* dispatch packet with no review feedback attached (see gaps).

#### BOARD-30 — Re-entering the card's own column reattaches instead of respawning
- **Given:** a card already resident in In Progress with an opened session.
- **When:** the user re-enters that same column.
- **Then:** intended — the engine reattaches (empty prompt, focus the task), never a second launch. The engine implements this (`step_engine.rs:590-592`), but **no board gesture can reach it**: a same-column drag short-circuits to `reorder` (BoardView.tsx:441-445, :652-660) and ⇧h/⇧l always target a *different* column.
- **Covers:** ADR-0032 item 3; ADR-0037 item 2.
- **Automation:** backend `issue_enter_column` twice on the same column; assert `step == "reattached"`. No UI driver exists.
- **Status:** unreachable-by-mouse

#### BOARD-31 — Two entries into the same column inside the dedupe window
- **Given:** a card in In Progress; the user drags it to Ready and immediately (<4s) back onto In Progress.
- **When:** the second drop lands.
- **Then:** intended — the second entry opens/focuses the step's session and delivers its prompt. Today `claimLaunch` (BoardView.tsx:114-124) suppresses any launch for the same issue+column within 4000ms, so the backend records a launch and emits `step:launch` while the frontend silently does nothing: no terminal, no prompt, no error.
- **Covers:** BoardView.tsx:109-124 (the outcome/event double-delivery workaround).
- **Automation:** RTL with a fake timer: resolve `issueEnterColumn` twice inside 4s, assert `terminalOpenAgent` call count.
- **Status:** partial

#### BOARD-32 — No board gesture ever stops an agent
- **Given:** a card with a live agent.
- **When:** the user drags it through every column in turn (shelf → step → human gate → terminal), including a `counts_as_done` one, confirming where asked.
- **Then:** the agent terminal is alive after every move; the only way to stop it remains ⌘. in the task view / ⌘⌫ teardown.
- **Covers:** ADR-0037 item 11; ADR-0032 item 3.
- **Automation:** backend: assert `settle_issues_for_task`/`enter_column` never touch the terminal manager (call-graph or integration assertion on terminal liveness).
- **Status:** implemented

---

### Keyboard

#### BOARD-33 — j/k walk cards, h/l walk every column including empty ones
- **Given:** the seeded board with cards in Backlog and Done only, focus on a Backlog card.
- **When:** the user presses `j`, `k`, then `l` four times.
- **Then:** `j`/`k` move card focus within Backlog; each `l` advances the **column** cursor one step — Ready, Quick, In Progress, In Review are real stops even though they are empty (card focus goes null there, the head/strip highlight moves), and pressing `l` on the last column does nothing.
- **Covers:** handoff v2 frame 4b; handoff v3 §8b ("h/l walks EVERY column").
- **Automation:** RTL keydown sequence on `window`; assert `.board-card.focused` and the narrow-strip `[data-active]`.
- **Status:** implemented

#### BOARD-34 — ⇧h/⇧l move the card through the same gates
- **Given:** focus on a card in Ready; In Progress is `on_enter: queue`.
- **When:** the user presses `⇧l` twice (Ready → Quick → In Progress) — or `⇧l` once onto a blocked card's step.
- **Then:** each ⇧ move goes through the identical gate the drag uses: Quick (run) dispatches instantly with no confirm; In Progress (queue) parks and opens the confirm; a blocked card opens the blocked confirm and does not move until ↵. ⇧j/⇧k reorder within the column.
- **Covers:** handoff v2 frame 4b; ADR-0037 item 3.
- **Automation:** RTL keydown + mocked `issueEnterColumn`; assert the same call shape as the drop path.
- **Status:** implemented — note that a single ⇧l onto a run-mode step is an un-undoable spend with no confirmation (see gaps).

#### BOARD-35 — ↵ on a card
- **Given:** a card with a linked task that is idle, and (intended) a card whose run ended badly.
- **When:** the user presses `↵` with the card focused.
- **Then:** intended (frame 4a) — a failed card's ↵ reads the linked task (jumps into the agent terminal); every other card opens the detail sheet. Today the read branch is unreachable: it keys on `task.status === "failed"`, and `update_status` has zero production callers (`fartcode-core/src/tasks/mod.rs:376`) — every task is born `in_progress` and stays there. `↵` therefore always opens the detail sheet, and the board has no key that reaches a live agent session.
- **Covers:** handoff v2 frame 4a; ADR-0037 consequences ("TaskStatus beyond in_progress stays dead").
- **Automation:** RTL with a mocked task whose status is `failed` proves the branch; production reachability cannot be driven.
- **Status:** partial (the branch exists; the state is unreachable-entirely)

#### BOARD-36 — `a` adds a card to the landing column
- **Given:** the board with cards.
- **When:** the user presses `a`, types a title, presses `↵`, then presses `esc`.
- **Then:** an inline hairline input appears above the frame with the mono footer `↵ add · esc cancel`; `↵` creates the card in the landing column (whichever column carries the flag), opens its detail in the right sheet, and clears the input **without closing it** (ready for the next title); `esc` closes the row.
- **Covers:** ADR-0037 item 7; handoff v3 §8a.
- **Automation:** RTL: keydown `a`, type, Enter; assert `issueCreate` then `issueEnterColumn(landing)` and `ui.boardDetailIssueId`.
- **Status:** implemented

#### BOARD-37 — The confirm overlay owns ↵ and esc while it is open
- **Given:** any board confirm overlay is open and a card is focused behind it.
- **When:** the user presses `j`, then `esc`.
- **Then:** `j` does nothing (focus does not move behind the overlay); `esc` resolves the overlay only. Typing in the add-issue input never triggers board keys.
- **Covers:** BoardView.tsx:537-552 (`isEditableTarget` + pending swallow).
- **Automation:** RTL keydown while `pending` is set.
- **Status:** implemented

---

### Narrow / laptop layout

#### BOARD-38 — Under 900px the board becomes a strip plus one column
- **Given:** the board pane resized below 900px (e.g. the changes/chat sheet open on a laptop).
- **When:** the user looks at the board and presses `l`.
- **Then:** a horizontally scrolling mono strip lists **every** column as `<name> <count>` in lowercase (never truncated or capped), the focused entry is underlined in accent, entries whose column holds a live agent render `--fc-working`; only the focused column's cards render below, with its spend subline under the strip and the footer `h l walk every column · strip follows focus`. `l` moves focus and the strip auto-scrolls (instantly) to keep it visible.
- **Covers:** handoff v3 §8b; ADR-0037 consequences ("narrow mode scrolls, never caps").
- **Automation:** RTL with a mocked `ResizeObserver` reporting <900px; assert `.board-strip-entry` count == column count and `[data-active]` follows `l`.
- **Status:** implemented

#### BOARD-39 — Move a card between columns in narrow mode
- **Given:** the board under 900px, a card in the focused column.
- **When:** the user drags the card with the mouse toward another column.
- **Then:** intended — there is a mouse path to move it (drop on a strip entry, or an explicit move affordance). Today only the focused column renders; strip entries are plain buttons with no drop handlers, so the only way to move a card across columns on a laptop is ⇧h/⇧l.
- **Covers:** handoff v3 §8b (silent on cross-column moves in narrow mode).
- **Automation:** RTL: `drop` on `.board-strip-entry`; asserts nothing happens today.
- **Status:** not-built

---

### Persistence, restart, concurrency

#### BOARD-40 — Column residence survives a restart
- **Given:** cards distributed across the six seeded columns, including one in Quick (a non-seeded column).
- **When:** the user quits and relaunches the app and reopens the project.
- **Then:** every card renders in the same column it was left in — the Quick card included, because display resolves through the `column_id` mirror before the `seed_lane` fallback.
- **Covers:** `lib/columnConfig.ts:95-105`; `issues/mod.rs:669-737`.
- **Automation:** backend `issue_enter_column` then a fresh store read; assert `column_id`.
- **Status:** implemented

#### BOARD-41 — A pending confirm across a restart
- **Given:** a card parked in a queue-mode step with the confirm open.
- **When:** the app restarts and the user reopens the project.
- **Then:** intended — the pending confirm is either restored or the card visibly carries "waiting on you to dispatch". Today parks are memory-only: the card sits in the queue column with no queued dot, no overlay, and no way to fire the step except dragging it out and back in. (The engine only re-parks when a *settle trigger* arrives — step_engine.rs:242-258 — which the user cannot cause.)
- **Covers:** step_engine.rs:66-72 restart contract; ADR-0037 item 3.
- **Automation:** backend: park, drop the engine state, then `step_confirm` → assert `NoParkedStep`; UI restoration needs a driver we lack.
- **Status:** not-built

#### BOARD-42 — Two live cards settle independently
- **Given:** two cards each dispatched into In Progress, each with its own task and its own agent terminal.
- **When:** the first agent's PTY exits.
- **Then:** only that card advances to In Review; the second stays in In Progress with its dot still running. A second, stale settle trigger carrying the already-consumed session identity moves nothing.
- **Covers:** step_engine.rs:18-52 (session-scoped settle, tombstones).
- **Automation:** backend `settle_issues_for_task(task_a, Some("pty:a"))` twice + once for task_b; assert settle counts.
- **Status:** implemented

#### BOARD-43 — Switching projects does not leak board state
- **Given:** project P1's board with a queued card and a step-done dot; project P2 also open.
- **When:** the user clicks P2's rail tile and then returns to P1.
- **Then:** P2's board renders only P2's columns and cards (no P1 counts, no P1 overlay). Returning to P1 shows the cards in their correct columns — but every derived step flag (queued dot, step-done dot) is gone, because `steps` is cleared on every project change.
- **Covers:** BoardView.tsx:210-216; store/columns.ts per-project cache.
- **Automation:** RTL: mount with P1, emit `step:settled`, remount with P2 then P1; assert the dot's absence.
- **Status:** partial (same root cause as BOARD-21)

#### BOARD-44 — Deleting the linked task frees the card
- **Given:** a card in In Progress linked to a task; the user deletes the task with ⌘⌫.
- **When:** the deletion completes.
- **Then:** the board refreshes on `task:deleted`; the card keeps its column but loses its run dot and elapsed meta, and the next drop onto a step provisions a **fresh** worktree rather than erroring on a dead link.
- **Covers:** ADR-0023; `issues.linked_task_id` FK `ON DELETE SET NULL`; step_engine.rs:696-711.
- **Automation:** backend `delete_task` then `issue_enter_column`; assert a new task id.
- **Status:** implemented

#### BOARD-45 — Elapsed on a running card
- **Given:** a card dispatched 4 minutes ago whose agent is still running.
- **When:** the user reads its meta line (refreshed on a 30s tick).
- **Then:** intended — `running · 4m` counts from when the run started. Today it derives from `task.statusChangedAt`, which is stamped at task creation and never moves (status never changes), so on a reworked card it reads the age of the task, not of the current step.
- **Covers:** `runState.elapsedShort`; ADR-0037 consequences on `TaskStatus`.
- **Automation:** RTL with a fixed `statusChangedAt` and a fake clock.
- **Status:** partial

---

### Editing the pipeline

#### BOARD-46 — Edit a column's config
- **Given:** a project whose Quick column should run a different model, or a team that wants a "Plan" step before "Implement".
- **When:** the user goes looking for where to change it.
- **Then:** intended (handoff §8d) — settings' project row gains a `Columns` child: collapsed rows with a drag handle, name, and the **same** config summary string the board headers use; expanded rows edit kind/runs/on-enter/on-settle/counts-as-done/tools/system prompt; `delete column` renders as a disabled label with "N cards live here — move them first" until the column empties; "Add column" appends a shelf named "New column". Today none of this exists: `columnCreate`/`columnUpdate`/`columnDelete`/`columnReorder` are declared in `lib/tauri.ts:1297-1357` with **zero callers**, and `ProjectSettings.tsx` has no Columns section, so the entire configurable-pipeline premise is reachable only from a devtools console.
- **Covers:** ADR-0037 item 9; handoff v3 §8d.
- **Automation:** RTL once the editor exists; today only the backend commands are testable.
- **Status:** not-built

#### BOARD-47 — Refuse to make the landing column an agent step
- **Given:** the seeded board (Backlog is landing).
- **When:** a caller sets Backlog's kind to `agent_step`, or sets `is_landing` on Quick.
- **Then:** both are rejected with a typed error naming the reason ("entry paths create cards directly and never fire on_enter…"); the board is unchanged. A 50-issue GitHub import can therefore never launch 50 agents.
- **Covers:** ADR-0037 item 7 (amended); `issues/columns.rs:491-502`.
- **Automation:** backend `column_update` both directions; assert `InvalidBoardColumnInput`.
- **Status:** implemented (backend only — no UI can reach the error, which is the design intent per §8d)

#### BOARD-48 — Refuse to delete an occupied column, and the seeded steps
- **Given:** Ready holds two cards; In Progress is a seeded `agent_step`.
- **When:** a caller deletes Ready, then In Progress, then the landing column.
- **Then:** all three are refused — `BoardColumnHasIssues { count: 2 }`, "seeded agent step … until columns become authoritative (E18-07)", and "move is_landing to another column" respectively; deleting an *empty* non-seeded column succeeds and compacts the remaining positions to 0..n-1.
- **Covers:** `issues/columns.rs:766-816`; MEMORY.md (temporary seeded-step delete guard).
- **Automation:** backend `column_delete` in all four shapes.
- **Status:** implemented

---

## 3 · PM chat, PRDs, and issue proposals

The PM chat is one persistent project-scoped ACP conversation (`get_or_create_project_conversation`) rendered by `ProjectChatPanel` inside the right sheet, sharing that sheet with Changes and card detail — only one shows at a time. Every send carries `buildPmPrompt(columns)` as `hiddenContext`, so the agent is told to grill one question at a time, write `docs/prds/<slug>.md` with its own file tools, and emit a fenced `fartCode-proposal` JSON block; `TranscriptItems.MessageRow` scans assistant text for `fartCode-proposal` and `fartCode-ticket-edit` fences and swaps each for an approval card, with everything else degrading to plain prose. Approve goes through `issue_parse_proposal` (validation) and `issue_apply_proposal` (all-or-nothing create + blocked-by edges, landing on the project's `is_landing` column).

Everything below was checked against `app-frontend/src/components/projectChat/*`, `ConversationView.tsx`, `TranscriptItems.tsx`, `lib/proposal.ts`, `lib/ticketEdit.ts`, `components/board/CardDetail.tsx`, `lib/commands.ts`, `store/ui.ts`, `store/conversations.ts`, `fartcode-core/src/issue_proposal.rs`, and `fartcode-app/src/commands/{issue_proposals,conversations,issues}.rs`.

---

### Opening, closing, and panel lifecycle

#### PM-01 — Open the PM chat on a project with the sheet closed
- **Given:** A project is selected, no task selected, the right sheet is closed (`changesOpen: false`, `store/ui.ts:104`), `projectChatOpen` defaults to `true` (`store/ui.ts:105`).
- **When:** The user presses `⌘⇧2`.
- **Then:** A 400px right panel appears with a 46px header reading `PM` on the left and `project root · ⌘⇧2` in mono on the right; the body shows either the hero (`What should we build?` / "Describe a feature — the agent pins down scope, writes a PRD, and proposes issues for the board.") or the restored transcript; the composer placeholder reads `plan something…`.
- **Covers:** ADR-0032 §8; design_handoff_v2 §5c; `commands.ts:224-238`.
- **Automation:** RTL: render `<App/>` with a stubbed `listProviders`/`get_or_create_project_conversation`, dispatch the `⌘⇧2` keydown, assert the header text and the placeholder.
- **Status:** implemented

#### PM-02 — `⌘⇧2` from chat mode closes the whole sheet
- **Given:** The PM chat is visible.
- **When:** The user presses `⌘⇧2` again.
- **Then:** The entire right sheet disappears (board reclaims the width). Pressing `⌘⇧2` once more brings the chat back with its transcript intact.
- **Covers:** `commands.ts:229-237` (`changesOpen` false, `projectChatOpen` stays true).
- **Automation:** RTL component test on the command + `useUi` state, asserting the aside is unmounted.
- **Status:** implemented

#### PM-03 — `⌘⇧1` from chat mode swaps to Changes rather than closing
- **Given:** The PM chat is visible.
- **When:** The user presses `⌘⇧1`.
- **Then:** The same 400px sheet now shows the Changes surface (header `Changes`), not the chat; the PM transcript is not destroyed — `⌘⇧2` restores it with the same messages.
- **Covers:** `commands.ts:166-184`.
- **Automation:** RTL component test.
- **Status:** implemented

#### PM-04 — The PM panel cannot be closed or minimized with the mouse
- **Given:** The PM chat is visible on a project.
- **When:** The user looks for a close/minimize control in the panel header, or anywhere in the project chrome.
- **Then:** *(intended)* A chevron minimize button sits at the right of the `PM` header, mirroring the task chat's, and hides the sheet. *(actual)* The header renders only `PM` and the mono scope text; `.project-chat-minimize` is styled in `project-chat.css:39` and used by `TaskChatPanel.tsx:47-56`, but `ProjectChatPanel.tsx:48-51` never renders it. There is no project header either (`ProjectHeader.tsx` is deleted; `App.tsx` renders no header). The only exits are `⌘⇧2`, `⌘⇧1`, opening a card, or the command palette.
- **Covers:** MEMORY.md ("PM panel has a minimize button (⌘⇧2 toggles back)"); design_handoff_v2 §5c.
- **Automation:** RTL: render `ProjectChatPanel`, assert `getByLabelText(/hide pm/i)` — currently fails.
- **Status:** unreachable-by-mouse

#### PM-05 — A project whose repository workspace never provisioned has no PM chat at all
- **Given:** A project row whose `repositoryWorkspaceId` is `null` (`ensure_repository_workspace` is non-fatal in the create flow — `projects/provider.rs:64-70` — so a failed provision leaves it null), no task selected.
- **When:** The user presses `⌘⇧2`.
- **Then:** *(intended)* The PM chat opens; the PM agent runs in the project root, which needs no workspace row (`acp_runtime.rs:325-328` resolves project-scoped cwd from the project path). *(actual)* `ChangesSidebar.tsx:102` returns `null` for `!taskId && !workspaceId`, so the sheet never renders and nothing happens on the keypress — no panel, no error, no explanation.
- **Covers:** ADR-0032 §8.
- **Automation:** Backend: create a project with `repository_workspace_id` NULL, then RTL-render the shell and dispatch `⌘⇧2`; assert the aside exists.
- **Status:** not-built (dead end)

#### PM-06 — Switching projects while the chat is open bleeds the previous project's transcript
- **Given:** Project A's PM chat is open with several turns of history; project B also exists.
- **When:** The user clicks project B in the rail.
- **Then:** *(intended)* The panel clears and shows project B's own conversation (or its hero). *(actual)* `ProjectChatPanel.tsx:17` keeps `conversationId` in state and only overwrites it on the new success (`:36`), so between the click and the resolved `ensureProject`, `ConversationView` renders with `ownerKey = project:B` but `conversationId = <A's conversation>` — project A's messages are visible under project B. If B's start *fails*, the error paragraph renders **and** A's transcript stays rendered indefinitely (`:52-54` are independent). In that state a send goes to A's conversation while `ProposalCard`'s `projectId` (derived from `ownerKey`) is B — approving would write B's board from A's proposal.
- **Covers:** —
- **Automation:** RTL: mount `ProjectChatPanel` with `projectId=A`, resolve, rerender with `projectId=B` and a rejecting `ensureProject`; assert A's transcript is gone.
- **Status:** partial (bug)

#### PM-07 — The ACP adapter is not installed
- **Given:** `claude-agent-acp` is not on PATH.
- **When:** The user opens the PM chat.
- **Then:** The panel renders the header plus a single error line, `ACP adapter binary not found on PATH: claude-agent-acp — install with: npm i -g @agentclientprotocol/claude-agent-acp` (`acp_runtime.rs:62-71`, surfaced by `ProjectChatPanel.tsx:52`). No composer, no retry button, no link to Settings → provider accounts; the only recovery is installing the binary and toggling the panel closed and open.
- **Covers:** —
- **Automation:** Backend: point `default_adapter_resolver` at an empty PATH; RTL assert the error text; assert no retry control exists (gap).
- **Status:** partial (no retry affordance)

#### PM-08 — Session stopped, then a prompt restarts it
- **Given:** The PM conversation's session lifecycle is `closed` (agent process exited).
- **When:** The user types into the composer and presses Enter.
- **Then:** Before the send, the dock shows `Session stopped — sending a prompt starts it again.`; after the send, the session restarts (`store/conversations.ts:123-125` calls `acpStart` first) and the turn runs; the notice clears when the snapshot reports a live session.
- **Covers:** `ConversationView.tsx:322-326`.
- **Automation:** Backend `acp_stop` + assert `acp:transcript` snapshot lifecycle, then `acp_send_prompt` and assert a new turn.
- **Status:** implemented

---

### Grilling, PRD authoring, and the prompt contract

#### PM-09 — The PM system prompt rides hidden on every send and never appears in the transcript
- **Given:** The PM chat is open on a project with the seeded six columns.
- **When:** The user sends "add oauth login".
- **Then:** The transcript shows exactly one right-aligned user bubble reading `add oauth login`; the PM prompt text (`You are the project manager for this repository…`) appears nowhere, on this turn or after a `session/load` replay (suppressed by the `[fartCode:hidden-context]` sentinel, `cell.rs:871-884`). The agent's reply asks **one** question with a recommended answer.
- **Covers:** ADR-0032 §5; `ConversationView.tsx:202-212`.
- **Automation:** Backend: `acp_send_prompt` with `hiddenContext`, assert the recorded prompt carries two blocks and the reduced transcript's user item carries only the visible text. The "one question at a time" half is model behavior — not falsifiable by assertion.
- **Status:** implemented

#### PM-10 — Board prose in the prompt names the project's actual columns
- **Given:** A project whose landing column has been renamed from `Backlog` to `Inbox` and whose `in_progress` mirror is named `Build`.
- **When:** The user sends any prompt from the PM chat.
- **Then:** The hidden context's last bullet reads `After the owner approves, the issues appear on the board in the Inbox column. Work proceeds when they drag cards to Build.`
- **Covers:** ADR-0037; E18-06; `pmPrompt.ts:32-43`.
- **Automation:** Already covered by `pmPrompt.test.ts` (`buildPmPrompt`); the end-to-end half needs `acp_send_prompt` to expose the hidden block for assertion.
- **Status:** implemented

#### PM-11 — Column config unreadable degrades the prompt, never blocks the send
- **Given:** `column_list` fails for the project (DB locked / migration mid-flight).
- **When:** The user sends a prompt.
- **Then:** The send still goes out; the hidden context's last bullet reads `After the owner approves, the issues appear on the board. Work proceeds when they drag cards to an agent column.` and contains no literal `undefined`.
- **Covers:** `pmPrompt.ts:96-103`; `pmPrompt.test.ts` ("names no column when the project's columns cannot be read").
- **Automation:** RTL with a rejecting `columnList` stub.
- **Status:** implemented

#### PM-12 — Writing the PRD raises a permission prompt in the chat dock
- **Given:** The PM chat is open; the agent decides to write `docs/prds/oauth-login.md`.
- **When:** The agent issues its write tool call.
- **Then:** A permission band docks above the composer reading `Allow <tool title>?` with the adapter's option buttons (allow variants styled primary, reject variants danger); a second queued request shows `(1 of 2)`. Choosing an option resolves it and the band disappears. The file lands in the **project root checkout**, so it also appears as an unstaged change in the Changes surface.
- **Covers:** ADR-0032 §6; `ConversationView.tsx:69-118, 316-318`.
- **Automation:** Backend: drive an ACP fixture adapter that requests permission; assert `acp:transcript` carries `pendingPermissions` and that `acp_resolve_permission` clears it.
- **Status:** implemented

#### PM-13 — The PRD the agent just wrote cannot be opened from the app
- **Given:** The agent has written `docs/prds/oauth-login.md` and emitted a proposal referencing it.
- **When:** The user clicks the PRD filename in the proposal card header, or the `docs/prds/oauth-login.md` mention in the agent's prose, or the `PRD` row in a resulting card's detail sheet.
- **Then:** *(intended)* The PRD opens in a readable surface. *(actual)* Nothing happens anywhere: the card header span is a `title`-tooltip only (`ProposalCard.tsx:177-184`), prose mentions are deliberately inert (`project-chat.css:123-129`, `cursor: default`), and card detail renders the path as bare `<code>` (`CardDetail.tsx:496-503`). The PRD is only readable outside fartCode.
- **Covers:** ADR-0032 §6.
- **Automation:** RTL: click each of the three surfaces, assert no navigation/open call — documents the gap.
- **Status:** not-built

#### PM-14 — Stopping mid-proposal leaves a half-written fence as raw text forever
- **Given:** The agent is streaming a `fartCode-proposal` block and has emitted the opening fence and part of the JSON.
- **When:** The user clicks **Stop** in the composer.
- **Then:** The turn settles as `Turn cancelled`; the message keeps the unterminated fence, so `extractProposalBlocks` (which requires a closing ```` ``` ````, `lib/proposal.ts:7`) finds nothing and the partial JSON renders as plain prose in the transcript permanently. There is no "retry this block" affordance and no way to delete the message.
- **Covers:** `ConversationView.tsx:216-219`.
- **Automation:** RTL: render `MessageRow` with an unterminated fence and a `proposalProjectId`; assert no `[role=group][aria-label=Proposal]` and that the raw text is visible.
- **Status:** partial

---

### Proposal block → approval card

#### PM-15 — A well-formed proposal block renders as a card
- **Given:** The PM chat has an assistant message containing a fenced `fartCode-proposal` block with `prd.path = docs/prds/oauth.md` and three issues, the third `blockedBy` the first.
- **When:** The message settles.
- **Then:** The prose before/after the fence renders as normal agent text, and in its place a card appears with a mono uppercase `Proposal` label, `oauth.md` on the right, three numbered rows (`1 2 3`) with titles, `blocked by 1` right-aligned on row 3, a footer reading `e edit · x drop`, and a button `↵ approve 3 → Backlog`.
- **Covers:** ADR-0032 §5; design_handoff_v2 §5c; `TranscriptItems.tsx:152-170`.
- **Automation:** RTL with `issue_parse_proposal` stubbed to the real Rust result shape.
- **Status:** implemented

#### PM-16 — A malformed proposal block degrades to plain text and never throws
- **Given:** The agent emits a `fartCode-proposal` fence containing `{"issues": []}` (or trailing-comma JSON, or an issue with a blank title, or duplicate titles).
- **When:** The message renders.
- **Then:** After a brief `Parsing proposal…` line the block renders as a `<pre>` of the raw payload; no card, no approve button, no console exception, and the rest of the transcript keeps rendering.
- **Covers:** ADR-0032 consequences ("malformed blocks must surface as plain transcript text, never throw"); `issue_proposal.rs:60-84`; `ProposalCard.tsx:48-60`.
- **Automation:** Rust: `parse_failures_err_never_panic` already covers the parser. Frontend: RTL with a rejecting `issue_parse_proposal`.
- **Status:** implemented

#### PM-17 — Two proposal blocks in one message render as two independent cards
- **Given:** One assistant message carries two `fartCode-proposal` fences (the prompt says "exactly ONE" but nothing enforces it).
- **When:** The message renders.
- **Then:** Two cards stack under one prose block, each with its own focus/drop/approve state. Approving both creates both sets; a `blockedBy` in card 2 naming a title from card 1 resolves **only if card 1 was approved first** (then it matches an existing board issue), and errors out with `blockedBy unknown title` if card 2 is approved first.
- **Covers:** `TranscriptItems.tsx:162-164`; `issue_proposal.rs:138-147`.
- **Automation:** RTL for the two-card render; backend `issue_apply_proposal` twice in each order for the resolution assertion.
- **Status:** implemented (order-dependent, unspecified by design)

#### PM-18 — Focus, walk, and the missing focus ring
- **Given:** A three-row proposal card is on screen; nothing in the card is focused.
- **When:** The user clicks row 2, then presses `j`, then `k` twice.
- **Then:** Row 2 gets the `focused` tint, then row 3, then rows 2 and 1. The card element itself takes DOM focus (`ProposalCard.tsx:196-199`) but shows **no** focus ring (`project-chat.css:301-303` sets `outline: none`), so before the first click there is nothing telling the user the `e`/`x`/`↵` keys in the footer are live.
- **Covers:** design_handoff_v2 §5c.
- **Automation:** RTL: `userEvent.click(row2)`, `keyboard('{j}')`, assert `.focused` moves.
- **Status:** partial

#### PM-19 — `e` edits a title inline and Enter commits
- **Given:** Row 2 is focused.
- **When:** The user presses `e`, types a new title, presses Enter.
- **Then:** The row swaps to a text input pre-selected with the old title; on Enter the row shows the new title and keyboard focus returns to the card. Double-clicking a row does the same thing with the mouse.
- **Covers:** ADR-0032 §5 ("edit titles"); `ProposalCard.tsx:75-103, 200`.
- **Automation:** RTL component test.
- **Status:** implemented

#### PM-20 — Escape cancels an edit; blur commits it
- **Given:** Row 2 is in edit mode with modified text.
- **When:** (a) The user presses Escape. (b) In a second run, the user clicks elsewhere in the card instead.
- **Then:** (a) The row reverts to the original title. (b) The row keeps the typed title (blur commits). A blur arriving after an Escape must not resurrect the edit — the `editingRef` mirror exists for exactly this race (`ProposalCard.tsx:44-46`).
- **Covers:** `ProposalCard.tsx:210-222`.
- **Automation:** RTL component test, including a `blur` fired after `Escape`.
- **Status:** implemented

#### PM-21 — Renaming a row rewrites the blocked-by edges that pointed at it
- **Given:** Row 3 shows `blocked by 1`; row 1 is titled `Token storage`.
- **When:** The user renames row 1 to `Token vault` and approves.
- **Then:** Row 3 still shows `blocked by 1` (never `blocked by Token storage` as a stray literal), and after approve the created issue for row 3 has a real dependency edge on the created `Token vault` issue — not an `blockedBy unknown title` error.
- **Covers:** `ProposalCard.tsx:96-102`; `issue_proposal.rs:131-136`.
- **Automation:** RTL for the note; backend `issue_apply_proposal` + `issue_get` asserting `blockers[0].title == "Token vault"`.
- **Status:** implemented

#### PM-22 — Renaming a row to duplicate another row's title is accepted and silently collapses an edge
- **Given:** A proposal with rows `Token storage` (1), `Middleware` (2, `blockedBy` `Token storage`), and `Docs` (3).
- **When:** The user renames row 3 to `Token storage` and approves.
- **Then:** *(intended)* The card refuses the rename or the approve, because `parse_proposal` treats duplicate titles as fatal (`issue_proposal.rs:74-82`). *(actual)* `issue_apply_proposal` never re-runs `parse_proposal` (`fartcode-app/src/commands/issue_proposals.rs:28`), so both issues are created with the same title and `own_titles` (a HashMap, `issue_proposal.rs:131`) keeps only the last — row 2's edge silently points at `Docs` instead of `Token storage`. No error, no warning.
- **Covers:** ADR-0032 (titles are the join key).
- **Automation:** Backend: call `issue_apply_proposal` with a hand-built duplicate-title `Proposal`; assert two issues and the wrong edge.
- **Status:** partial (bug)

#### PM-23 — `x` drops a row, and dropping it removes edges pointing at it
- **Given:** A three-row card; row 3 shows `blocked by 1`.
- **When:** The user focuses row 1 and presses `x`.
- **Then:** Row 1 stays visible, struck through at reduced opacity with the note `dropped`; row 3's note disappears entirely (its only blocker is gone); the approve button re-reads `↵ approve 2 → Backlog`. Pressing `x` again on row 1 restores it and row 3's `blocked by 1` returns.
- **Covers:** ADR-0032 §5 ("drop issues"); `ProposalCard.tsx:105-112, 134-145`.
- **Automation:** RTL component test.
- **Status:** implemented

#### PM-24 — Dropping a row is keyboard-only
- **Given:** A proposal card, user working with the mouse.
- **When:** The user tries to drop row 2 without touching the keyboard.
- **Then:** *(intended)* A per-row drop affordance (a hover-revealed `×`, per the footer's key legend having a mouse twin). *(actual)* There is none: click focuses, double-click edits, and `x` is the only drop path (`ProposalCard.tsx:156-157`). Same for approve navigation — `j`/`k`/`↑`/`↓` have no mouse twin beyond clicking a row.
- **Covers:** design_handoff_v2 §5c (footer legend `e edit · x drop`).
- **Automation:** RTL: assert no button inside `.proposal-row`.
- **Status:** unreachable-by-mouse

#### PM-25 — Dropping every row disables approve
- **Given:** A two-row card.
- **When:** The user presses `x` on both rows.
- **Then:** Both rows are struck through, the button reads `approve 0 → Backlog` and is disabled; pressing Enter does nothing. Un-dropping either row re-enables it.
- **Covers:** `ProposalCard.tsx:114-115, 238`.
- **Automation:** RTL component test.
- **Status:** implemented

#### PM-26 — The card never shows the body or acceptance criteria being approved
- **Given:** A proposal whose issues each carry a body and 3-4 acceptance criteria.
- **When:** The user reviews the card before approving.
- **Then:** *(intended)* The approval gate shows what is about to be written — at minimum an expandable row revealing body + acceptance. *(actual)* Only titles and blocked-by notes render (`ProposalCard.tsx:186-232`); the user approves bodies and acceptance criteria sight-unseen and can only read them afterwards by opening each created card's detail sheet.
- **Covers:** ADR-0032 §5 ("hard human gate").
- **Automation:** RTL: assert the card DOM contains no body/acceptance text from the parsed proposal.
- **Status:** not-built

---

### Approving

#### PM-27 — Approve writes the issues and lands them on the landing column
- **Given:** A three-row card, none dropped, on a project with the seeded board (`Backlog` flagged `is_landing`).
- **When:** The user clicks `↵ approve 3 → Backlog` (or presses Enter with the card focused).
- **Then:** The button shows `applying…` and is disabled; on success the whole card is replaced by `✓ 3 issues added to Backlog`; three cards appear in the board's Backlog column without a manual refresh (driven by `issue:created`), each carrying `prdPath = docs/prds/oauth.md`, and the blocked row shows the blocked treatment on the board.
- **Covers:** ADR-0032 §5; ADR-0037 §7; `issue_proposal.rs:107-160`; `BoardView.tsx:234-236`.
- **Automation:** Backend command `issue_apply_proposal` + assert `issue:created` events and `issue_list` contents; RTL for the card's applied state.
- **Status:** implemented

#### PM-28 — The approve button lies when the landing column is not named Backlog
- **Given:** A project whose landing flag has been moved to a column named `Triage`.
- **When:** The user opens a proposal card and approves.
- **Then:** *(intended)* The button reads `↵ approve 3 → Triage` and the applied line reads `✓ 3 issues added to Triage` — design_handoff_v3 README says the `"approve N → <column>"` copy generates from column config. *(actual)* Both strings hardcode `Backlog` (`ProposalCard.tsx:67` and `:242`) while the backend correctly lands the rows on `Triage` (`issue_proposal.rs:119-121`, test `apply_lands_on_a_moved_landing_column`). The card tells the user the wrong destination.
- **Covers:** design_handoff_v3 README "Migration/seed notes"; ADR-0037 §7.
- **Automation:** RTL: render the card with a columns store whose landing column is `Triage`; assert the button text — currently fails.
- **Status:** partial (bug)

#### PM-29 — Approve is all-or-nothing when a blocker title is unknown
- **Given:** A proposal where row 2 is `blockedBy` a title that exists neither in the proposal nor on the board (e.g. the user renamed row 1 but the block referenced the old name from outside the proposal).
- **When:** The user approves.
- **Then:** No cards appear on the board at all (row 1 was created then rolled back, `issue_proposal.rs:96-104`); the card stays in its editable state and shows an inline error line containing `is blockedBy unknown title`; the user can drop the offending row and approve again.
- **Covers:** `issue_proposal.rs:89-105`; `ProposalCard.tsx:233`.
- **Automation:** Rust test `apply_unknown_blocker_rolls_back_everything` covers the backend; RTL with a rejecting `issue_apply_proposal` for the card.
- **Status:** implemented

#### PM-30 — A dependency cycle inside a proposal rolls back
- **Given:** A proposal where X is `blockedBy` Y and Y is `blockedBy` X.
- **When:** The user approves.
- **Then:** The board gains nothing; the card shows the cycle error inline.
- **Covers:** ADR-0032 §1; Rust test `apply_cycle_within_proposal_rolls_back`.
- **Automation:** Backend command test + RTL error assertion.
- **Status:** implemented

#### PM-31 — Approving twice creates duplicate issues (no idempotence, no undo)
- **Given:** A proposal card the user has already approved — it reads `✓ 3 issues added to Backlog`.
- **When:** The user presses `⌘⇧1` then `⌘⇧2` (or clicks a board card and closes the detail, or the streaming turn settles under them — see PM-32), then clicks approve on the same card again.
- **Then:** *(intended)* The card stays applied — the approval is recorded against the transcript. *(actual)* `ProposalCard` keeps `state` in component-local `useState` (`ProposalCard.tsx:36-41`) with nothing persisted, so any remount restores the approvable card and a second approve writes **three more identical issues**. `apply_proposal` has no dedupe. There is also no undo for an approve: the only reversal is deleting each created card one at a time from card detail.
- **Covers:** ADR-0032 §5 ("hard human gate").
- **Automation:** RTL: render, approve, unmount, remount with the same `raw`, assert the applied state — currently fails. Backend: call `issue_apply_proposal` twice, assert 6 rows.
- **Status:** partial (bug)

#### PM-32 — Approving while the turn is still streaming loses the applied state when it settles
- **Given:** The agent has finished emitting the proposal fence but the turn is still generating (trailing prose, or another tool call).
- **When:** The user approves immediately, sees `✓ 3 issues added to Backlog`, and waits for the turn to end.
- **Then:** *(intended)* The applied state survives. *(actual)* The active turn renders through `TurnBlock` and settled turns through `SettledTurn = memo(TurnBlock)` (`TranscriptItems.tsx:462-502`) — different element types, so React unmounts and remounts the subtree at settle. The card reverts to the approvable state with the applied write invisible, inviting the duplicate write in PM-31.
- **Covers:** ADR-0031 (two-tier transcript).
- **Automation:** RTL: render `ConversationView` with a live snapshot, approve, then push a snapshot that moves the turn into `committed`; assert the applied text survives — currently fails.
- **Status:** partial (bug)

#### PM-33 — Proposal-card keys leak to the board's window listener
- **Given:** The PM chat is open with a focused proposal card; the user has previously focused a board card with `j`/`k` or a click.
- **When:** The user presses `↵` to approve. Separately: presses `j`, then `a`.
- **Then:** *(intended)* The card owns those keys while it holds focus. *(actual)* `ProposalCard.onKeyDown` calls `preventDefault` but never `stopPropagation` (`ProposalCard.tsx:147-164`), and `BoardView` installs a `window` keydown listener that only bails on editable targets (`BoardView.tsx:537-538, 628`). So `↵` approves **and** opens the board's focused card detail (`.proposal-card` does not match its `.board-card` guard at `:555`) — which swaps the sheet away from the chat and destroys the card's applied state (PM-31). `j`/`k` move both the card's row cursor and the board's card focus. `a` opens the board's inline add-issue composer while the user's focus is in the chat.
- **Covers:** ADR-0037 §8b board keyboard; design_handoff_v2 §5c card keys.
- **Automation:** RTL: render the project view with the chat open, focus a board card, focus the proposal card, press Enter; assert `boardDetailIssueId` stayed null — currently fails.
- **Status:** partial (bug)

#### PM-34 — Blocked-by against an existing board issue resolves by exact title
- **Given:** The board already has an issue titled `Existing schema work` in a non-done column; the PM proposes `Follow-up` with `blockedBy: ["Existing schema work"]`.
- **When:** The user approves.
- **Then:** The card's note reads `blocked by Existing schema work` (an outside title passes through verbatim rather than as a row number, `ProposalCard.tsx:139-143`); after approve, `Follow-up` appears on the board in the blocked treatment with the existing issue as its blocker.
- **Covers:** `issue_proposal.rs:138-147`; Rust test `apply_resolves_blocker_against_existing_issues`.
- **Automation:** Backend command test.
- **Status:** implemented

#### PM-35 — Two same-titled issues already on the board make the edge ambiguous
- **Given:** The board has two issues both titled `Auth cleanup`; a proposal references `blockedBy: ["Auth cleanup"]`.
- **When:** The user approves.
- **Then:** The edge attaches to whichever comes first in board order (`issue_proposal.rs:143-146`, "accepted v1 ambiguity"). Nothing in the card warns the user that the reference is ambiguous, and the resulting edge is not shown before the write.
- **Covers:** `issue_proposal.rs:141-147` (comment).
- **Automation:** Backend: seed two same-titled issues, apply, assert which id got the edge.
- **Status:** implemented (unspecified by design)

---

### Ticket edits from card detail

#### PM-36 — Ask PM from a selection in a card's body
- **Given:** A board card is open in the detail sheet, not in edit mode, and its body has rendered markdown.
- **When:** The user selects a sentence in the body.
- **Then:** An `Ask PM` pill appears anchored under the selection. Clicking it opens a popover showing the excerpt (truncated at 80 chars), a textarea placeholder `Change this part of the ticket…  (Enter sends, ⇧Enter breaks)`, a `→ Project chat` destination label, Cancel, and `Send to PM`.
- **Covers:** `CardDetail.tsx:151-172, 581-630`.
- **Automation:** RTL: set a `window.getSelection` range inside `.card-detail-md`, fire `mouseUp`, assert the pill.
- **Status:** implemented

#### PM-37 — Sending the ask hands off to the chat with the issueId attached
- **Given:** The Ask PM popover is open with a typed question.
- **When:** The user presses Enter (or clicks `Send to PM`).
- **Then:** The button shows `Sending…`; on success the card detail closes and the right sheet swaps to the PM chat, where the user's bubble reads `Ticket "<title>" (issueId: <id>) — selected excerpt:` followed by the fenced excerpt and the question. If the PM is mid-turn, the prompt appears in the queued-prompts strip instead of running immediately.
- **Covers:** `CardDetail.tsx:183-211`; `pmPrompt.ts:68-75`.
- **Automation:** RTL with stubbed `acp_send_prompt`, asserting the composed text and `ui.projectChatOpen === true`.
- **Status:** implemented

#### PM-38 — Ask PM is unreachable without a mouse selection, and a bodyless ticket sends the placeholder
- **Given:** (a) A user driving from the keyboard. (b) An issue whose body is empty, so the detail renders `No description — double-click to add one.` inside the selectable region (`CardDetail.tsx:366-374`).
- **When:** (a) The user tries to reach Ask PM with the keyboard. (b) The user selects the placeholder text and clicks Ask PM.
- **Then:** *(intended)* A keyboard/command path to "ask the PM about this ticket", and no ability to quote UI chrome. *(actual)* (a) There is no command, no button, no chord — the pill only appears from a `mouseup` inside the markdown region. (b) The excerpt sent to the agent is the literal placeholder `No description — double-click to add one.`, which the PM then treats as ticket content.
- **Covers:** —
- **Automation:** (a) Assert no `ask-pm` command in the registry. (b) RTL: select the placeholder node, fire `mouseUp`, assert the composed prompt.
- **Status:** partial / unreachable-by-mouse-only

#### PM-39 — A ticket-edit block renders as an edit card
- **Given:** The PM replied to an ask with a `fartCode-ticket-edit` fence carrying `issueId`, `title: null`, a full replacement `body`, and a 3-item `acceptance`.
- **When:** The message settles.
- **Then:** A card appears labelled `Ticket edit` with the issue's current title on the right, a `Body` section rendering the new markdown, an `Acceptance (3)` list, and a footer with `esc dismiss` and `↵ apply`.
- **Covers:** `TicketEditCard.tsx:81-140`; `pmPrompt.ts:68-75`.
- **Automation:** RTL with a stubbed `issue_list`.
- **Status:** implemented

#### PM-40 — Apply patches the issue and the board reflects it
- **Given:** A ticket-edit card for an issue currently open behind the chat.
- **When:** The user clicks `↵ apply` (or presses Enter with the card focused).
- **Then:** The button shows `applying…`, then the whole card is replaced by `✓ Ticket updated`; the board card's title/body update from the `issue:updated` event without a refresh.
- **Covers:** `TicketEditCard.tsx:65-77`; `issues/mod.rs:521-580`.
- **Automation:** Backend `issue_update` + assert `issue:updated`; RTL for the applied state.
- **Status:** implemented

#### PM-41 — Apply replaces the body wholesale with no diff and no undo
- **Given:** A ticket-edit card whose `body` is the agent's full replacement text.
- **When:** The user reviews before applying.
- **Then:** *(intended)* An old-vs-new view, matching the Title row's `old → new` treatment. *(actual)* Only the **new** body renders (`TicketEditCard.tsx:110-115`) and only the **new** acceptance list (`:117-127`) — the user cannot see what is being destroyed, and after apply there is no undo: the previous body is gone from the DB and only recoverable by retyping it in card detail.
- **Covers:** design_handoff_v2 §5c card shell.
- **Automation:** RTL: assert the card DOM contains the new body but not the issue's current body.
- **Status:** partial

#### PM-42 — Apply is enabled even when the card says it will fail
- **Given:** A ticket-edit block whose `issueId` does not exist on this project's board.
- **When:** The card renders, and the user clicks `↵ apply` anyway.
- **Then:** The card shows `Issue not found on this board — apply will fail.` **and** the apply button stays enabled (`TicketEditCard.tsx:128-138`). Clicking it calls `issue_update` with that id: if the id is genuinely unknown the card shows an `IssueNotFound` error; if the id belongs to **another project's** issue the update **succeeds** — `issue_update` takes no project id and does no ownership check (`fartcode-app/src/commands/issues.rs:89-108`), so project A's PM chat silently edits project B's ticket while the card claims it will fail.
- **Covers:** —
- **Automation:** Backend: create two projects, apply a ticket-edit naming project B's issue from project A's chat, assert B's issue changed.
- **Status:** partial (bug)

#### PM-43 — An all-null ticket-edit block degrades to raw text
- **Given:** The agent copies the schema literally with `title`, `body`, and `acceptance` all `null` (the exact shape `PM_PROMPT_VERSION` 3 was cut to prevent).
- **When:** The message renders.
- **Then:** `parseTicketEdit` returns null (`lib/ticketEdit.ts:50`), the card never appears, and the raw JSON renders as a `<pre>`. Nothing is applied and no error is shown.
- **Covers:** `pmPrompt.ts:17-22`; `pmPrompt.test.ts` ("carries a ticket-edit example the parser accepts").
- **Automation:** RTL: render `TicketEditCard` with an all-null payload, assert the `<pre>`.
- **Status:** implemented

#### PM-44 — An empty acceptance array silently wipes the criteria
- **Given:** A ticket-edit block with `"acceptance": []` and everything else null.
- **When:** The card renders and the user applies.
- **Then:** The card shows `Acceptance (0)` above an empty list — no warning that this **clears** existing criteria — and apply replaces the issue's acceptance with an empty list (`issues/mod.rs:535-537`). The criteria are unrecoverable.
- **Covers:** `lib/ticketEdit.ts:38-43`.
- **Automation:** Backend: seed an issue with 3 criteria, apply the block, assert 0.
- **Status:** partial

#### PM-45 — A no-op title edit renders an empty card
- **Given:** A ticket-edit block whose `title` equals the issue's current title and whose `body`/`acceptance` are null.
- **When:** The card renders.
- **Then:** The title row is suppressed (`TicketEditCard.tsx:103`), leaving a card with a header, no fields at all, and an enabled `↵ apply` that writes the identical title. The user is asked to approve a change with nothing shown.
- **Covers:** —
- **Automation:** RTL component test.
- **Status:** partial

#### PM-46 — Dismiss is irreversible and drops the user into raw JSON
- **Given:** A ticket-edit card on screen.
- **When:** The user clicks `esc dismiss` (or presses Escape with the card focused).
- **Then:** *(intended)* The card collapses to a quiet "dismissed" line, restorable. *(actual)* The card is replaced by a `<pre>` of the raw JSON payload (`TicketEditCard.tsx:53-55`) which stays in the transcript for the rest of the session; there is no way to bring the card back short of asking the PM to re-emit the block.
- **Covers:** design_handoff_v2 §5c footer (`esc dismiss · ↵ apply`).
- **Automation:** RTL: dismiss, assert the `<pre>` and that no re-open control exists.
- **Status:** partial

#### PM-47 — Ticket-edit cards never appear in a task chat
- **Given:** A task view with the task chat panel open (`⌘⇧A`), and an agent reply that happens to contain a `fartCode-ticket-edit` or `fartCode-proposal` fence.
- **When:** The message renders.
- **Then:** No cards appear — the text renders verbatim including the fences, because `proposalProjectId` is only derived from a `project:` owner key (`ConversationView.tsx:198-200`) and `TaskChatPanel` passes the task id. A task agent therefore cannot write board issues through this path.
- **Covers:** ADR-0032 §5 (writes are gated to the PM surface); ADR-0033.
- **Automation:** RTL: render `ConversationView` with `ownerKey = <taskId>` and a proposal fence; assert no `[aria-label=Proposal]`.
- **Status:** implemented

---

### Concurrency, persistence, and layout

#### PM-48 — Two projects each keep their own PM conversation
- **Given:** Projects A and B both have PM history.
- **When:** The user switches A → B → A, sending a prompt in each.
- **Then:** Each project's transcript shows only its own turns; `get_or_create_project_conversation` returns one persistent ACP conversation per project (`fartcode-app/src/commands/conversations.rs:139-180`) and both agent processes stay alive across the switch (no `acp_stop` on unmount), so a turn started in A is still streaming when the user returns. *(Caveat: the transient bleed in PM-06 is visible during the switch itself.)*
- **Covers:** ADR-0032 §8.
- **Automation:** Backend: call `get_or_create_project_conversation` twice per project, assert two distinct stable ids; assert `list_project_conversations` filters by scope.
- **Status:** partial

#### PM-49 — A PM turn keeps running while the panel is hidden
- **Given:** The PM chat is generating a reply.
- **When:** The user presses `⌘⇧1` (swap to Changes), waits, then `⌘⇧2`.
- **Then:** The transcript comes back with the turn's full output including any proposal card — the store keeps receiving `acp:transcript` snapshots at App level (`App.tsx:33`), so nothing is lost. The `Working…` row reflects the live state on return.
- **Covers:** ADR-0031.
- **Automation:** RTL with a snapshot pushed while the panel is unmounted.
- **Status:** implemented

#### PM-50 — Restart restores the PM transcript only if the adapter supports session load
- **Given:** A PM conversation with several turns including an approved proposal; the app is quit and relaunched.
- **When:** The user selects the project and presses `⌘⇧2`.
- **Then:** *(intended, and what happens with an adapter advertising `loadSession`)* `acp_start` resumes via `session/load`, the replayed history rebuilds the transcript, hidden-context blocks stay suppressed, and the proposal block re-renders — **as a fresh approvable card**, which is the PM-31 duplicate-write hazard on the restart path. With an adapter that does not support `session/load`, a brand-new session starts and the panel shows the hero as if the project had never been planned; nothing tells the user the history is gone.
- **Covers:** ADR-0027; `session/manager.rs:107-161`.
- **Automation:** Backend: fixture adapter with `loadSession` true/false; assert `acp_history` after restart in each case.
- **Status:** partial

#### PM-51 — Narrow window: the sheet pins at 400px and the board scrolls
- **Given:** A 1280×800 window, flyout expanded, PM chat open, six seeded columns.
- **When:** The user drags the sheet's gutter handle to the right to shrink the panel.
- **Then:** The panel stops at 400px and will not narrow further, even though `.project-chat` declares `min-width: 280px` (`ChangesSidebar.tsx:105-107` clamps with `Math.max(resize.width, 400)`). The board area keeps its `min-width: calc(var(--column-count) * 150px)` and scrolls horizontally instead of collapsing; the page body itself never scrolls sideways. There are no `@media` breakpoints anywhere in the frontend styles, so nothing else changes at narrow widths.
- **Covers:** ADR-0038 note ("narrow mode SCROLLS, never caps"); DESIGN.md §layout (400px right panel).
- **Automation:** RTL/jsdom cannot measure; needs a real-window driver (Playwright/Tauri webdriver) we do not have wired.
- **Status:** implemented (untestable with current harness)

#### PM-52 — The PM agent is whichever ACP provider is first in the registry
- **Given:** The project's `defaultAgent` setting is set to a non-first ACP-capable provider.
- **When:** The user opens the PM chat.
- **Then:** *(intended)* The PM runs on the project's default agent, and the panel says which. *(actual)* `ProjectChatPanel.tsx:27-29` takes `listProviders().find(p => p.capabilities.includes("acp"))` — the first entry of the static compiled-in registry (`fartcode-providers/src/lib.rs:139`), ignoring `defaultAgent` entirely. Nothing in the panel names the provider or model, and there is no picker. Note the `no ACP-capable provider available` error at `:30` is effectively dead code, since `list_dtos` returns the registry regardless of what is installed.
- **Covers:** ADR-0010; ADR-0037 §step config (per-column provider/model exists for board steps but not for the PM).
- **Automation:** Backend: set `defaultAgent`, then assert the created project conversation's provider — currently mismatches.
- **Status:** partial

#### PM-53 — Context usage is visible but the hidden prompt's cost is not attributed
- **Given:** A long PM conversation.
- **When:** The user looks at the composer.
- **Then:** A `Nk / Mk` context chip renders when the adapter reports usage (`ConversationView.tsx:363-368`). The ~2.5 KB PM system prompt is re-sent as a hidden block on **every** turn (`cell.rs:686-688`), which is invisible in the UI and unexplained anywhere in the panel.
- **Covers:** —
- **Automation:** Backend: assert the recorded prompt blocks on turn N include the hidden block.
- **Status:** implemented (undocumented cost)

---

## 4 · Task view, agent sessions, and terminals

The task surface is terminal-first (ADR-0033): a 46px `TaskHeader` over one or two panes whose
tabs are almost always PTYs, with the ⌘J lifecycle-script drawer as a bottom sheet. Terminal
sessions live outside React (`lib/terminals.ts`, keyed by PTY id) so tab flips and task switches
detach the DOM without killing the shell; PTY lifetime belongs to the tab store, and the backend
enforces one agent terminal per task (`TerminalManager::find_running_agent`). Everything below was
read against `components/TaskView.tsx`, `TaskHeader.tsx`, `TerminalView.tsx`, `TabBar.tsx`,
`lib/terminals.ts`, `lib/commands.ts`, `store/tabs.ts`, `store/scripts.ts`,
`fartcode-app/src/terminals.rs`, `commands/terminals.rs`, `commands/tasks.rs`,
`commands/lifecycle.rs`, and `fartcode-core/src/pty/*`.

---

### Creating a task (⌘N composer)

#### TASK-01 — Open the composer from inside a task view
- **Given:** A project is selected and a task is open (task view mounted, an agent terminal focused).
- **When:** Press ⌘N.
- **Then:** The `New task` overlay card (`role="dialog"`, aria-label "New task") appears centred over the task view with the `›` glyph input auto-focused and the placeholder `describe the task…`; the footer reads `⌥ options` left and `↵ create & start` right.
- **Covers:** design_handoff_v2 §5h; `add-task` registered at **global** scope precisely so it fires from the task view.
- **Automation:** RTL component test on `Modals` with `lib/tauri` mocked + `useSidebar` seeded (`add-task` run() → `setCreateTaskTarget`).
- **Status:** implemented

#### TASK-02 — ↵ creates the task and hands the pane to the agent
- **Given:** The composer is open on a project whose `defaultAgent` binary resolves on PATH, no auto-run setup script configured.
- **When:** Type `fix the login redirect` and press ↵.
- **Then:** The dialog closes; the flyout/board selection moves to the new task; the task view's header breadcrumb reads `<project> / fix the login redirect` with a filled amber pulsing dot; the pane shows a live agent PTY **full-bleed with no tab bar**.
- **Covers:** ADR-0033 §4 + §5; design_handoff_v2 §5h footer `↵ create & start`.
- **Automation:** backend `create_task` (integration test `fartcode-app/tests/task_creation_agent_launch.rs`) + assert `terminal_list_for_task` returns exactly one `kind: "agent"` entry with `running: true`; the no-tab-bar half is an RTL test on `TaskView` with a one-tab pane.
- **Status:** implemented

#### TASK-03 — ⌥ unfolds the options block and the `from` picker lists branches
- **Given:** The composer is open on a project with local branches `main`, `feat/x` and remote `origin/feat/y`.
- **When:** Press ⌥ (or click `⌥ options`), then open the `from` select.
- **Then:** Three mono rows appear — `agent`, `from`, `branch`. The `from` menu lists `<baseRef> · default`, `project root · current checkout`, then each branch name once (remote twins of a local name and `*/HEAD` are dropped). Picking `feat/x` sets the `from` value to `feat/x` and the `branch` row to `feat/x`.
- **Covers:** design_handoff_v2 §5h.
- **Automation:** RTL test on `CreateTaskDialog` with `listProjectBranches` mocked.
- **Status:** implemented

#### TASK-04 — "project root · current checkout" creates a worktree-less task
- **Given:** The composer is open, options unfolded.
- **When:** Select `project root · current checkout` from `from`, then ↵.
- **Then:** The task is created with `workspace: "project-root"`; the `branch` row reads `current checkout · no isolation` before submit; the created task's terminals open with `cwd` = the project path, and the ⌘⌫ confirm for this task later omits the `removes worktree …` line.
- **Covers:** design_handoff_v2 §5h; `create_task_params` `WorkspaceTarget::ProjectRoot` + `GitSetup::None`.
- **Automation:** backend `create_task` with `workspace="project-root"` + assert the task's workspace row has no separate path (`fartcode-app/tests/create_task_params.rs` is the existing seam).
- **Status:** implemented

#### TASK-05 — Choose the agent and model for the new task
- **Given:** The composer is open, options unfolded, two agent CLIs installed (`claude` default, `codex` present).
- **When:** Click the `agent` row's value.
- **Then:** *(intended)* A menu opens listing installed providers and their models; picking `codex · sonnet` makes the created task launch `codex`.
- **Covers:** design_handoff_v2 §5h ("values right-aligned `#a4a4ab` with ⌄ where a menu opens").
- **Automation:** RTL test on `CreateTaskDialog` once a control exists.
- **Status:** not-built — the `agent` row renders a static string (`{agent} · default model`) with no `⌄` and no `<select>`; the launched provider is always the `defaultAgent` app setting.

#### TASK-06 — Queue the task instead of starting it now
- **Given:** Two agents are already running in this project and the composer is open.
- **When:** Look for a queue / "create without starting" control.
- **Then:** *(intended)* A choice exists between starting the agent immediately and parking the task with no session.
- **Covers:** design_handoff_v2 FLOWS §F5 ("queue in the design contradicts ADR-0033 §4 unless queueing is the concurrency limiter only") — never settled.
- **Automation:** needs a driver we lack (the behaviour does not exist to assert).
- **Status:** not-built — `create_task` always calls `launch_default_agent`; the composer's only submit is `↵ create & start`.

#### TASK-07 — Create a task with no agent CLI installed
- **Given:** No provider binary from the registry resolves on PATH; the project has no setup script.
- **When:** Create a task with ⌘N + ↵ and wait for the task view.
- **Then:** *(intended)* The pane states that the default agent is not installed and offers the install path. *(actual)* The pane shows the 5b empty state with the label `stopped · now` and the Resume/Split/New-terminal key list; nothing names the missing binary. The only trace is a `warn` line in the app log.
- **Covers:** design_handoff_v2 §5b (stop-reason label), §7d (agents on this machine).
- **Automation:** backend: run `create_task` with a PATH containing no agent binary + assert `terminal_list_for_task` is empty; frontend: RTL test on `PaneEmpty` with `agentByTask` unset and `statusChangedAt` = now.
- **Status:** partial — the empty state renders, but its label is a fabricated "stopped" for a task that never started.

#### TASK-08 — ⌥-chords inside the composer toggle the options block
- **Given:** The composer is open with options hidden.
- **When:** Press any ⌥-combination (e.g. ⌥←) inside the name field.
- **Then:** The options block unfolds — the bare `Alt` keydown that precedes the combination is what the toggle listens for.
- **Covers:** design_handoff_v2 §5h (⌥ unfolds options).
- **Automation:** RTL: `fireEvent.keyDown(window, { key: "Alt" })` and assert the `from` row appears.
- **Status:** implemented (with the side effect above)

---

### The terminal-first pane (ADR-0033)

#### TASK-09 — One tab shows no tab bar
- **Given:** A task with exactly one terminal (the agent) and no split.
- **When:** Open the task.
- **Then:** No `.tab-bar` element renders in the left pane; the terminal starts directly under the 46px header hairline. There is no `×` affordance on the terminal.
- **Covers:** ADR-0033 §5.
- **Automation:** RTL test on `TaskView` — seed `useTabs.panesByTask` with one tab, assert `queryByRole`/`.tab-bar` is absent.
- **Status:** implemented

#### TASK-10 — ⌘⇧T summons a second terminal and the tab bar with it
- **Given:** The same one-agent-tab task.
- **When:** Press ⌘⇧T.
- **Then:** A second tab appears and the left pane's tab bar becomes visible with two chips — `TTY <agent>` and `TTY Terminal` — the new one active with an accent underline; the new PTY runs `$SHELL` (or the project's `taskStartupCommand`) in the task's worktree.
- **Covers:** ADR-0033 §5; design_handoff_v2 §5b key list.
- **Automation:** RTL on `TaskView` + mocked `terminalOpen`; backend leg via `TerminalManager::open` with `agent: None` (existing `agent_terminals_integration.rs` fixture style).
- **Status:** implemented

#### TASK-11 — ⌘T reattaches the live agent instead of stacking a second
- **Given:** A task with a running agent terminal.
- **When:** Press ⌘T twice.
- **Then:** No new tab appears either time; the existing agent tab is focused. `terminal_list_for_task` still reports exactly one `kind: "agent"` entry with the same id.
- **Covers:** ADR-0033 §2/§3; `fartcode-app/tests/agent_terminals_integration.rs::second_agent_open_same_task_reattaches`.
- **Automation:** backend command `terminal_open_agent` called twice + assert equal ids (already covered); frontend dedupe via a store test on `useTabs.addTab` with a duplicate id.
- **Status:** implemented

#### TASK-12 — ⌘⇧O with a live agent lands on that agent, not a second OMP
- **Given:** A task whose live agent terminal is `claude`.
- **When:** Press ⌘⇧O.
- **Then:** No `omp` process spawns; the existing `claude` tab is focused and keeps its `claude` title (`addTab` dedupes by terminal id).
- **Covers:** ADR-0033 §2 "dispatch, ⌘⇧O, and the comment-task flow all converge".
- **Automation:** backend `terminal_open_agent(task, "omp")` after a live `claude` + assert the returned id equals the claude id.
- **Status:** implemented

#### TASK-13 — ⌘T is refused while the setup script is running
- **Given:** A project with `setup` configured and `autoRunSetupScriptOnTaskCreation` on; a freshly created task whose setup is still running; the pane has no tabs.
- **When:** Press ⌘T.
- **Then:** Nothing spawns. The pane keeps showing the dimmed one-liner `● Waiting on setup before starting…` (accent dot, body at 50% opacity) — the key list is deliberately suppressed.
- **Covers:** design_handoff_v2 §7b; `resumeAgentTab` early return on `setup.running`.
- **Automation:** RTL on `TaskView` with `useScripts.byTask[id].setup = { running: true }`; backend leg via `terminal_open_lifecycle` + `terminal_list_for_task`.
- **Status:** implemented

#### TASK-14 — ⌘T after a failed setup opens the drawer instead of the agent
- **Given:** The task's setup script exited 1.
- **When:** Press ⌘T.
- **Then:** No agent spawns. The ⌘J drawer opens on the `setup` tab showing the log tail ending in a red `setup exited 1 · <elapsed>` line; the header launcher reads `setup ✗` in `--fc-bad-text`; the pane's empty-state label reads `setup failed · exit 1` in red.
- **Covers:** design_handoff_v2 §7b ("a failed setup blocks agent start").
- **Automation:** backend: run a lifecycle terminal with `sh -c 'exit 1'` and assert the retained entry reports `running:false, exitCode:1` + the tail contains the red exit line (`lifecycle_terminals_integration.rs` pattern); frontend: RTL on `TaskView` + `TaskHeader` with the scripts store seeded.
- **Status:** implemented

#### TASK-15 — Rerunning a failed setup green does not start the agent
- **Given:** Setup failed at creation (agent launch was skipped by `launch_default_agent_after_setup`); the drawer is open on `setup`.
- **When:** Press `r` in the drawer and let the rerun exit 0.
- **Then:** The drawer tab loses its red `✗ exit 1` suffix and the header launcher flips to `setup ✓`; the pane's label changes from `setup failed · exit 1` to `stopped · <elapsed>`. **No agent starts** — the user must still press ⌘T.
- **Covers:** design_handoff_v2 §7b; `store/scripts.ts` comment "a green setup may auto-launch the agent (backend gate)" only holds for the creation-time waiter.
- **Automation:** backend `terminal_open_lifecycle` twice + assert no `kind:"agent"` entry appears; frontend RTL on `Drawer`.
- **Status:** partial — correct but under-communicated: nothing tells the user the agent is still not running.

#### TASK-16 — ⌘T when the resolved agent binary is missing
- **Given:** The `defaultAgent` setting names a provider whose binary is not on PATH; the pane is empty.
- **When:** Press ⌘T (or click the `Resume the agent` row in the empty state).
- **Then:** *(intended)* An inline failure states `agent not installed: <provider>` with a path to settings §7d. *(actual)* Nothing visible happens — `terminal_open_agent` rejects with that exact string and `lib/commands.ts` swallows it into `console.error("agent resume failed", e)`.
- **Covers:** design_handoff_v2 §5b/§7d.
- **Automation:** RTL: mock `terminalOpenAgent` to reject, run the `resume-agent` command, assert no user-visible node changes (documents the gap).
- **Status:** partial — the command runs and fails silently.

#### TASK-17 — ⌘. interrupts the live agent
- **Given:** A task with a running agent mid-turn.
- **When:** Press ⌘.
- **Then:** A `0x03` byte is written to the agent PTY; the terminal shows the agent CLI's own interrupt handling (its `^C` / "Interrupted" output). The app never fakes a stopped state and the tab stays open.
- **Covers:** design_handoff_v2 keymap decision (⌘. = stop agent); FLOWS §3.5.
- **Automation:** backend: `TerminalManager::write(id, "\x03")` against a `sh -c 'sleep 30'` agent entry + assert `terminal:exited` follows; frontend: mock `terminalWrite` and assert it is called with `"\x03"` and the live agent's id.
- **Status:** implemented

#### TASK-18 — ⌘. with no live agent
- **Given:** A task whose agent already exited (only a plain shell tab remains, or no tabs at all).
- **When:** Press ⌘.
- **Then:** *(intended)* Some acknowledgement that there is nothing to stop. *(actual)* Nothing happens and nothing is logged — `terminal_list_for_task` returns no running agent and the promise resolves.
- **Covers:** design_handoff_v2 §5b.
- **Automation:** RTL: run `stop-agent` with `terminalListForTask` mocked to `[]`, assert `terminalWrite` is never called.
- **Status:** partial

#### TASK-19 — ⌘. while a plain shell tab is focused
- **Given:** A split task: left pane = running agent, right pane = a shell running `sleep 300`; keyboard focus is in the right shell.
- **When:** Press ⌘.
- **Then:** The **agent** is interrupted; the focused `sleep` keeps running. `stop-agent` resolves its target from `terminal_list_for_task` (first `kind==="agent" && running`), never from the focused tab.
- **Covers:** design_handoff_v2 keymap decision; ADR-0033.
- **Automation:** RTL: seed two terminals, focus the shell, run `stop-agent`, assert the write went to the agent id.
- **Status:** implemented (arguably correct-by-design, but the key label gives no hint it is scope-wide)

---

### Splits, panes, and the tab bar

#### TASK-20 — ⌘D opens a split with a fresh shell
- **Given:** A task with one agent tab, no split.
- **When:** Press ⌘D.
- **Then:** The pane area splits in two with a hairline between them; the right pane gets a brand-new PTY tab titled `Terminal` in the task's worktree; **both** panes now render a tab bar, and the right bar's active chip carries the accent underline (`.tab-bar.active`) while the left's carries the neutral hairline.
- **Covers:** ADR-0033 §5 ("a split keeps both bars regardless of tab count"); design_handoff_v2 §5b `Split with a shell ⌘D`.
- **Automation:** RTL on `TaskView` with `terminalOpen` mocked; store test on `useTabs.toggleSplit`.
- **Status:** implemented

#### TASK-21 — Clicking into a pane does not make it the active pane
- **Given:** A split task; the split was just created so `activePaneByTask` is `"right"`.
- **When:** Click inside the **left** pane's terminal surface, then press ⌘⇧T.
- **Then:** *(intended)* The new terminal opens in the pane you clicked. *(actual)* The new tab opens in the **right** pane, and the accent underline stays on the right bar even though the keyboard caret is in the left terminal.
- **Covers:** ADR-0033 §5 ("the split's active-pane tint is the only focus affordance").
- **Automation:** RTL: seed a split, `fireEvent.click` the left `.terminal-container`, assert `useTabs.getState().activePaneByTask[taskId]` is still `"right"`.
- **Status:** partial — `setActivePane` is only reached by clicking a **tab chip** (`setActiveTab`); `TerminalView`'s click handler only focuses xterm.

#### TASK-22 — Collapsing the split kills the right pane's terminals with no confirm
- **Given:** A split task where the right pane holds the **agent** terminal (⌘T was pressed while the right pane was active).
- **When:** Press ⌘\ (or ⌘D is not the collapse key — ⌘\ / `split-pane` is).
- **Then:** The split collapses and the agent process is killed immediately — no confirm, no itemisation of what dies. `terminal_list_for_task` reports no running agent afterwards.
- **Covers:** design_handoff_v2 §7a (the delete confirm itemises "kills the running agent"); nothing equivalent guards tab/split teardown.
- **Automation:** store test on `useTabs.toggleSplit` asserting `killTerminal` is called for right-only ids; backend `TerminalManager::close` + `list_for_task`.
- **Status:** implemented (the kill), gap (the missing confirm)

#### TASK-23 — Closing the last tab of the split collapses it
- **Given:** A split where the right pane has exactly one tab.
- **When:** Click that tab's `×` (or press ⌘W with the right pane active).
- **Then:** The right pane disappears, `activePane` returns to `left`, and if the left pane now has one tab its bar disappears too — the task view returns to the full-bleed single-terminal shape.
- **Covers:** ADR-0033 §5; `store/tabs.ts` closeTab right-pane branch.
- **Automation:** store test on `useTabs.closeTab`.
- **Status:** implemented

#### TASK-24 — ⌘W on the sole agent tab kills a running agent silently
- **Given:** A task with exactly one tab: a running agent mid-turn. No tab bar is visible (so no `×`).
- **When:** Press ⌘W.
- **Then:** The agent process is killed, the tab vanishes, and the pane falls to the 5b empty state labelled `stopped · now`. No confirm appears and no warning says a live agent is about to die.
- **Covers:** ADR-0033 consequences ("With the bar hidden there is no × on the sole tab: ⌘W closes it"); design_handoff_v2 §7a is the only place that warns about killing an agent.
- **Automation:** store test on `useTabs.closeTab` → `killTerminal`; backend `terminal_close` + assert the PTY child is reaped.
- **Status:** implemented (the kill), gap (no confirm on a destructive action)

#### TASK-25 — Closing the last tab of the LEFT pane while a split is open
- **Given:** A split task; the left pane has one tab, the right has one.
- **When:** Close the left tab.
- **Then:** The left pane stays (the split does not collapse — only the right pane's emptiness collapses it), renders an **empty tab bar** plus the 5b key list, and the right pane is untouched. The user recovers with ⌘T / ⌘⇧T.
- **Covers:** `store/tabs.ts` closeTab (only `pane === "right"` collapses).
- **Automation:** store test on `useTabs.closeTab`; RTL on `TaskView` for the empty-bar rendering.
- **Status:** implemented (visually odd: an empty 34px tab bar with a hairline and nothing in it)

#### TASK-26 — ⌘1–9 and Ctrl+Tab walk the active pane's tabs
- **Given:** A task with four tabs in the left pane, tab 1 active, and keyboard focus inside the terminal.
- **When:** Press ⌘3, then Ctrl+Tab, then Ctrl+⇧Tab.
- **Then:** The active chip moves to tab 3, then tab 4, then back to tab 3, wrapping at the ends. The previously active terminal keeps its scrollback and its PTY (only `display:none` changes); returning to it re-focuses xterm's helper textarea.
- **Covers:** design_handoff_v2 keymap ("⌘1–9 stays tab nav"); `TerminalView` activation effect.
- **Automation:** store tests on `jumpToTab` / `cycleTab`; the focus half needs a driver we lack (xterm does not measure in jsdom).
- **Status:** implemented

#### TASK-27 — ⌘\ in a task view splits instead of toggling the flyout
- **Given:** The project flyout is open and a task view is focused.
- **When:** Press ⌘\.
- **Then:** *(intended, per the settled keymap)* The flyout collapses. *(actual)* The task view splits — `split-pane` is bound to ⌘\ at `task-view` scope, which outranks `toggle-sidebar`'s global ⌘\ in the precedence chain. ⌘B is the only flyout toggle that works inside a task.
- **Covers:** design_handoff_v2 README "Keymap: … ⌘B/⌘\ stay flyout toggle"; DESIGN.md Layout ("244px project flyout (⌘\ / ⌘B)").
- **Automation:** `registry.test.ts`-style test against the **real** registry (`registerAllCommands()`), dispatching ⌘\ with `taskView: true` and asserting the returned command id.
- **Status:** implemented-contradicting-spec

---

### Agent completion and the needs-you state

#### TASK-28 — The agent finishes and the terminal reports the exit
- **Given:** A task whose agent terminal is running.
- **When:** The agent CLI exits (e.g. type `/exit` or let it finish).
- **Then:** `terminal:exited` fires; the terminal appends `[process exited with code 0] — close this tab`; the header dot changes from filled amber pulse to the dim idle dot; typing into that terminal no longer reaches a PTY. The tab remains open.
- **Covers:** ADR-0033; design_handoff_v2 §5a agent dot.
- **Automation:** backend: open an agent entry running `sh -c 'exit 0'` and assert the `terminal:exited` emit + entry removal from `list_for_task` (`agent_terminals_integration.rs::exited_agent_terminal_allows_a_fresh_spawn`); frontend: RTL on `TaskHeader` driving `onTerminalExited`.
- **Status:** implemented

#### TASK-29 — The 5b "stopped · elapsed" state after the agent completes
- **Given:** A one-tab task whose agent just exited.
- **When:** Look at the pane.
- **Then:** *(intended, frame 5b)* The centred mono label `stopped by you · 8m ago` over the 260px Resume/Split/New-terminal key list. *(actual)* The dead terminal stays on screen with its `[process exited…]` line; the empty state only renders when `tabs.length === 0`, i.e. after the user manually presses ⌘W.
- **Covers:** design_handoff_v2 §5b.
- **Automation:** RTL on `TaskView` with a one-tab pane + an exited session — assert `.tv-empty` is absent.
- **Status:** partial — the 5b frame is effectively unreachable on the normal completion path.

#### TASK-30 — The hollow "needs you" ring when the agent asks a question
- **Given:** A task whose agent has stopped mid-turn to ask the user something (TUI path).
- **When:** Look at the header dot, the rail tile, and the flyout row.
- **Then:** *(intended)* A hollow amber ring (`status-needs-you`, 8px) on all three, with the flyout meta line reading `needs you`.
- **Covers:** DESIGN.md status-dot vocabulary; design_handoff_v2 FLOWS §F7 ("Status comes from agent hooks, never output sniffing" — E3-05); ADR-0037 item 5.
- **Automation:** needs a driver we lack — no hook server exists to drive the transition.
- **Status:** unreachable-entirely — `TaskHeader` keys the ring on `task.status === "review"`, and `TaskStore::update_status` has **no production callers** (`components/board/runState.ts:6` records the same finding). E3-05's hook server is a PRD line and an env-overlay stub (`pty/env_allowlist.rs:163`), not code.

#### TASK-31 — Answering the agent in the terminal
- **Given:** The agent is prompting inside the TUI (e.g. a y/n permission line).
- **When:** Click the terminal surface and type `y` + ↵.
- **Then:** The keystrokes reach the PTY (`terminal_write`) and the agent proceeds. App chords are unaffected: ⌘W/⌘⇧T/⌘. still fire because `isEditableTarget` explicitly exempts `.xterm-helper-textarea`.
- **Covers:** design_handoff_v2 FLOWS §F7 "Phase-0 version: click through to the terminal".
- **Automation:** backend `terminal_write` round-trip against a `sh -c 'read x; echo got-$x'` PTY + assert the output chunk; the click-to-focus leg needs a driver we lack.
- **Status:** implemented

#### TASK-32 — Agent exit settles the linked board card
- **Given:** A task created by dragging issue #12 into an `agent_step` column; its agent terminal is running.
- **When:** The agent process exits.
- **Then:** The terminal pump calls `flip_for_exited_agent` with session identity `pty:<terminalId>`; the issue advances/holds per that column's `on_settle`, and the board card's dot changes accordingly (`step:settled` / lane move) without any user gesture.
- **Covers:** ADR-0032 auto-flip; ADR-0037 settle; `dispatch.rs:168`.
- **Automation:** backend `fartcode-app/tests/dispatch_integration.rs` ("pump hook") drives exactly this.
- **Status:** implemented

---

### Persistence, reattach, and restart

#### TASK-33 — Switching tasks keeps both terminals alive
- **Given:** Task A has a running agent producing output; task B exists.
- **When:** Switch to B (flyout row click or ⌘⌥↓), wait, then switch back to A.
- **Then:** A's terminal shows the output that arrived while it was hidden, with scrollback and cursor position intact; no respawn happens (the PTY id is unchanged) and the process was never signalled. `TerminalView`'s unmount only calls `session.host.remove()`.
- **Covers:** `lib/terminals.ts` module-scope session registry; `TerminalView` "detach, never kill".
- **Automation:** backend: assert the PTY id from `terminal_list_for_task` is stable across the switch; frontend: store test that `killTerminal` is not called on task switch.
- **Status:** implemented

#### TASK-34 — Webview reload reattaches and replays the tail
- **Given:** A task with a live agent that has printed a screenful; the Rust process is untouched.
- **When:** Reload the webview (dev: HMR / ⌘R).
- **Then:** `ensureTabs` finds the persisted tab ids in `terminal_list_for_task` (they are live) and reattaches **without respawning**; each terminal replays up to 64 KB of scrollback from `terminal_tail` before live output resumes, in order (output arriving during the fetch is buffered, not dropped).
- **Covers:** ADR-0028 §3; `terminals.rs` `TAIL_CAP` / `push_tail`.
- **Automation:** backend: `TerminalManager::tail` after writing >64 KB + assert the tail is capped and oldest-first-drained; frontend: store test on `ensureTabs` with `terminalListForTask` returning the persisted ids (assert `terminalOpen` is never called).
- **Status:** implemented

#### TASK-35 — App restart with tmux OFF resurrects the agent tab as a plain shell
- **Given:** Project `tmux` setting off. A task with one agent tab titled `claude`. Quit the app (⌘Q) and relaunch.
- **When:** Open the task.
- **Then:** *(intended)* The tab either comes back as an agent session or is clearly gone. *(actual)* `WindowEvent::Destroyed → detach_all` killed the agent PTY; on reopen `ensureTabs` sees a dead id, calls `terminal_open` (a plain `$SHELL`), and keeps the persisted title — so the pane shows a tab labelled **`TTY claude`** that is actually a bare shell, with the tab bar still hidden (one tab) so the mislabelling is invisible.
- **Covers:** ADR-0021/ADR-0028 restart-survival contract; ADR-0033 §3.
- **Automation:** store test on `ensureTabs`: `getViewState` returns a saved terminal tab, `terminalListForTask` returns `[]`, `terminalOpen` resolves a fresh id — assert the kept tab has the new id and the **old title**.
- **Status:** partial — restart survival is real for shells, wrong for agents.

#### TASK-36 — App restart with tmux ON reattaches survivors and surfaces extras
- **Given:** Project `tmux` on and a tmux binary on PATH. A task with two shell tabs. Force-quit the app (SIGKILL — no `Destroyed` event), relaunch, open the task.
- **When:** The task view mounts.
- **Then:** Both persisted tabs reattach to the surviving `<project>:<task>:terminal:<slot>` sessions with their cwd and scrollback; `terminal_surviving` then reports 0 (all covered) so no extra tab is added. If a third session survived that no persisted tab covers, one extra `Terminal` tab appears for it.
- **Covers:** ADR-0028 §2/§3; `terminals.rs::pick_slot` / `surviving_session_count`.
- **Automation:** backend integration test over `choose_terminal_slot` + `list_tmux_sessions_by_prefix` (pure fns are already unit-tested); the full restart leg needs a driver we lack.
- **Status:** implemented

#### TASK-37 — Closing a tab is final for its tmux session
- **Given:** tmux on; a task with a shell tab in slot 0.
- **When:** Close the tab (`×` / ⌘W), then press ⌘⇧T.
- **Then:** `tmux ls` no longer lists that session; the new terminal creates slot 0 fresh (the freed slot is reused, not climbed past). Nothing from the closed shell comes back.
- **Covers:** ADR-0028 §1 ("close kills").
- **Automation:** backend: `TerminalManager::close` then `list_tmux_sessions_by_prefix` — needs a tmux binary in CI.
- **Status:** implemented

#### TASK-38 — Deleting a task tears down terminals, tabs, and view state
- **Given:** A task with a running agent, a shell tab, a retained (exited) setup terminal, and a saved `view-state:task:<id>:tabs` row.
- **When:** Press ⌘⌫, read the confirm, press ⌘⌫ again.
- **Then:** The confirm itemises `kills the running agent` (with a live pulse dot), `removes worktree <branch>`, `deletes N line comments · N terminals`, and `branch <branch> is kept`. On confirm: every PTY of the task is killed, the task's tmux sessions (including orphans from a crashed instance) are swept, the `task:deleted` event drops the tab state locally, and the two view-state rows are deleted.
- **Covers:** ADR-0023; design_handoff_v2 §7a; `delete_task` → `terminals.close_task`; `store/tabs.ts` `wireTabsEvents`.
- **Automation:** backend `delete_task` + assert `list_for_task` empty and the kv rows gone; frontend store test on `dropTask`.
- **Status:** implemented

#### TASK-39 — Boot prune drops tab state for tasks deleted out-of-band
- **Given:** A `view-state:task:<id>:tabs` row whose task row no longer exists (e.g. deleted while a previous process crashed mid-teardown).
- **When:** Relaunch the app.
- **Then:** `prune_orphans` deletes the orphaned `:tabs` row at boot; no ghost task view is reachable.
- **Covers:** E1-08; `fartcode-core/src/view_state.rs::prune_orphans` (already unit-tested).
- **Automation:** backend unit test (exists).
- **Status:** implemented

---

### Concurrency and task switching

#### TASK-40 — Two tasks, two agents, at once
- **Given:** Task A and task B in the same project, each with its own worktree and a running agent.
- **When:** Switch A → B → A while both agents produce output.
- **Then:** Both agents keep running in their own worktrees; each terminal shows its own uninterrupted output on return; neither PTY is resized to the other's dimensions (the resize only fires from the visible container's `ResizeObserver` and the activation poke).
- **Covers:** ADR-0033 §2 (one agent **per task**, not per app).
- **Automation:** backend: two `terminal_open_agent` calls for different task ids + assert distinct ids and both `running` (`agent_terminals_integration.rs::other_tasks_get_their_own_agent_terminal`).
- **Status:** implemented

#### TASK-41 — ⌘⌥↓ / ⌘⌥↑ walk tasks in flyout order
- **Given:** A project with tasks T1..T4, T3 pinned, and a task view open on T1.
- **When:** Press ⌘⌥↓ repeatedly.
- **Then:** Selection walks pinned-first then tree order (T3, T1, T2, T4), skipping archived tasks and tasks under collapsed projects, and wraps. Each landing mounts that task's panes (`ensureTabs` runs once per task and is idempotent).
- **Covers:** `visibleTaskOrder` contract (unit-tested in `store/sidebar.test.ts`).
- **Automation:** unit test on `visibleTaskOrder` (exists) + a store test on `switchTask`.
- **Status:** implemented

#### TASK-42 — Telling which of two tasks has a live agent, without opening them
- **Given:** Task A's agent is running; task B's agent exited an hour ago.
- **When:** Look at the project flyout.
- **Then:** *(intended)* A shows under **Running**, B under **Recent**. *(actual)* Both show under **Running** with a filled amber pulse — the flyout keys on `task.status`, which is frozen at `in_progress` from birth. The live-agent truth (`useScripts.agentByTask`) is only hydrated for tasks whose `TaskView` or board card has mounted.
- **Covers:** design_handoff_left_nav flyout contract; DESIGN.md "flyout in-flight = agent-step column with a live session, or needs-you".
- **Automation:** RTL on `Nav` with two tasks both `status: "in_progress"` — assert both land in the Running group (documents the gap); `Nav.test.tsx` is the existing seam.
- **Status:** partial — the board's cards are live (they call `useScripts.hydrate`), the flyout and rail are not.

#### TASK-43 — Which agent is running is invisible in the default task view
- **Given:** A task running `codex` while the app's default agent is `claude`; single tab, no split.
- **When:** Look at the task view.
- **Then:** *(intended)* Something on screen names the provider. *(actual)* Nothing does — the header shows only breadcrumb + title + dot, and the agent name lives solely in the tab chip, which is hidden at one tab (ADR-0033 §5). The name only reappears after ⌘⇧T summons the bar.
- **Covers:** design_handoff_v2 §5a (header contents); ADR-0033 §5.
- **Automation:** RTL on `TaskView` + `TaskHeader` with a one-tab pane — assert the provider string appears nowhere.
- **Status:** partial

---

### The ⌘J script drawer (task-view chrome)

#### TASK-44 — A header launcher opens the drawer on its script and starts it
- **Given:** A project with `setup` and `run` configured; neither has run for this task.
- **When:** Click the `run` launcher in the header.
- **Then:** The 210px bottom sheet opens on the `run` tab; the script starts once (a second click reattaches instead of spawning a second run — the backend dedupes in-flight lifecycle runs by type); the log body opens with the script's own lines echoed dim and `$`-prefixed, and ends with `run exited 0 · 1.4s` in dim (or red on failure).
- **Covers:** design_handoff_v2 §5a/§7b; E1-06.
- **Automation:** backend `lifecycle_terminals_integration.rs` (dedupe + retained entry + tail) — exists; frontend RTL on `TaskHeader` + `Drawer`.
- **Status:** implemented

#### TASK-45 — `r` in the drawer with no script configured
- **Given:** A project with **no** lifecycle scripts configured.
- **When:** Press ⌘J, then `r` on any tab.
- **Then:** *(intended)* The drawer says the script is unset and offers the settings path. *(actual)* All three tabs (`setup`/`run`/`teardown`) render regardless of configuration; the body reads `not run yet · r runs it`; `r` calls `terminal_open_lifecycle`, which rejects with `no <type> script configured for this project`, and the error goes to `console.error` only. The message never changes — a dead end with a lying affordance.
- **Covers:** design_handoff_v2 §7b (the drawer's tabs are "the three lifecycle scripts"); `TaskHeader` already filters to configured scripts, the drawer does not.
- **Automation:** RTL on `Drawer` with `terminalOpenLifecycle` mocked to reject — assert the body text is unchanged.
- **Status:** partial

#### TASK-46 — The drawer keeps `r` while the script terminal has focus
- **Given:** The drawer is open on a running script.
- **When:** Click into the script's terminal, type `r`, then click the drawer strip and press `r`.
- **Then:** The first `r` goes into the script's stdin (the terminal owns the keyboard); the second reruns the script. The drawer chrome re-takes focus after every tab/terminal flip, so a freshly opened drawer's `r` reruns rather than typing.
- **Covers:** design_handoff_v2 §7b (`r rerun · ⌘j close`).
- **Automation:** RTL on `Drawer` for the chrome-focus path; the xterm-focus path needs a driver we lack.
- **Status:** implemented

---

### Narrow / laptop layout

#### TASK-47 — The task view on a ~900px window
- **Given:** The window narrowed to 900px with the flyout (244px) and the Changes sheet (320px) both open, on a split task.
- **When:** Read the terminals.
- **Then:** *(intended)* Some narrow rule — collapse the split, drop a panel, or a minimum pane width. *(actual)* The main region is ~280px, each split pane ~140px, and each xterm fits to roughly 18 columns; nothing collapses, nothing warns, and the rail does not narrow to the 48px DESIGN.md specifies. The only narrow mode in the app is the board's (`.board-narrow`, ResizeObserver at 900px).
- **Covers:** DESIGN.md Layout ("Under ~900px the board collapses to one column and the rail narrows to 48px"); design_handoff_v3 §8b covers the board only.
- **Automation:** RTL/jsdom cannot measure xterm; a visual check at 900px is the honest driver — needs a driver we lack.
- **Status:** not-built

#### TASK-48 — Header actions at narrow width
- **Given:** A narrow window, a long task title, and three configured scripts (`setup ✓ · run · teardown`) plus the `⌘⇧1 changes` toggle.
- **When:** Read the 46px header.
- **Then:** The action cluster keeps its full width (`flex: none`) and the task title ellipsises first (`min-width: 0` + `text-overflow: ellipsis`); the breadcrumb never wraps and the header never grows past 46px.
- **Covers:** design_handoff_v2 §5a.
- **Automation:** RTL on `TaskHeader` asserting the CSS contract classes; the truncation itself needs a driver we lack.
- **Status:** implemented

---

### Unreachable surfaces in this area

#### TASK-49 — Open an ACP conversation as a pane tab
- **Given:** A task with an ACP-capable provider.
- **When:** Try to open the structured-chat transcript as a **tab** in the task pane.
- **Then:** *(intended)* A `conversation` tab (`ACP` glyph) opens beside the terminal. *(actual)* No caller exists — ⌘⇧A (`open-conversation`) routes to the right **sheet** (`TaskChatPanel`), and `focusConversationTab` / `focusOrOpenTab` in `lib/acp-conversation.ts` are unreferenced. The `conversation` kind stays registered only so persisted pre-redesign tabs survive `sanitizePane`.
- **Covers:** E2-11-6; ADR-0033 (the sheet is the chat surface now).
- **Automation:** grep-level assertion (no caller) + RTL that ⌘⇧A opens the sheet, not a tab.
- **Status:** unreachable-by-mouse (dead code path)

#### TASK-50 — ⌘⇧A opens the task chat in the sheet
- **Given:** A task view with the sheet closed.
- **When:** Press ⌘⇧A, then ⌘⇧A again.
- **Then:** First press opens the right sheet in chat mode ("Task chat" header) and starts the task's ACP conversation; second press closes the sheet. With the sheet already open on Changes, one press switches Changes → chat instead of closing.
- **Covers:** E2-11-6; `openTaskChat` in `lib/commands.ts`.
- **Automation:** RTL on `ChangesSidebar` + `useUi` state assertions.
- **Status:** implemented

---

## 5 · Changes, commit, PR, and checks

The ship loop is the ⌘⇧1 right sheet (`ChangesSidebar.tsx`) with two tabs — **Changed/Staged** (rows + commit card + git footer) and **Pull Requests** (`PullRequestPanel.tsx`) — plus the diff tabs it opens into the task panes (`DiffView.tsx`, `DiffSelectionPopover.tsx`, `CommentThread.tsx`). Everything refreshes off the fs/git watcher (`git:changed` / `files:changed`, coalesced ~150 ms in `store/changes.ts`) and the PR sync cache (`pr:updated`, `fartcode-core/src/pr_sync`); nothing in the UI polls. Backends are thin Tauri wrappers over `fartcode-git` (`status.rs`, `stage.rs`, `diff.rs`, `commit.rs`, `remote.rs`, `pr_sync.rs`) — every row action, commit, and push is one git CLI invocation with the non-interactive env.

Scenarios below were written against the code, not the docs. Where the code and `design_handoff_v2 §5d/5e/5f` or `FLOWS.md F8` disagree, the code wins and the delta is recorded as a gap.

---

### Sheet, scope, and persistence

#### SHIP-01 — Open the changes sheet on a task with ⌘⇧1
- **Given:** A project with a task whose workspace is provisioned on disk and has ≥1 modified tracked file; the task view is focused and the sheet is closed.
- **When:** Press `⌘⇧1` (or click the `⌘⇧1 changes` chip at the right of the task header).
- **Then:** A right-hand panel appears with a "Changes" header, a `Changed / Pull Requests` tab row, a `CHANGED` section listing the modified file with its `M` glyph and `+n −n` stats, an empty `STAGED` section, the commit card, and the footer hint `d discards after a confirm · fetch / pull / push in ⌘K`. The header ref reads the current branch.
- **Covers:** FLOWS.md F8-1; design_handoff_v2 §5d; `commands.ts:166` `toggle-changes`.
- **Automation:** RTL component test on `ChangesSidebar` with the changes/commit-state stores seeded; or browser smoke driving `window.__changesStore`.
- **Status:** implemented

#### SHIP-02 — ⌘⇧1 swaps chat mode → changes mode before closing the sheet
- **Given:** Task scope, sheet open showing the task chat (`⌘⇧A`).
- **When:** Press `⌘⇧1` once, then `⌘⇧1` again.
- **Then:** First press replaces the chat with the changes surface at the same width; second press closes the sheet entirely. Chat and changes never render stacked.
- **Covers:** `commands.ts:171-185`; ADR-0032 §5 (one right panel).
- **Automation:** RTL test asserting `useUi` state transitions plus rendered surface.
- **Status:** implemented

#### SHIP-03 — Project scope shows the repo checkout but the rows are dead ends
- **Given:** A project selected with no task selected (board view), the project has a `repositoryWorkspaceId` and dirty files in the project checkout.
- **When:** Press `⌘⇧1` and click any file row.
- **Then:** The sheet lists the project checkout's changes and the row's tooltip reads `<path> — diff opens in the task view`; clicking does nothing (no tab opens anywhere). Stage/unstage/discard on the row still work.
- **Covers:** `ChangesSidebar.tsx:489-504` (`no-diff` class, `if (!taskId) return`); E17 dogfood note in the file header.
- **Automation:** RTL test: render at project scope, click a row, assert `useTabs` is untouched.
- **Status:** partial (see SHIP-GAP-03 — no diff surface at project scope)

#### SHIP-04 — Sheet open state, tab, and width do not survive a restart
- **Given:** Sheet open on the **Pull Requests** tab, dragged to 560 px wide.
- **When:** Quit and relaunch fartCode, reselect the same task.
- **Then (intended):** The sheet reopens on Pull Requests at 560 px. **Actual:** the sheet is closed; reopening lands on `Changed` at the default 400 px.
- **Covers:** MEMORY.md ("`changesOpen` — NOT persisted"); `ChangesSidebar.tsx:66` local `panelTab`; `useGutterResize.ts:4` ("Widths are in-memory only").
- **Automation:** Manual restart, or a test asserting no `setViewState` call for these keys.
- **Status:** not-built

#### SHIP-05 — Task whose workspace row exists but has no path offers provisioning
- **Given:** A task created against a workspace whose `workspaces.path` is NULL/empty (e.g. provisioning failed).
- **When:** Open the sheet.
- **Then:** The body reads "This task's workspace isn't on disk yet." with a **Provision workspace** button; pressing it shows "Provisioning…", then either the change list renders or the raw error text replaces the button's state.
- **Covers:** `ChangesSidebar.tsx:261-293`; `commands/git.rs:35` (`workspace has no local path`).
- **Automation:** Backend: insert a workspace row with NULL path, `invoke("git_status")`, assert the exact error string; frontend RTL on the branch.
- **Status:** implemented

#### SHIP-06 — Task with no workspace at all
- **Given:** A task whose `workspaceId` is null (and no project `repositoryWorkspaceId`).
- **When:** Press `⌘⇧1`.
- **Then:** Nothing opens — the panel returns null (`!taskId && !workspaceId` is the only null case; with a task and no workspace the body reads "This task has no workspace yet — changes appear once it's provisioned." and there is no Pull Requests tab).
- **Covers:** `ChangesSidebar.tsx:102`, `:230` (tab row gated on `workspaceId`), `:257`.
- **Automation:** RTL with a task lacking `workspaceId`.
- **Status:** implemented

---

### Change list: states and live refresh

#### SHIP-07 — Clean worktree renders the empty state, not empty sections
- **Given:** A provisioned task workspace with `git status` clean.
- **When:** Open the sheet.
- **Then:** The body shows a single line "No changes"; the commit card and footer still render below it (the card's Commit row is disabled).
- **Covers:** `ChangesSidebar.tsx:307`.
- **Automation:** RTL with a seeded empty snapshot.
- **Status:** implemented

#### SHIP-08 — A >10,000-file worktree degrades to a notice with no way out
- **Given:** A worktree with more than `MAX_STATUS_FILES` (10,000) status entries — e.g. an unignored `node_modules` after a fresh install.
- **When:** Open the sheet.
- **Then:** The body reads "Too many changed files to show — refine on the command line." **and the commit card and footer are not rendered** (they are gated on `snapshot && changeCount` paths that this branch skips), so there is no in-app stage-all, no commit, and no `.gitignore` affordance.
- **Covers:** `status.rs:27` `MAX_STATUS_FILES`; `ChangesSidebar.tsx:303-306`.
- **Automation:** Backend integration test asserting `truncated: true` with empty lists; frontend RTL on the truncated snapshot.
- **Status:** partial (see SHIP-GAP-08)

#### SHIP-09 — An agent writing files updates the list without user action
- **Given:** Sheet open on a task; an agent terminal in the same worktree.
- **When:** The agent writes `src/new.ts` and edits `src/old.ts`.
- **Then:** Within roughly one debounce window (~100 ms watcher batch + 150 ms store coalesce) a row `A src/new.ts` and a row `M src/old.ts` appear, with `+n` counts; no click, no refresh button.
- **Covers:** `fs_watch/mod.rs` DEBOUNCE; `store/changes.ts:112-133`; FLOWS.md F8-1 ("live-refreshing off fs+git events").
- **Automation:** Backend: touch files in the worktree and assert a `files:changed` event carries the worktree-relative paths; frontend: emit the event and assert one refetch (not two).
- **Status:** implemented

#### SHIP-10 — The project-scope sheet does not live-refresh
- **Given:** A project whose tasks all use linked worktrees (no task on the project-root workspace); the sheet is open at project scope showing the checkout's changes.
- **When:** Edit a file in the project checkout from an external terminal.
- **Then (intended):** The new row appears within a debounce. **Actual:** nothing changes until a row action (stage/unstage/discard) forces a refetch, or the task/project selection cycles.
- **Covers:** `fs_watch/mod.rs:363` `boot_targets` (`FROM tasks t JOIN workspaces w`) — the project's `repository_workspace_id` is never registered; `watchers.rs` only registers on `TaskProvisioned`.
- **Automation:** Backend integration test: create a project, no tasks, touch a file at the project root, assert no `files:changed` for the repository workspace.
- **Status:** not-built (see SHIP-GAP-10)

#### SHIP-11 — A worktree that disappears leaves a stale list with no error
- **Given:** Sheet open showing 3 changed files.
- **When:** `rm -rf` the worktree directory from a terminal (or the worktree is pruned externally).
- **Then (intended):** The panel surfaces the git failure and offers a retry. **Actual:** the three rows stay on screen indefinitely — `entry.error` is only rendered when there is no snapshot, and refetch failures keep the last snapshot.
- **Covers:** `ChangesSidebar.tsx:296` (`entry?.error && !snapshot`); `store/changes.ts:56-58` (error patched, snapshot retained).
- **Automation:** Frontend: seed a snapshot, then force `gitStatus` to reject, assert no error node in the DOM.
- **Status:** not-built (see SHIP-GAP-11)

#### SHIP-12 — Renames render source → target
- **Given:** A staged rename (`git mv a.ts b.ts`).
- **When:** Open the sheet.
- **Then:** The `STAGED` section shows one row with glyph `R` and the path rendered as `a.ts → b.ts`; clicking it opens a staged diff whose header repeats the arrow and whose baseline is `HEAD:a.ts`.
- **Covers:** `status.rs` rename records; `diff.rs:87` (`HEAD:{head_path}` with `orig_path`); `ChangesSidebar.tsx:510-518`.
- **Automation:** Backend integration test over a temp repo asserting `origPath` on the snapshot and the diff payload.
- **Status:** implemented

---

### Stage, unstage, discard

#### SHIP-13 — Stage one file with the row's `s`
- **Given:** Sheet open with one modified file under `CHANGED`.
- **When:** Click the `s` affordance in the row's right-hand meta.
- **Then:** The row leaves `CHANGED` and appears under `STAGED` with a `u` affordance; the section counts flip 1→0 and 0→1; the commit card's Commit row becomes enabled once a message is typed.
- **Covers:** design_handoff_v2 §5d; `ChangesSidebar.tsx:528-535`; `stage.rs:18`.
- **Automation:** RTL click + mocked `git_stage`, asserting the refetch; backend test `stage_and_unstage_round_trip` already exists (`stage.rs:106`).
- **Status:** implemented

#### SHIP-14 — Stage all with `a`
- **Given:** Sheet open (panel focused) with 2 modified + 1 untracked file.
- **When:** Press `a`, or click `a stage all` on the CHANGED section label.
- **Then:** All three move to `STAGED` (including the untracked file — `git add -A`); the `a stage all` affordance disappears because the unstaged count is 0.
- **Covers:** `ChangesSidebar.tsx:176-177`, `:436-448`; `stage.rs:24`.
- **Automation:** RTL keydown on the region; backend test `stage_all_picks_up_everything`.
- **Status:** implemented

#### SHIP-15 — Unstage with `u`, including on an unborn HEAD
- **Given:** A brand-new repo with no commits, one file staged.
- **When:** Hover the staged row and press `u` (or click `u`).
- **Then:** The file returns to `CHANGED` as an untracked `A` row; no error appears (the backend falls back from `restore --staged` to `rm --cached -r`).
- **Covers:** `stage.rs:31-43` + test `unstage_on_unborn_head_uses_rm_cached`.
- **Automation:** Backend integration test (exists); frontend RTL for the key path.
- **Status:** implemented

#### SHIP-16 — Discard demands the inline confirm and `esc` cancels
- **Given:** Sheet open, one modified tracked file.
- **When:** Click `d` on the row, then press `Escape`.
- **Then:** An overlay card appears reading "Discard changes to `<path>`?" with the sub-line "Unstaged edits to this file are lost." and a footer `esc cancel · d discard`; `Escape` dismisses it and the file is unchanged on disk. Pressing `d` instead reverts the file to its index state and the row disappears.
- **Covers:** design_handoff_v2 §5d "d discards after a confirm"; `ChangesSidebar.tsx:357-408`; `stage.rs:50`.
- **Automation:** RTL: click `d`, assert `role="alertdialog"`, press Escape, assert `git_discard` was never invoked.
- **Status:** implemented

#### SHIP-17 — Discarding an untracked file says it will be deleted
- **Given:** An untracked file `scratch.txt` in the worktree.
- **When:** Click `d` on its row.
- **Then:** The confirm's sub-line reads "The file is untracked — discarding deletes it." Confirming removes the file from disk (not just the index).
- **Covers:** `ChangesSidebar.tsx:393-395`; `stage.rs:56-68`.
- **Automation:** Backend test `discard_restores_modified_and_deletes_untracked` (exists) + RTL for the copy.
- **Status:** implemented

#### SHIP-18 — Discard is unrecoverable and the confirm does not say so beyond one line
- **Given:** An untracked file containing an hour of unsaved agent output.
- **When:** Confirm the discard.
- **Then (intended):** Either an undo affordance or a trash-instead-of-unlink path exists. **Actual:** the file is `remove_file`'d immediately; nothing in the app can bring it back and there is no undo anywhere in the sheet.
- **Covers:** `stage.rs:64-67`.
- **Automation:** Backend test asserting the file is gone; the "no undo" half is an inspection of the command surface.
- **Status:** not-built (see SHIP-GAP-18)

#### SHIP-19 — A failing row action is completely silent
- **Given:** A file listed in the snapshot that has since been removed from disk *and* was never tracked (the snapshot is 150 ms stale).
- **When:** Click `d` and confirm.
- **Then (intended):** An inline error ("path is neither tracked nor on disk"). **Actual:** the promise rejects unhandled; no error node, no toast, the row simply stays (the follow-up refetch never runs because the throw happens before it).
- **Covers:** `ChangesSidebar.tsx:133`, `:443`, `:532`, `:551` — every call site is `void store.x(...)` with no `.catch`; `store/changes.ts:78-96` re-throws; `stage.rs:59` produces the error.
- **Automation:** Frontend: mock `git_discard` to reject, assert no error node and an unhandled rejection.
- **Status:** not-built (see SHIP-GAP-19)

#### SHIP-20 — A keyboard-only user cannot stage, unstage, or discard a single file
- **Given:** Sheet open with 4 changed files; no pointing device used.
- **When:** Tab into the sheet and try to select the second row, then press `s`.
- **Then (intended):** Arrow keys move a row selection; `s` stages the selected row. **Actual:** rows are `tabIndex={-1}` with no arrow-key navigation, so `active` is never set; `s`/`u`/`d` no-op. Only `a` (stage all) is reachable without a mouse.
- **Covers:** `ChangesSidebar.tsx:488` (`tabIndex={-1}`), `:492-493` (`active` set only by `onMouseEnter`/`onFocus`), `:178-189`.
- **Automation:** RTL: focus the region, `userEvent.keyboard("s")`, assert `git_stage` not called.
- **Status:** unreachable-entirely without a pointing device (see SHIP-GAP-20)

#### SHIP-21 — The single-key target survives the pointer leaving the row
- **Given:** Sheet open; hover row `src/critical.ts` (setting it active), then move the pointer to the "Changes" header (not over any row).
- **When:** Press `d`.
- **Then (intended):** Nothing, or a confirm for a visibly-selected row. **Actual:** the discard confirm opens for `src/critical.ts` — `active` is never cleared on mouse-leave and nothing in the list is visually marked as the target.
- **Covers:** `ChangesSidebar.tsx:492-493` (no `onMouseLeave`), `:186-189`.
- **Automation:** RTL: `pointerEnter` a row, `pointerLeave`, keydown `d`, assert the alertdialog appears.
- **Status:** partial — behaves as coded, but the target is invisible (see SHIP-GAP-21)

#### SHIP-22 — A conflicted file has no mouse affordance to resolve it
- **Given:** A worktree mid-merge with one conflicted file (`git status` reports a `u` record).
- **When:** Open the sheet and look at the conflicted row in both sections.
- **Then (intended):** A way to mark the conflict resolved (stage it) after editing. **Actual:** the row renders `conflict` in the meta slot with **no** `s`, `d`, or `u` affordance in either section; the only path is hovering the CHANGED-side row and pressing `s` (undocumented — the footer hint never mentions it).
- **Covers:** `ChangesSidebar.tsx:521-522` (conflict short-circuits the meta), `:178-181` (key path ignores conflict state); `status.rs:147-181` (conflicts land in both lists).
- **Automation:** Backend: build a conflicted fixture and assert the path appears in both `staged` and `unstaged`; frontend RTL asserting no `s`/`d` buttons on a conflicted row.
- **Status:** partial (see SHIP-GAP-22)

---

### Diff view

#### SHIP-23 — Single click previews, double click pins, and a second preview replaces the first
- **Given:** Task view with an agent terminal tab; sheet open with 3 changed files.
- **When:** Single-click file A, then single-click file B, then double-click file C.
- **Then:** After A a `DIFF a` tab appears; clicking B replaces that tab (still one diff tab); double-clicking C leaves B's preview replaced and C's tab persistent, so a subsequent single-click of A opens a *new* preview alongside C's pinned tab.
- **Covers:** `lib/diff-tabs.ts:41-79`; `ChangesSidebar.tsx:502` (`preview: e.detail < 2`).
- **Automation:** RTL over the tabs store, or browser smoke via `window.__diffTabs.open`.
- **Status:** implemented

#### SHIP-24 — Unified/split toggle persists across restarts
- **Given:** A diff open on a modified file (both sides exist) in split mode.
- **When:** Click `unified`, then quit and relaunch and open any diff.
- **Then:** The view mounts in unified mode (single editor with `unifiedMergeView` chunks) — the choice is stored under `view-state:app:diff-mode` and is app-wide, not per tab.
- **Covers:** `store/diffs.ts:44`, `:209-223`; `DiffView.tsx:539-554`.
- **Automation:** Backend `set_view_state`/`get_view_state` round trip + a browser smoke reload.
- **Status:** implemented

#### SHIP-25 — Added and deleted files render as one document with no mode toggle
- **Given:** One untracked new file and one deleted tracked file.
- **When:** Open each diff.
- **Then:** Each shows a single editor with no `unified | split` control; the header carries an `added` badge (old side absent) or a `deleted` badge (new side absent) next to the `unstaged` side badge.
- **Covers:** `DiffView.tsx:334` (`singleDoc`), `:447-459`, `:531-532`.
- **Automation:** RTL with seeded payloads (`oldExists:false` / `newExists:false`).
- **Status:** implemented

#### SHIP-26 — Binary and oversized files refuse to render, with the reason
- **Given:** A changed PNG, and a changed 900 KB text file (> `MAX_DIFF_CONTENT_BYTES` = 512 KB).
- **When:** Open each diff.
- **Then:** The PNG shows "Binary file — preview unavailable."; the large file shows "Diff too large (0.9 MB) — preview unavailable." Neither shows a mode toggle, neither is editable, and the sizes come from the payload (contents are withheld server-side).
- **Covers:** `diff.rs:26-28`, `:108-112`; `DiffView.tsx:566-572`.
- **Automation:** Backend integration test over a temp repo with a NUL-containing file and a >512 KB file.
- **Status:** implemented

#### SHIP-27 — Inline-edit an unstaged hunk and save with ⌘S
- **Given:** An unstaged diff of a tracked text file open and focused, split mode.
- **When:** Type into the right-hand (worktree) editor, then press `⌘S`.
- **Then:** A `●` dirty badge appears in the diff header and on the tab chip while typing; after `⌘S` the badge clears, the file on disk contains the edit, and the resulting `files:changed` refresh does **not** rebuild the editor (cursor, scroll, and undo history survive) because the payload matches the live document.
- **Covers:** FLOWS.md F8-3; `DiffView.tsx:161-176`, `:345-370`; `store/diffs.ts:166-191`; `commands/files.rs`.
- **Automation:** Needs a real browser driver — CodeMirror needs layout, so jsdom RTL cannot drive this. Browser smoke + `window.__diffTabs` seam.
- **Status:** implemented

#### SHIP-28 — An external change while the tab is dirty does not clobber the edit
- **Given:** An unstaged diff with unsaved edits (dirty badge showing).
- **When:** An agent rewrites the same file from a terminal.
- **Then:** The editor keeps the user's in-progress text (the rebuild is skipped while dirty); saving afterwards overwrites the agent's version. There is no conflict prompt and no indication that the on-disk file diverged.
- **Covers:** `DiffView.tsx:368-369` ("An in-progress edit wins over a clobbering rebuild").
- **Automation:** Browser driver: mark dirty, emit `files:changed`, assert the document is unchanged.
- **Status:** implemented (behavior is deliberate; the silent divergence is SHIP-GAP-28)

#### SHIP-29 — A failed save surfaces a header chip and keeps the buffer
- **Given:** An unstaged diff open; the file is made unwritable (`chmod 400`) or the workspace path is removed.
- **When:** Edit and press `⌘S`.
- **Then:** A `save failed` chip appears in the diff header with the raw error as its tooltip, and the dirty badge stays set (the buffer is not lost).
- **Covers:** `store/diffs.ts:186-190`; `DiffView.tsx:534-538`.
- **Automation:** Browser driver with `write_workspace_file` rejecting.
- **Status:** implemented

#### SHIP-30 — Closing a dirty diff tab throws the edit away with no confirm
- **Given:** An unstaged diff with unsaved edits (dirty `●` on the tab).
- **When:** Click the tab's `×`, or press `⌘W`.
- **Then (intended):** A confirm ("unsaved changes"). **Actual:** the tab closes immediately and the edit is gone — `closeTab` has no dirty check and only terminal tabs get special handling.
- **Covers:** `TabBar.tsx:44-54`; `store/tabs.ts:253-299`.
- **Automation:** RTL over the tabs store: mark dirty, close, assert no dialog.
- **Status:** not-built (see SHIP-GAP-30)

#### SHIP-31 — Staged-side diffs are read-only
- **Given:** A staged file; its diff tab open (title carries `(staged)`).
- **When:** Try to type in either editor and press `⌘S`.
- **Then:** Nothing is typed (both sides are `EditorState.readOnly`), no dirty badge appears, and no write is attempted.
- **Covers:** `DiffView.tsx:335-339` (`editable` requires `side === "unstaged"`).
- **Automation:** Browser driver, or a unit assertion on the extension set.
- **Status:** implemented

#### SHIP-32 — Diff tabs survive a restart without sidecar state
- **Given:** Two diff tabs open (one staged, one unstaged) on a task.
- **When:** Quit, relaunch, reselect the task.
- **Then:** Both tabs return with the same titles and re-fetch their payloads — the params are re-parsed from the tab id `diff:<side>:<workspaceId>:<path>`; a path containing colons still resolves.
- **Covers:** `lib/diff-tabs.ts:25-31`; `DiffView.tsx:273-275`, `:320-325`.
- **Automation:** Unit test on `parseDiffTabId` (pure) + a restart smoke.
- **Status:** implemented

---

### Line comments

#### SHIP-33 — Select code → `+ comment` → `↵` adds a note and opens the thread
- **Given:** An unstaged diff open on `src/api.ts`.
- **When:** Select lines 40–44 in the worktree editor, click the `+ comment` chip that appears under the selection, type "handle the 429 case", press `↵`.
- **Then:** The popover closes, a `●` marker appears in the gutter at line 40, and the comment thread panel opens over the diff showing `note` + the comment text; the selection highlight clears.
- **Covers:** design_handoff_v2 §5f; ARCHITECTURE §14; `DiffSelectionPopover.tsx:68-97`; `CommentThread.tsx`.
- **Automation:** Needs a browser driver for the selection; the persistence half is `invoke("add_line_comment")` + `invoke("list_line_comments")`.
- **Status:** implemented

#### SHIP-34 — `⌘↵` turns the selection into a linked task and the badge goes live
- **Given:** Same selection, popover open with text typed.
- **When:** Press `⌘↵`, accept the pre-filled task name in the quick-task dialog, submit.
- **Then:** The comment persists first, then a provisioned task is created and selected; the agent terminal opens in the new worktree pre-loaded with the §14 prompt (file, branch, enclosing function, line range, selected code). Returning to the original task's diff, the comment row reads `note → <task name> · running · now` with a pulsing dot, and the word flips to `done` when the task's status changes — without a refresh.
- **Covers:** FLOWS.md F8-4; `DiffSelectionPopover.tsx:101-134`; `Modals.tsx:614-665`; `commands/line_comments.rs:136-226`; `CommentThread.tsx:98-143`.
- **Automation:** Backend integration test on `create_task_from_comment` (task created, `linked_task_id` and `tasks.source_comment_id` both set, prompt shape); frontend needs the browser driver for the selection.
- **Status:** implemented

#### SHIP-35 — Resolving is manual; a finished linked task never auto-resolves
- **Given:** A comment linked to a task that has reached `done`.
- **When:** Open the thread.
- **Then:** The row shows `→ <task> · done` with a neutral dot but **is not** struck through/resolved; clicking `✓` marks it resolved (`· resolved ✓`, gutter marker flips to `✓`) and the `✓` action disappears.
- **Covers:** ARCHITECTURE §14 decision; ADR-0035 item 2; `CommentThread.tsx:147-155`; `commands/line_comments.rs:110-116`.
- **Automation:** Backend: `resolve_line_comment` + assert a `comment:resolved` event; RTL for the row.
- **Status:** implemented

#### SHIP-36 — Deleting a comment is one unguarded click
- **Given:** A thread with one comment carrying a linked task.
- **When:** Click the red `✗`.
- **Then (intended):** A confirm, given the row is the only pointer back to the review context. **Actual:** the comment is deleted immediately from every open thread and gutter; the linked task survives but is now orphaned from any comment.
- **Covers:** `CommentThread.tsx:156-162`; `commands/line_comments.rs:118-121`.
- **Automation:** RTL: click `✗`, assert `delete_line_comment` invoked with no intervening dialog.
- **Status:** not-built (see SHIP-GAP-36)

#### SHIP-37 — A comment whose linked task was deleted degrades gracefully
- **Given:** A comment linked to a task; the task is deleted via `⌘⌫`.
- **When:** Reopen the diff and the thread.
- **Then:** The row reads `note → task deleted` and the dot falls back to the neutral `todo` state; the comment itself is still there and still resolvable.
- **Covers:** `CommentThread.tsx:143`; ADR-0023 teardown.
- **Automation:** Backend: delete the task, `list_line_comments`, assert the comment row survives; RTL for the copy.
- **Status:** implemented

#### SHIP-38 — Gutter markers ignore which file they belong to
- **Given:** One task with a comment on `a.ts` line 120 and a comment on `b.ts` line 4.
- **When:** Open the diff for `b.ts` (a file with ≥120 lines).
- **Then (intended):** Exactly one marker, at line 4. **Actual:** markers render at **both** line 4 and line 120 — `commentGutter` filters on `sourceSide` only, never on `filePath`; clicking the spurious marker opens a thread that (correctly filtered) does not list it. With markers unsorted by line the `RangeSet.of` call can additionally throw.
- **Covers:** `DiffView.tsx:231-252` (no `c.filePath` check) vs `CommentThread.tsx:47-49` (which does filter); `line_comments/mod.rs:195` orders by `file_path, line_number`, so cross-file ordering is not monotonic.
- **Automation:** RTL/browser: seed two comments as above, count `.lc-marker` nodes.
- **Status:** partial — defect (see SHIP-GAP-38)

#### SHIP-39 — An agent-authored comment lands attributed and live, but no agent can call it
- **Given:** A task with a diff open; the `agent_add_line_comment` command invoked with `provider: "claude"` for an in-range line of an existing file.
- **When:** The command runs.
- **Then:** The comment persists with `createdBy = agent:claude`, a `comment:created` event fires, and the open thread shows `claude · note …` without a refresh. Out-of-range lines, missing files, and `../` paths are rejected with typed errors.
- **Covers:** ADR-0035; `commands/line_comments.rs:72-93`; `line_comments/mod.rs:320-360`; `CommentThread.tsx:127`.
- **Automation:** Backend integration test on the command (all four guardrails) + an event assertion. The *agent-initiated* half has no transport: no MCP tool registration exists, so a running agent cannot invoke it.
- **Status:** partial — host entry point implemented, agent invocation unreachable-entirely (see SHIP-GAP-39)

---

### Commit card

#### SHIP-40 — Commit exactly the staged set with `↵`
- **Given:** 2 staged files and 1 unstaged file; focus in the commit message input; message "fix retry".
- **When:** Press `↵`.
- **Then:** The row label flips to "Committing…", the message clears, both staged rows disappear, the unstaged file remains untouched, and the branch's HEAD advances one commit. The card's disabled state re-derives (Commit is now disabled — nothing staged).
- **Covers:** design_handoff_v2 §5d; `CommitCard.tsx:37-47`, `:96-107`; backend test `commit_commits_exactly_the_staged_set`.
- **Automation:** Backend integration test (exists) + RTL on the card with mocked commands.
- **Status:** implemented

#### SHIP-41 — The disabled matrix: empty message, nothing staged, detached HEAD, no remote
- **Given:** Four setups — (a) staged files but an empty message, (b) a message but nothing staged, (c) a detached HEAD, (d) a repo with no `origin`.
- **When:** Hover each of the three key rows.
- **Then:** (a) and (b) disable Commit and both push rows, with titles "Nothing staged" / the commit title; (c) disables everything with "HEAD is detached — nothing to commit"; (d) leaves Commit enabled but disables `Commit & push` with "No push remote configured" and hides the PR row entirely (`canCreatePr` requires a remote).
- **Covers:** `CommitCard.tsx:29-30`, `:76-85`; `commit.rs:68-118`; backend tests `state_no_remote_configured`, `push_detached_head_errors`.
- **Automation:** Backend integration tests over temp repos (mostly exist); RTL over the four `GitCommitStateDto` shapes.
- **Status:** implemented

#### SHIP-42 — `⌘↵` commits then pushes, setting upstream on the first push
- **Given:** A branch never pushed, `origin` configured, one staged file, a message typed.
- **When:** Press `⌘↵` in the message input.
- **Then:** The row reads "Committing…" then "Pushing…"; afterwards `origin/<branch>` exists and the branch tracks it (`set-upstream` on the first push, plain `git push` thereafter). The commit-state refetch flips `published` so the PR row's tooltip changes from "Push and set upstream on origin" to "Push to origin".
- **Covers:** `CommitCard.tsx:48-56`; `commit.rs:151-171` + test `push_first_time_sets_upstream_second_uses_default`.
- **Automation:** Backend integration test against a bare sibling remote (exists).
- **Status:** implemented

#### SHIP-43 — `⌘⇧↵` opens a GitHub *compare* page — it does not create the PR
- **Given:** A GitHub `origin`, staged changes, a message, no open PR for the branch.
- **When:** Press `⌘⇧↵`.
- **Then:** The card commits, pushes if unpublished, and the OS browser opens `https://github.com/<owner>/<repo>/compare/<branch>?expand=1`. Back in the app the Pull Requests tab still reads "no open pull request for `<branch>`" until the user submits the form in the browser *and* a sync runs. The row label says "Commit, push & open PR"; nothing tells the user the PR is not yet created.
- **Covers:** design_handoff_v2 §5d row 3; `commit.rs:182-231` (compare URL, Phase 0); `CommitCard.tsx:58-70`.
- **Automation:** Backend test `create_pr_published_branch_returns_url_without_pushing` (exists) asserts the compare URL; the browser hand-off is manual.
- **Status:** partial (see SHIP-GAP-43)

#### SHIP-44 — With a PR already open the third row degrades to a note
- **Given:** The sync cache holds an `open` PR whose `head_ref` is the current branch.
- **When:** Open the sheet.
- **Then:** The `Commit, push & open PR` row is replaced by the mono note "PR already open — push instead"; `⌘⇧↵` from the message input does nothing. Invoking `git_create_pr` directly errors with "a pull request is already open: `<url>`".
- **Covers:** `CommitCard.tsx:147-163`; `commit.rs:200-204`; `pr_sync.rs:31-52` `CachedPrLookup`; backend tests `pr_open_guard_via_mocked_lookup`, `create_pr_guard_blocks_when_pr_open`.
- **Automation:** Backend: upsert an open PR into the cache, assert `git_commit_state.canCreatePr === false`.
- **Status:** implemented

#### SHIP-45 — Opening a PR for a branch that is already committed and pushed is impossible
- **Given:** A branch with commits, pushed to a GitHub remote, working tree clean, no PR open.
- **When:** Try to create the PR from anywhere in the app: the commit card, the palette, the PR tab.
- **Then (intended):** A "Create PR" action. **Actual:** the card's PR row is disabled (it inherits `commitDisabled`, which requires ≥1 staged file *and* a non-empty message); the palette has only fetch/pull/push/publish; the PR tab has no create affordance. `git_create_pr` has exactly one caller and it is gated behind a pending commit.
- **Covers:** `CommitCard.tsx:29` + `:147-160`; `commands.ts:217-220`; `PullRequestPanel.tsx` (no create path); grep confirms `gitCreatePr` is called only from `CommitCard.tsx:63`.
- **Automation:** Static: assert the single call site; behavioral: RTL with a clean snapshot asserting `aria-disabled="true"` on the PR row.
- **Status:** unreachable-by-mouse (see SHIP-GAP-45)

#### SHIP-46 — Git failures keep the message and show inline
- **Given:** A pre-commit hook that exits non-zero (or a push rejected as non-fast-forward).
- **When:** Press `↵` (or `⌘↵`).
- **Then:** The phase returns to idle, a red `role="alert"` line under the key rows carries git's stderr verbatim, and — for a commit failure — the typed message is still in the input for retry. For a *push* failure after a successful commit the message is already cleared (the commit did land), which is correct but reads as data loss.
- **Covers:** `CommitCard.tsx:32-56`, `:165-169`; `commit.rs:258-274`.
- **Automation:** Backend: install a failing `pre-commit` hook in a temp repo, assert the error text; RTL for the alert node.
- **Status:** implemented

---

### Git footer and palette verbs

#### SHIP-47 — A repo with zero remotes offers the inline add-remote form
- **Given:** A workspace whose repo has no remotes at all.
- **When:** Open the sheet and scroll to the footer.
- **Then:** A two-field row (`remote` / `remote URL`) with an `add` affordance is shown; typing a name with a space and pressing `↵` shows the inline error `invalid remote name "…" (letters, digits, '.', '_', '-' only)`; a valid pair adds the remote, clears the URL field, refetches, and the form disappears (remotes is no longer empty) while `Commit & push` becomes enabled.
- **Covers:** design_handoff_v2 §5d footer; `GitFooter.tsx:43-78`; `remote.rs:102-124` + test `add_remote_validates_and_adds`.
- **Automation:** Backend test (exists); RTL for the form and the error.
- **Status:** implemented

#### SHIP-48 — fetch / pull / push / publish run from the palette with no visible result
- **Given:** A task selected, sheet open, branch behind its upstream.
- **When:** `⌘K` → "Git: pull" → `↵`.
- **Then (intended):** Some confirmation that the pull happened (and, on a diverged branch, git's "Not possible to fast-forward" message). **Actual:** the palette closes and nothing else happens visibly — success is only inferable from ahead/behind numbers that the footer does not render, and failures go to `console.error`. The same holds for fetch, push, and publish.
- **Covers:** design_handoff_v2 §5d ("git plumbing goes to the palette, not buttons"); `commands.ts:200-220` (`.catch(e => console.error(...))`, `defaultKeys: []`); `remote.rs:59-61` (`pull --ff-only`).
- **Automation:** Backend integration test for `git_pull` on a diverged branch (asserting the error string, which the UI drops); frontend: run the command with a rejecting mock and assert no DOM change.
- **Status:** partial (see SHIP-GAP-48)

#### SHIP-49 — `CommitState` computes ahead/behind that nothing renders
- **Given:** A branch 2 ahead / 1 behind its upstream.
- **When:** Open the sheet after a fetch.
- **Then (intended):** The footer shows `↑2 ↓1` (the DTO carries `upstream`, `ahead`, `behind`, `remotes` explicitly for the footer). **Actual:** the footer renders only the static hint line; the counts are fetched on every `git:changed` and discarded.
- **Covers:** `commit.rs:56-63` ("E4-08 footer: pull/push affordances key off this"); `GitFooter.tsx:84-86`; backend test `ahead_behind_counts_and_ff_pull`.
- **Automation:** Backend test (exists) proves the data; the UI absence is an inspection.
- **Status:** not-built (see SHIP-GAP-49)

---

### Pull Requests tab and checks

#### SHIP-50 — No GitHub token: the tab is a consent gate, not an error
- **Given:** No token in the OS keyring; the workspace has a GitHub remote.
- **When:** Click the `Pull Requests` tab.
- **Then:** The body explains that the token lives in the OS keyring, never in the app database, and offers **Import from gh CLI** and **Paste token** (a password input). Importing stores the token and immediately kicks a sync; the panel re-renders with the PR (or the no-PR state). Failures show inline.
- **Covers:** `PullRequestPanel.tsx:427-509`; `commands/github.rs:148-165`; `github/token.rs`.
- **Automation:** Backend: `github_token_status` / `github_token_set` / `github_token_clear` round trip asserting only the mask is returned; RTL for the gate.
- **Status:** implemented

#### SHIP-51 — Non-GitHub remote and detached HEAD both render the "not GitHub" state
- **Given:** (a) a GitLab `origin`; (b) a detached HEAD on a GitHub repo. Token present in both.
- **When:** Open the Pull Requests tab.
- **Then:** Both show "no GitHub remote on this branch — push to GitHub to see pull requests" (the PR target resolves to `None` in both cases, so a detached HEAD is misreported as a remote problem). No refresh footer is shown.
- **Covers:** `pr_target.rs:29-54` + tests `non_github_remote_is_none`, `detached_head_is_none`; `PullRequestPanel.tsx:132-137`.
- **Automation:** Backend integration tests over temp repos (exist).
- **Status:** partial — copy is wrong for the detached case (see SHIP-GAP-51)

#### SHIP-52 — Branch with no PR shows the branch name and a refresh
- **Given:** GitHub remote, token present, branch pushed but no PR opened.
- **When:** Open the tab.
- **Then:** "no open pull request for `<branch>`" plus a footer with an empty left slot and an `r refresh` button.
- **Covers:** `PullRequestPanel.tsx:138-146`; `commands/github.rs:60-77`.
- **Automation:** Backend: empty cache + a real branch, assert `PrSectionDto { tokenPresent: true, repository: Some, pr: None }`.
- **Status:** implemented

#### SHIP-53 — Checks sort failed-first with durations and a logs link
- **Given:** A cached PR with checks: one `completed/failure` (2m 14s), one `in_progress`, one `completed/success` (41s), one `queued`.
- **When:** Open the tab (it defaults to the `checks` sub-tab).
- **Then:** Rows appear in the order failed → running → passed → queued; the failed row shows a red dot, `failed · 2m 14s · logs` with `logs` linking to the check URL; running shows `running · <elapsed>`; passed shows `passed · 41s`; queued shows `queued` and sinks last. The tab labels carry counts (`files N`, `commits N`, `checks N`, `comments N`).
- **Covers:** design_handoff_v2 §5e; FLOWS.md F8-6; `PullRequestPanel.tsx:19-59`, `:171`, `:336-393`.
- **Automation:** RTL with a seeded `PrDto` — pure ordering/formatting, no network.
- **Status:** implemented

#### SHIP-54 — A running check's elapsed stays fresh between syncs
- **Given:** A PR with one `in_progress` check started 30 s ago; the tab open and left alone.
- **When:** Wait ~60 s without touching anything.
- **Then:** The meta advances (`running · 30s` → `running · 1m`) on a coarse 30 s render tick; the tick stops once no check is running.
- **Covers:** `PullRequestPanel.tsx:178-183`, `:63-68`.
- **Automation:** RTL with fake timers.
- **Status:** implemented

#### SHIP-55 — The footer states the merge verdict, failure-first
- **Given:** Two PRs — (a) `mergeableState: "clean"`, no failing checks; (b) 1 failing check.
- **When:** Open each.
- **Then:** (a) the footer-left reads `mergeable` (or `draft · mergeable` for a draft, `merged`/`closed` for a landed PR); (b) it reads `1 failing — merge blocked` in the bad-text colour, overriding the merge word.
- **Covers:** design_handoff_v2 §5e footer; `PullRequestPanel.tsx:71-89`, `:320-328`.
- **Automation:** RTL over the two DTO shapes.
- **Status:** implemented

#### SHIP-56 — `r` refreshes once, never twice
- **Given:** The Pull Requests tab open with the panel focused.
- **When:** Press `r` once.
- **Then:** The refresh button reads `syncing…` and exactly one `pr_section_sync` is issued — the panel's own handler preventDefaults and the parent skips when `defaultPrevented` is set. Pressing `r` inside the token-paste input types the letter instead.
- **Covers:** `ChangesSidebar.tsx:162-173`; `PullRequestPanel.tsx:110-116`.
- **Automation:** RTL: keydown on the region, assert the invoke count is 1.
- **Status:** implemented

#### SHIP-57 — A PR sync failure is invisible
- **Given:** The tab open on a cached PR; then the stored token is revoked on GitHub (or the machine goes offline, or the account hits its rate limit).
- **When:** Press `r`.
- **Then (intended):** "sync failed — token rejected" / "rate limited until HH:MM". **Actual:** the button flashes `syncing…` and returns; the stale cached PR keeps rendering as if fresh. Nothing anywhere shows the last-sync time or the failure count, and because `tokenPresent` is still true the token gate never reappears.
- **Covers:** `commands/github.rs:95-115` ("Sync failures are logged, never surfaced"); `pr_sync/mod.rs:200-205` (failure counters stored but never read by the UI); `store/pr.ts:43-56` (errors only visible when there is no cached section).
- **Automation:** Backend: point the client at a 401-returning API base and assert `pr_section_sync` still returns `Ok`; frontend RTL asserting no error node.
- **Status:** not-built (see SHIP-GAP-57)

#### SHIP-58 — A merged or closed PR stays "open" in the app forever
- **Given:** A branch with an open PR cached; the PR is merged on GitHub.
- **When:** Press `r` (or let the scheduler run) and reopen the sheet.
- **Then (intended):** The footer flips to `merged` and the commit card's `Commit, push & open PR` row returns. **Actual:** the branch query is `?state=open`, so the merged PR is no longer returned, `fetch_pr_by_branch` yields `None`, the upsert never runs, and the cached row keeps `status = 'open'` — the tab renders a merged PR as open/mergeable and `open_pr_url` keeps the commit card degraded to "PR already open — push instead" indefinitely.
- **Covers:** `github/client.rs:76-88` (`state=open`); `pr_sync.rs:99-106` (`Ok(None) => Ok(())`); `pr_sync/mod.rs:153-169` (guard keys on `status = 'open'`).
- **Automation:** Backend integration test: upsert an open PR, run a sync against a fixture API returning an empty list, assert the row is still `open`.
- **Status:** not-built (see SHIP-GAP-58)

#### SHIP-59 — Every external link in the PR tab is inert
- **Given:** A cached PR rendered in the tab.
- **When:** Click `pr 214`, a commit subject, a check's `logs`, or a PR comment body.
- **Then (intended):** The OS browser opens the GitHub page. **Actual:** these are raw `<a target="_blank">` under a CSP of `default-src 'self'`; nothing opens. Every other outbound link in the app goes through `open()` from `@tauri-apps/plugin-shell` (CommitCard, BoardView, CardDetail, markdown) — this panel is the only place that does not.
- **Covers:** `PullRequestPanel.tsx:195-206`, `:262-266`, `:381-387`, `:301-309`; `tauri.conf.json` CSP; `CommitCard.tsx:10` for the working pattern.
- **Automation:** Static grep is decisive; behaviorally needs the packaged app.
- **Status:** not-built (see SHIP-GAP-59)

#### SHIP-60 — There is no merge affordance anywhere
- **Given:** A PR with all checks green and `mergeableState: "clean"`.
- **When:** Look for a way to merge from the app.
- **Then (intended):** Either a merge action or an explicit "merge on GitHub" link. **Actual:** the footer states `mergeable` and stops; the PR number's link (SHIP-59) is inert, so the app offers no route to landing the change. `FLOWS.md` explicitly flagged the design's `⌘↵ merge` as skipping reality — but the reverse gap (no merge at all, not even a hand-off) was never closed.
- **Covers:** `PullRequestPanel.tsx` (no merge call); no `merge` command in `fartcode-core/src/github/client.rs`.
- **Automation:** Static: no merge command exists in the Tauri command list.
- **Status:** not-built (see SHIP-GAP-60)

#### SHIP-61 — `pr:updated` refreshes the open panel without a keypress
- **Given:** The Pull Requests tab open on a workspace the store already tracks.
- **When:** The sync engine upserts a changed payload (e.g. a check flips to failure).
- **Then:** Within ~50 ms the panel re-reads the cache and the failed check jumps to the top with `N failing — merge blocked` in the footer. An identical payload emits nothing (idempotent upsert), so a quiet PR never causes a re-render.
- **Covers:** `store/pr.ts:87-111`; `pr_sync/mod.rs:61-130` + tests `upsert_is_idempotent_for_identical_payloads`, `upsert_emits_on_change`.
- **Automation:** Backend tests (exist) + a frontend event-emission assertion.
- **Status:** implemented

#### SHIP-62 — The PR tab renders offline, straight from the cache, after a restart
- **Given:** A synced PR; then quit the app and disconnect the network.
- **When:** Relaunch and open the Pull Requests tab.
- **Then:** The full PR (files, commits, checks, comments, merge state) renders immediately from SQLite; the background sync fails silently and the cached view stands. Cursors in `kv` mean the scheduler resumes its backoff where it left off.
- **Covers:** `pr_sync.rs:9-11` (restart safety); `pr_sync/mod.rs:134-149`, `:171-205`; `commands/github.rs:33-44`.
- **Automation:** Backend integration test with a seeded DB and no network.
- **Status:** implemented

---

### Concurrency, restart, and layout

#### SHIP-63 — Two tasks in one project keep separate change lists
- **Given:** Two tasks on separate worktrees of the same repo, both with dirty files; the sheet open.
- **When:** Switch between them with the flyout.
- **Then:** Each shows only its own worktree's rows and its own branch in the header ref; staging in one never moves a row in the other. A shared-git-dir event (a branch ref update) refreshes both, and a per-worktree `index` write refreshes only the owning one.
- **Covers:** `fs_watch/classifier.rs:47-89` + tests `index_change_routes_to_owning_worktree_only`, `shared_branch_ref_fans_out_to_all_sharing_workspaces`; per-workspace store keys.
- **Automation:** Backend classifier unit tests (exist); frontend: two workspace ids in the store, emit one event, assert one refetch.
- **Status:** implemented

#### SHIP-64 — A burst of writes from two agents collapses into one refetch
- **Given:** Two agents writing into the same worktree simultaneously; the sheet open.
- **When:** ~50 file events land inside 200 ms.
- **Then:** `git_status` is invoked once (the 150 ms trailing debounce plus the in-flight dedupe), the list settles on the final state, and the commit card's state refetch rides the same debounce.
- **Covers:** `store/changes.ts:36-38`, `:108-133`; `fs_watch/mod.rs` DEBOUNCE + `MAX_PATHS_PER_EVENT`.
- **Automation:** Frontend: emit N events with fake timers, assert one invoke.
- **Status:** implemented

#### SHIP-65 — Narrow window: the sheet can squeeze the task view to nothing
- **Given:** The window at its 800 px minimum, the project flyout open (56 px rail + 244 px flyout), the sheet dragged to its 640 px maximum.
- **When:** Look at the task pane.
- **Then (intended):** The sheet overlays or the flyout auto-collapses below some breakpoint. **Actual:** the shell grid is `auto minmax(0,1fr) auto` with no media queries anywhere in the stylesheets, so the main column collapses to zero width and the terminal/diff becomes unusable until the user manually shrinks the sheet or closes the flyout.
- **Covers:** `styles.css:172-180`; `ChangesSidebar.tsx:105-108` (400 px floor for detail/chat modes); `useGutterResize(400, 280, 640, -1)`; no `@media (max-width…)` in `styles.css` or `styles/*.css`.
- **Automation:** Browser smoke at 800×600 asserting the `.main` bounding box width.
- **Status:** not-built (see SHIP-GAP-65)

---

## 6 · Task end states: delete, archive, restore, teardown

fartCode has exactly one destructive task path — `⌘⌫` in the task view opens `DeleteTaskConfirm`
(`Modals.tsx:328`), the app's only red action label — and exactly one non-destructive one:
the `a` key *inside that same dialog*. Everything else about ending a task is either dead code
(`update_status` has no caller, so every task is `in_progress` forever), unreachable
(`toggle_pin` has a command, a store action and an ordering contract but no UI caller), or
implicit (archived tasks come back only by typing their name into `⌘K`, which restores them as a
side effect of opening them). Scenarios below are grounded in `app-frontend/src/components/Modals.tsx`,
`lib/commands.ts`, `store/sidebar.ts`, `fartcode-core/src/tasks/deletion.rs`,
`fartcode-app/src/commands/{tasks,projects}.rs`, `watchers.rs` and `indexer.rs`.

---

### Entry points and reachability

#### LIFE-01 — Open the delete confirm from the task view
- **Given:** project `ade` selected, task "Fix the pump" selected (task view rendered, agent terminal focused or not).
- **When:** press `⌘⌫`.
- **Then:** an overlay card appears titled `Delete #<first 8 of task id> Fix the pump?`, with a footer reading `esc cancel · a archive instead` on the left and `⌘⌫ delete` (red, `#c98d8d`) on the right.
- **Covers:** handoff v2 §7a; ADR-0023 item 7; FLOWS §F11.
- **Automation:** RTL component test — seed `useSidebar` with a project+task, render `<Modals/>` + `useCommands()`, dispatch `⌘Backspace`, assert `role="dialog"` `aria-label="Delete task"`.
- **Status:** implemented

#### LIFE-02 — `⌘⌫` at project scope does nothing
- **Given:** project selected, NO task selected (board visible).
- **When:** press `⌘⌫`.
- **Then:** nothing happens — no dialog, no board card removed, no focus change.
- **Covers:** `commands.ts:254` scope `task-view`; `registry.ts:216` `activeScopes`.
- **Automation:** RTL — assert no dialog after the keypress with `selectedTaskId: null`.
- **Status:** implemented (correct, but it means the board — the surface where most work lives — has no task-delete path at all)

#### LIFE-03 — `⌘⌫` while typing does not delete
- **Given:** task view open, caret inside the card-detail title input or the PM chat composer.
- **When:** press `⌘⌫`.
- **Then:** the character/word is deleted in the field; no confirm dialog opens.
- **Covers:** `commands.ts:259` `skipInEditor: true`; `registry.ts:249`.
- **Automation:** RTL — focus an `<input>`, dispatch the chord, assert no dialog.
- **Status:** implemented

#### LIFE-04 — No delete or archive affordance exists on any task surface
- **Given:** a project with one running task; the flyout open, the board rendered, the task view open with two tabs, a board card linked to the task.
- **When:** hover and right-click, in turn: the flyout row, the flyout group label, the task header (`TaskHeader`), the tab-bar chip, the board card, the card-detail header, the card-detail footer.
- **Then (intended):** at least one of these reveals a destructive affordance for the TASK. **Then (actual):** none does. `setDeleteTaskTarget` has exactly one caller in the whole frontend (`lib/commands.ts:263`); the card-detail footer's only destructive key deletes the ISSUE, not the task; right-click on a rail tile deletes the PROJECT.
- **Covers:** FLOWS §F11 ("Design has no delete/archive affordance or confirm anywhere").
- **Automation:** static assertion — `grep -c setDeleteTaskTarget app-frontend/src` must be 1 (the registration) + `TaskHeader.tsx` / `Nav.tsx` / `TabBar.tsx` render no button whose label matches /delete|archive|remove/i.
- **Status:** not-built (this is the area's headline gap — see LIFE-G1)

#### LIFE-05 — The palette is the only mouse path to delete
- **Given:** a task selected; `⌘K` open, query empty.
- **When:** scroll to the row "Delete task" (hint `⌘⌫`) and CLICK it.
- **Then:** the palette closes and the delete confirm opens.
- **Covers:** `CommandPalette.tsx:93` scope filter; `commands.ts:254`.
- **Automation:** RTL — render `<CommandPalette/>` with a selected task, click the row by title, assert `useUi.getState().deleteTaskTarget` is set.
- **Status:** implemented (but undiscoverable: no surface points at it)

#### LIFE-06 — Archive has no entry point of its own
- **Given:** a task selected; `⌘K` open, query "archive".
- **When:** read the result list.
- **Then (intended):** an "Archive task" command row. **Then (actual):** "No matches" for commands — archive is not a registered command, has no chord outside the delete dialog, and appears nowhere else. The only way to archive is `⌘⌫` → press `a` inside a dialog headed "Delete … ?".
- **Covers:** handoff v2 §7a ("`a` archive instead"); ADR-0005 ("archive is the non-destructive alternative").
- **Automation:** RTL — assert no registered command id matches /archive/.
- **Status:** partial (works, but only as a secondary key on a destructive dialog)

---

### What the confirm says

#### LIFE-07 — Live agent is itemized with a pulse dot
- **Given:** task with a live agent terminal (`terminal_list_for_task` returns `kind:"agent", running:true`).
- **When:** open the delete confirm.
- **Then:** the first list row is a pulsing amber `status-dot status-in_progress` followed by "kills the running agent".
- **Covers:** handoff v2 §7a.
- **Automation:** RTL with a mocked `terminalListForTask`; assert the `.fc-confirm-live` row and the dot class.
- **Status:** implemented

#### LIFE-08 — Worktree line and the "branch is kept" line
- **Given:** a worktree-kind task on branch `fartCode/fix-the-pump-a1b2`.
- **When:** open the delete confirm.
- **Then:** a row reads `removes worktree fartCode/fix-the-pump-a1b2`, and a dimmed last row reads `branch fartCode/fix-the-pump-a1b2 is kept`.
- **Covers:** handoff v2 §7a; ADR-0023 item 6 (`delete_branch` defaults false).
- **Automation:** RTL with a mocked `gitCommitState` returning `{branch}`.
- **Status:** implemented (note the label says "worktree <branch name>" — the value shown is the branch, not the path).

#### LIFE-09 — Comment and terminal counts
- **Given:** a task with 3 line comments and 2 open terminals.
- **When:** open the delete confirm.
- **Then:** a row reads `deletes 3 line comments · 2 terminals` (singular forms when count is 1; the row is omitted entirely when both are 0).
- **Covers:** handoff v2 §7a.
- **Automation:** RTL with mocked `listLineComments` + `terminalListForTask`.
- **Status:** implemented

#### LIFE-10 — Project-root task: the confirm itemizes nothing
- **Given:** a task created with `from = project root` (`workspace: "project-root"`, so `task.workspaceId === project.repositoryWorkspaceId`), no agent running, no comments, no terminals.
- **When:** press `⌘⌫`.
- **Then:** the dialog shows the title `Delete #… <name>?` and an EMPTY consequence list — no worktree line, no branch line, no counts. The red `⌘⌫ delete` button is still armed.
- **Covers:** `Modals.tsx:352` `isWorktree`; ADR-0008 `WorkspaceTarget::ProjectRoot`.
- **Automation:** RTL — set `repositoryWorkspaceId === task.workspaceId`, assert `.fc-confirm-list` has no children.
- **Status:** partial (correct data, but "confirm that lists no consequences" is an unspecified state)

#### LIFE-11 — The confirm never names the tmux session
- **Given:** a task whose terminals run under tmux durability (project setting `useTmux` on), tmux sessions `fartCode-<b64>` alive.
- **When:** open the delete confirm.
- **Then (intended, handoff v2 §7a):** a row reads `kills tmux <session>`. **Then (actual):** no tmux row appears at any time.
- **Covers:** handoff v2 §7a; MEMORY "Known gaps (data, not design) — tmux session name not itemized in the delete confirm".
- **Automation:** RTL — assert no rendered text matches /tmux/.
- **Status:** not-built

#### LIFE-12 — The confirm never warns about uncommitted work
- **Given:** a worktree task with uncommitted changes on disk (`git status --porcelain` non-empty in the worktree).
- **When:** open the delete confirm and read every row.
- **Then (intended, ADR-0023 item 5 — "deletion is user-confirmed so the E2-07 dirty-check is bypassed on that path (the confirmation dialog carries the warning)"):** the dialog names the uncommitted work. **Then (actual):** no row mentions it; confirming calls `remove_worktree(..., force = true)` which `rm -rf`s the directory including the dirty tree.
- **Covers:** ADR-0023 item 5; `worktrees.rs:282` (`if !force && …is_worktree_clean`).
- **Automation:** backend — create a task worktree, write an uncommitted file, call `delete_task`, assert the file is gone (proves the bypass); frontend RTL asserts nothing renders the warning.
- **Status:** not-built (the ADR's stated safety net does not exist)

#### LIFE-13 — The confirm never mentions the linked board card
- **Given:** issue "Ship E19" dispatched, so `issues.linked_task_id = <task id>` and its card sits in an agent-step column.
- **When:** open the delete confirm for that task.
- **Then (intended):** a row naming the card that will be unlinked/stranded. **Then (actual):** no such row; on confirm the FK `ON DELETE SET NULL` clears `linked_task_id` and the card is left in its column with no task, silently.
- **Covers:** `migrations/0002_issues.sql:17`; ADR-0032 (board never tears down, ⌘⌫ only).
- **Automation:** backend — dispatch an issue, `delete_task`, assert `issue.linked_task_id IS NULL` and the issue's lane/column is unchanged; frontend asserts the confirm renders no issue title.
- **Status:** partial (backend behaviour is correct and intentional; the confirm hides it)

---

### Confirm interaction

#### LIFE-14 — `esc` cancels and tears down nothing
- **Given:** delete confirm open for a task with a live agent.
- **When:** press `esc`.
- **Then:** the dialog closes; the task is still selected; the agent terminal still streams; `list_tasks` still returns the row.
- **Covers:** `commands.ts:479` `close-modal`; `ui.ts:151`.
- **Automation:** RTL + assert `deleteTask` mock never called.
- **Status:** implemented

#### LIFE-15 — Backdrop click cancels
- **Given:** delete confirm open.
- **When:** click outside the card (on `.modal-backdrop`).
- **Then:** the dialog closes with no deletion (the card itself stops propagation, so clicking inside does nothing).
- **Covers:** `Modals.tsx:455`.
- **Automation:** RTL `userEvent.click` on the backdrop.
- **Status:** implemented

#### LIFE-16 — A failing delete keeps the dialog open with an inline error
- **Given:** delete confirm open; the backend `delete_task` will reject (e.g. the project row was deleted concurrently in another window).
- **When:** press `⌘⌫` / click `delete`.
- **Then:** the button shows `deleting…`, then the dialog STAYS open with a `role="alert"` paragraph carrying the error string; the task is still in the flyout. No toast (the app has no toast system).
- **Covers:** `Modals.tsx:388-399`.
- **Automation:** RTL with `deleteTask` mock rejecting.
- **Status:** implemented

#### LIFE-17 — A remapped delete key is advertised but not honoured inside the dialog
- **Given:** the user rebound `delete-task` to `⌃⌫` in settings (`saveOverride`), then opened the confirm from that new chord.
- **When:** press `⌃⌫` while the dialog is open.
- **Then (intended):** the task is deleted (the footer button is literally labelled `⌃⌫ delete` via `hint("delete-task")`). **Then (actual):** nothing happens — the dialog's own listener hard-codes `e.key === "Backspace" && e.metaKey`; only `⌘⌫` (or clicking the button) works.
- **Covers:** `Modals.tsx:429` vs `Modals.tsx:452`; ADR-0014-era keybinding contract (E14-01/02).
- **Automation:** RTL — apply an override for `delete-task`, render the dialog, dispatch the new chord, assert `deleteTask` not called while the label shows the new chord.
- **Status:** partial

#### LIFE-18 — `a` archives; `A` archives too; typing `a` in a field does not
- **Given:** delete confirm open.
- **When:** (a) press `a`; (b) reopen and press `⇧A`; (c) reopen, focus any input in the page and type `a`.
- **Then:** (a) and (b) both archive and close the dialog; (c) does nothing destructive (guarded by `typingTarget`). The dialog has no text input of its own, so (c) is only reachable from an underlying field that keeps focus.
- **Covers:** handoff v2 §7a; `Modals.tsx:432-440`.
- **Automation:** RTL keyboard events.
- **Status:** implemented (the un-shifted-only intent is not enforced — low risk)

---

### What delete actually does

#### LIFE-19 — Full teardown on confirm
- **Given:** task with a live agent PTY, a tmux-backed shell, a worktree in the pool, 2 line comments, an ACP conversation.
- **When:** confirm the delete.
- **Then:** the task disappears from the flyout and from `list_tasks`; `terminal_list_for_task(taskId)` returns `[]`; `tmux ls` no longer lists `fartCode-<b64 of project:task:terminal:*>`; the worktree directory is gone from disk; `view-state:task:<id>` and `…:tabs` keys are gone; a `task:deleted` event fires.
- **Covers:** ADR-0023 items 2–5; E2-09.
- **Automation:** backend integration — `fartcode-core/tests/task_deletion_integration.rs::delete_removes_worktree_rows_view_state_and_reaps_session` covers rows/worktree/view-state/session; the tmux sweep is `TerminalManager::close_task` (`terminals.rs:591`) and needs a live-tmux integration test we do not have.
- **Status:** implemented

#### LIFE-20 — Sibling task on the same workspace keeps the worktree
- **Given:** two tasks sharing one workspace row (`RepositoryInstance` target).
- **When:** delete one of them.
- **Then:** the deleted task's row is gone but the worktree directory still exists and the surviving task's Changes panel still resolves files.
- **Covers:** ADR-0023 item 4; `deletion.rs:237`; `worktrees.rs:271`.
- **Automation:** `task_deletion_integration.rs::sibling_task_keeps_worktree_but_row_is_deleted`.
- **Status:** implemented

#### LIFE-21 — The project root is never removed
- **Given:** a `project-root` task (workspace kind `project-root`, path = the repo checkout).
- **When:** delete it.
- **Then:** the task row is gone; the repository checkout on disk is untouched; the shared `project-root` workspace row survives for other tasks.
- **Covers:** ADR-0023 item 4/5; `worktrees.rs:258` `CannotRemoveProjectRoot`.
- **Automation:** `task_deletion_integration.rs::project_root_workspace_is_never_deleted`.
- **Status:** implemented

#### LIFE-22 — Double delete and delete-during-provision are safe
- **Given:** two windows/actors; a task mid-provision in one.
- **When:** confirm delete twice in a row (or delete while `provision_task` runs).
- **Then:** the second call returns cleanly; no error dialog; no orphaned worktree row.
- **Covers:** ADR-0023 items 1/4.
- **Automation:** `task_deletion_integration.rs::double_delete_is_idempotent`, `::delete_during_provision_is_safe`.
- **Status:** implemented

#### LIFE-23 — The branch always survives, and nothing can delete it
- **Given:** a task on generated branch `fartCode/x-1234`; delete it.
- **When:** afterwards run `git branch --list 'fartCode/*'` in the project checkout, and search the whole UI for a branch-cleanup affordance.
- **Then:** the branch is still there; the frontend never passes `deleteBranch: true` (`tauri.ts:133` defaults to `null` → backend `unwrap_or(false)`), and no surface offers branch deletion. After N deleted tasks the repo carries N dead branches.
- **Covers:** ADR-0023 item 6; `deletion.rs:156`; `commands/tasks.rs:305`.
- **Automation:** backend — `delete_task` with default options, assert the branch ref still resolves; static assertion that no frontend call site sets `deleteBranch`.
- **Status:** partial (backend capability exists, no caller — unreachable feature)

#### LIFE-24 — The configured teardown script never runs on delete
- **Given:** project settings with `scripts.teardown = "docker compose down"`; a task whose run script started containers.
- **When:** delete the task.
- **Then (intended, ADR-0014 lifecycle scripts):** the teardown script runs before the worktree is destroyed. **Then (actual):** it does not — `TaskDeletionService` only reaps PTYs (`deletion.rs:43` explicitly notes Phase 0 "lacks" teardown scripts) and the confirm never mentions it; the containers keep running against a worktree path that no longer exists.
- **Covers:** ADR-0014; ADR-0023 note on `TEARDOWN_WAIT`.
- **Automation:** backend — configure `scripts.teardown` writing a sentinel file, delete the task, assert the sentinel is absent.
- **Status:** not-built

#### LIFE-25 — A partial teardown is silent
- **Given:** a worktree directory the app cannot remove (e.g. a file inside is held/permission-denied).
- **When:** confirm the delete.
- **Then (intended):** the user learns the worktree survived. **Then (actual):** `delete_task` returns `Ok` — the worktree failure is a `tracing::warn` only (`deletion.rs:141`), the dialog closes, the task vanishes from the UI, and an orphan directory stays in the pool with no in-app trace.
- **Covers:** ADR-0023 item 2 ("the rows are the contract").
- **Automation:** backend — chmod the pool dir read-only, `delete_task`, assert `Ok(())` and that the directory still exists.
- **Status:** partial (deliberate design; unreported to the user)

#### LIFE-26 — Local state is dropped and does not come back after restart
- **Given:** a task with a split pane and 3 tabs, selected, then deleted.
- **When:** restart the app.
- **Then:** the task is not in the flyout, `⌘K` for its name returns no task hit, and the app does not land on it (`load()`'s `validTask` check rejects the persisted id); `useTabs.panesByTask` has no entry (`tabs.ts:448` drops it on `task:deleted`).
- **Covers:** E1-08 restore; `sidebar.ts:87`; `tabs.ts:415`.
- **Automation:** backend `delete_task` + assert `search::query(name)` empty and `view_state` keys gone; RTL for the tabs drop.
- **Status:** implemented

#### LIFE-27 — Board card unlinks and re-dispatch spawns a fresh worktree
- **Given:** issue with a linked task in an agent-step column; the task is deleted.
- **When:** open the card detail.
- **Then:** the header button reads `Dispatch` again (not `Open task`), the `Task` meta row is gone, and dispatching creates a NEW task + worktree rather than reattaching.
- **Covers:** `dispatch.rs:53` reattach guard; `step_engine.rs:696`; `BoardView.tsx:243` reload on `task:deleted`.
- **Automation:** `fartcode-app/tests/dispatch_integration.rs` pattern — dispatch, delete task, dispatch again, assert two distinct task ids.
- **Status:** implemented

---

### Archive

#### LIFE-28 — Archive hides the task from flyout and rail
- **Given:** project with exactly one task, shown in the flyout under "Running"; the rail tile carries an amber dot.
- **When:** `⌘⌫` then `a`.
- **Then:** the flyout shows `nothing running`; the rail tile's dot disappears; the task row still exists in the DB (`archived_at` set); `task:archived` fires.
- **Covers:** handoff v2 §7a; `Nav.tsx:158`/`Nav.tsx:24`; `tasks/mod.rs:415`.
- **Automation:** backend `task_archive` + assert `archivedAt` non-null and the event; RTL for the flyout/rail.
- **Status:** implemented

#### LIFE-29 — Archiving from the task view leaves you inside the archived task
- **Given:** the archived task was the SELECTED task when you pressed `a` (the only way to reach the dialog).
- **When:** archive, then wait ~1s for the `task:archived` round-trip.
- **Then (intended):** the app leaves the archived task (its own `onClose` sets `selectedTaskId: null`). **Then (actual):** the task view comes BACK. `wireSidebarEvents` maps `task:archived` to `s.load()` (`sidebar.ts:260`), `load()` restores `selectedTaskId` from the persisted `view-state:app:sidebar` key — which still holds the archived id because `doArchive` mutates state without `persistSidebarView()` — and `list_tasks` does not filter archived rows, so the id validates. You end up sitting in a task the flyout says does not exist, with no "archived" badge anywhere in `TaskHeader`.
- **Covers:** `Modals.tsx:409-417`; `sidebar.ts:65-104`, `:224`, `:260`; `commands/tasks.rs:248` (`list_by_project` has no archived filter).
- **Automation:** RTL — seed persisted view state with the task id, archive through the dialog, flush the event handler, assert `useSidebar.getState().selectedTaskId` is back to the archived id.
- **Status:** partial (this is a functional dead end — see LIFE-G2)

#### LIFE-30 — Archiving does not stop the agent
- **Given:** a task with a live agent terminal actively burning tokens.
- **When:** `⌘⌫` then `a`.
- **Then (intended):** archive is "non-destructive" but the run should at least be visible or stopped. **Then (actual):** `task_archive` only writes `archived_at` (`commands/tasks.rs:269`); `terminal_list_for_task` still reports `running: true`, the tmux session survives, spend continues — and the flyout/rail no longer show it, so the running agent is now invisible. `⌘.` (stop-agent) is task-view scoped, so the only way to reach it is to un-hide the task.
- **Covers:** ADR-0005 ("the reference reaps the session in 'archive' mode — E2-05"); `tasks/mod.rs:144`.
- **Automation:** backend — open an agent terminal, `task_archive`, assert `list_for_task` still shows `running: true`.
- **Status:** not-built (reference archive-mode teardown was never implemented)

#### LIFE-31 — Archive keeps the worktree, the branch and the tabs
- **Given:** archived task with a worktree, uncommitted changes and 2 persisted tabs.
- **When:** inspect the pool directory, `git branch`, and `view-state:task:<id>:tabs`.
- **Then:** the worktree directory exists with the uncommitted changes intact, the branch resolves, and the tabs view-state row is untouched — so a later restore reopens the same tabs.
- **Covers:** handoff v2 §7a ("worktree + branch survive").
- **Automation:** backend — archive, assert path exists + branch resolves + `view_state::get` returns the tabs blob.
- **Status:** implemented

#### LIFE-32 — An archived task is still reachable (and reusable) from its board card
- **Given:** issue linked to a task that is then archived.
- **When:** open the card detail and click `Open task`; then drag the card onto an agent-step column.
- **Then:** `Open task` switches to the archived task's view; the dispatch/step-launch path reattaches to it (`app.tasks.get(task_id)` ignores `archived_at`) and never clears `archived_at`. The task is now live-in-use and archived at the same time — a state no surface can represent.
- **Covers:** `dispatch.rs:53`; `step_engine.rs:696`; `CardDetail.tsx:218` (`linkedTask` lookup does not filter archived).
- **Automation:** backend — archive a linked task, `issue_dispatch`, assert `reattached: true` and `archived_at` still set.
- **Status:** partial (unspecified behaviour)

---

### Restore

#### LIFE-33 — Restore an archived task via `⌘K`
- **Given:** task "Fix the pump" archived.
- **When:** `⌘K`, type "pump", read the hit, press `↵`.
- **Then:** the result row's hint reads `task · archived — ↵ restores`; on `↵` the palette closes, `task_restore` clears `archived_at`, `task:restored` fires, the flyout shows the task again, and the app navigates to it.
- **Covers:** handoff v2 §7a ("restore via ⌘K"); `CommandPalette.tsx:144-165`; `commands/tasks.rs:279`.
- **Automation:** backend `task_restore` + event assertion; RTL for the palette hint and the `restoreTask` call.
- **Status:** implemented

#### LIFE-34 — There is no way to open an archived task WITHOUT restoring it
- **Given:** two archived tasks; you want to peek at one before deciding.
- **When:** select its `⌘K` hit.
- **Then (intended):** a way to inspect without mutating. **Then (actual):** opening IS restoring — `restoreTask` fires unconditionally before navigation, with no confirm and no undo.
- **Covers:** `CommandPalette.tsx:161`.
- **Automation:** RTL — assert `restoreTask` is called on `↵`.
- **Status:** partial

#### LIFE-35 — A failed restore is silent
- **Given:** the archived task's row was removed by another window; the palette still lists the stale FTS hit.
- **When:** press `↵` on it.
- **Then (intended):** an error. **Then (actual):** `restoreTask(...).catch(() => {})` swallows it (`CommandPalette.tsx:161`), the palette closes, and `selectTask` sets a selection for a task that no longer exists — the task view mounts with an empty pane whose ⌘T/⌘⇧T keys all fail into the console.
- **Covers:** `CommandPalette.tsx:161`; `sidebar.ts:118`.
- **Automation:** RTL with `restoreTask` rejecting; assert no alert rendered and `selectedTaskId` set.
- **Status:** partial

#### LIFE-36 — Archived tasks whose names you forgot are unreachable
- **Given:** 6 archived tasks; `⌘K` open with an EMPTY query.
- **When:** read the list.
- **Then (intended):** some way to enumerate archived work. **Then (actual):** the empty query returns commands only (`search::query` returns `[]` for a blank string, `search.rs:102`); there is no Archive list in the flyout, the board, or settings. An archived task is findable only by typing part of its name.
- **Covers:** MEMORY "pinned/recent/archive tree sections were deleted per the design; ⌘K is the jump surface".
- **Automation:** RTL — palette with empty query, assert zero rows of `itemType === "task"`.
- **Status:** not-built

#### LIFE-37 — Restoring after a restart leaves the Changes panel dead
- **Given:** a task archived, then the app restarted, then the task restored via `⌘K`.
- **When:** edit a file inside the restored task's worktree from an external editor and watch the Changes panel.
- **Then (intended):** the panel refreshes (150 ms coalesced `files:changed`). **Then (actual):** nothing updates — `fs_watch::boot_targets` excludes `archived_at IS NOT NULL` rows (`fs_watch/mod.rs:368`) and only `TaskProvisioned` re-registers a watch (`watchers.rs:38`); `TaskRestored` is not handled, so the workspace stays unwatched until the next restart.
- **Covers:** E4-01 watch lifecycle; `watchers.rs:51`.
- **Automation:** backend — archive, restart the service, restore, touch a file in the worktree, assert no `FilesChanged` event within a timeout.
- **Status:** not-built

---

### Marking work done, and pinning

#### LIFE-38 — There is no "done" for a task
- **Given:** a finished task whose PR merged.
- **When:** look for any way to mark it done: the task header, the flyout row, the palette, the board card, settings.
- **Then (intended):** a completion state. **Then (actual):** `TaskStore::update_status` has zero non-test callers (`tasks/mod.rs:376`; the E17/E18 auto-flip moves the ISSUE's column, never the task's status), every create path passes `initial_status: None` → `in_progress`, so every task fartCode has ever created is `in_progress` forever. Consequences you can see: the flyout's "Running" group grows without bound, "Needs you" and "Recent" are permanently empty, and the rail tile's amber dot never clears while any non-archived task exists.
- **Covers:** ADR-0005 (status set, no allowlist); MEMORY "task.status never changes today — do not derive agent state from it"; `Nav.tsx:24`, `Nav.tsx:158-172`.
- **Automation:** backend — create 3 tasks through `create_task`, assert all `status == "in_progress"` and that no exposed Tauri command can change it; RTL — assert the "Recent" group never renders.
- **Status:** not-built (archive is the de-facto "done", and it is buried inside the delete dialog)

#### LIFE-39 — Pinning is unreachable
- **Given:** a project with 4 tasks; you want one at the top of the task-switch order.
- **When:** look for a pin affordance and try `⌘⌥↓` to see the ordering.
- **Then (intended):** pinning changes the `⌘⌥↑/↓` order. **Then (actual):** `toggle_pin` (backend command), `useSidebar.togglePin` (store action) and `visibleTaskOrder`'s pinned-first branch all exist, but no component calls `togglePin` — the only references outside the store are test mocks. `isPinned` is always false, so the pinned branch of the ordering contract is dead.
- **Covers:** E2-10 ordering contract; `sidebar.ts:186`, `:205-221`; MEMORY "pin data still drives `visibleTaskOrder` (E2-10)".
- **Automation:** static assertion — `togglePin` has no caller under `src/components`; RTL — `visibleTaskOrder` unit test already covers the pinned branch synthetically (`sidebar.test.ts:70`).
- **Status:** unreachable-entirely

---

### Project deletion and its cascade

#### LIFE-40 — Delete a project by right-clicking its rail tile
- **Given:** two projects in the rail, each with tasks.
- **When:** right-click the second tile.
- **Then:** a confirm card appears: `Delete <name>?` with the body "Tasks, worktrees, and rows are torn down. The repository on disk is left untouched." and footer `esc cancel` / `↵ delete`. Confirming removes the tile, drops the project's tasks from the store, selects the first remaining project, and fires `project:deleted`.
- **Covers:** `Nav.tsx:98`; `Modals.tsx:780`; `projects/mod.rs:288`.
- **Automation:** RTL `fireEvent.contextMenu` on the tile + mocked `deleteProject`; backend `projects_integration.rs` for the cascade.
- **Status:** implemented (the only hint is the tile's `title` attribute "right-click to delete" — no context menu, no keyboard path, and the confirm itemizes nothing: no task count, no worktree count)

#### LIFE-41 — Deleting a project leaves its agents and tmux sessions running
- **Given:** project `ade` with 2 tasks, each with a live agent terminal under tmux.
- **When:** delete the project and confirm.
- **Then (intended):** the same teardown a task delete performs, for every task. **Then (actual):** `delete_project` calls only `app.projects.delete` + `step_engine::on_project_deleted` (`commands/projects.rs:27`) — there is no `TerminalManager::close_project`, no `acp.stop_task`, and the SQL cascade emits no per-task `TaskDeleted` events, so nothing unregisters watches or reaps PTYs. `tmux ls` still lists the sessions and the agent processes keep running against a directory that was just `rm -rf`'d.
- **Covers:** ADR-0023 (task-level teardown contract) vs `commands/projects.rs`; `terminals.rs:591` (task-scoped only).
- **Automation:** backend integration with `tauri::test::mock_app()` — open an agent terminal on a task, `delete_project`, assert `terminals.list_for_task` still reports the entry (proves the leak).
- **Status:** not-built

#### LIFE-42 — Deleted project's tasks survive in `⌘K` until the next restart
- **Given:** project `ade` with a task named "Fix the pump"; delete the project; do NOT restart.
- **When:** `⌘K`, type "pump", press `↵`.
- **Then (intended):** no hit. **Then (actual):** the hit is still there — the indexer deletes only the `project` document on `ProjectDeleted` (`indexer.rs:39`) and the cascade fires no `TaskDeleted`, so the task documents linger until the boot backfill. Selecting the hit calls `selectProject(<deleted id>)` then `selectTask(<deleted id>)`, and the app renders a task view for a project and task that no longer exist: empty pane, `⌘T` fails into the console, no way back except clicking another rail tile.
- **Covers:** `indexer.rs:42-59`; `CommandPalette.tsx:157-164`; `sidebar.ts:118`.
- **Automation:** backend — create project+task, `delete_project`, assert `search::query("pump")` still returns the task row; RTL — assert the phantom task view mounts.
- **Status:** not-built (index leak) / unreachable-by-mouse (no exit from the phantom view)

#### LIFE-43 — Two projects with the same folder name share a worktree pool
- **Given:** `~/code/a/ade` and `~/work/b/ade` both added as projects (both named `ade`), each with worktrees under `<defaultWorktreeDirectory>/ade/`.
- **When:** delete the first project and confirm.
- **Then:** the shared pool directory `<defaultWorktreeDirectory>/ade/` is `remove_dir_all`'d, so the SECOND project's worktrees — including uncommitted work — are destroyed. The second project's task rows survive and now point at missing paths; its Changes panel reports "workspace has no local path".
- **Covers:** `projects/mod.rs:320-334` (the code documents this as a known limitation until the segment scheme changes, ADR-0015); `provider.rs:175` `safe_path_segment(&project.name, …)`.
- **Automation:** backend — two projects with identical names, provision a worktree in each, delete one, assert the other's worktree path no longer exists.
- **Status:** partial (documented data-loss path with no confirm text warning about it)

#### LIFE-44 — Deleting a card orphans its task
- **Given:** issue "Ship E19" dispatched — linked task with a worktree and a live agent.
- **When:** open the card detail, click `delete issue`, press `↵` on the inline confirm.
- **Then:** the card vanishes from the board and the sheet closes. The TASK survives, worktree and branch intact, agent still running — reachable only via the flyout's Running group or `⌘K`. The confirm ("Delete this issue?") says nothing about the linked task or the running agent.
- **Covers:** `CardDetail.tsx:532-576`; `issues/mod.rs:741`.
- **Automation:** backend — dispatch, `issue_delete`, assert `tasks.get(task_id)` is `Some` and the terminal entry still `running`.
- **Status:** partial

---

### Concurrency, restart, layout

#### LIFE-45 — Deleting one project's task does not disturb another project's running agent
- **Given:** project A with a running agent on task A1; project B selected with task B1 open.
- **When:** delete B1.
- **Then:** A1's agent keeps streaming, A's rail dot stays amber, A's flyout entry is unchanged; only B's task list changes. (`close_task` filters by `task_id`; the tmux sweep is prefixed `{project}:{task}:terminal:`.)
- **Covers:** `terminals.rs:591`.
- **Automation:** backend integration with two projects/tasks; assert the surviving entry.
- **Status:** implemented

#### LIFE-46 — Delete during a live agent turn
- **Given:** a task whose agent is mid-turn (ACP or PTY).
- **When:** confirm the delete.
- **Then:** `acp.stop_task` runs first, the PTY is cancelled with a 5s bounded wait, then rows are deleted; the terminal tab disappears; the task's linked card is left in its column without a settle (the agent's exit-driven flip finds no linked issue because `linked_task_id` was already `SET NULL`).
- **Covers:** `commands/tasks.rs:302`; `deletion.rs:43` `TEARDOWN_WAIT`; `terminals.rs:398` `flip_for_exited_agent`.
- **Automation:** backend — dispatch, delete mid-run, assert the issue's column is unchanged and no `step:settled` fires.
- **Status:** partial (correct but leaves a card parked/stranded with no user-visible explanation; `step_engine` has `on_issue_deleted` and `on_project_deleted` but no `on_task_deleted`)

#### LIFE-47 — Restart after archive: nothing resurfaces
- **Given:** one archived task and one live task; quit and relaunch the app.
- **When:** observe the landing state.
- **Then:** the app lands on the live task (or the first project), the archived task is absent from the flyout, and its worktree is no longer file-watched (`boot_targets` excludes archived). `⌘K` still finds it by name.
- **Covers:** `fs_watch/mod.rs:360-368`; `sidebar.ts:82-104`.
- **Automation:** backend restart harness — assert `boot_targets` excludes the archived task; frontend `load()` unit test.
- **Status:** implemented

#### LIFE-48 — Narrow window clips the delete confirm
- **Given:** the window resized to ~380 px wide (the app has no responsive breakpoints — `grep @media` finds only `prefers-reduced-motion`).
- **When:** press `⌘⌫`.
- **Then (intended):** the confirm fits, or the backdrop scrolls. **Then (actual):** `.fc-confirm { width: 420px }` with a `display:flex; align-items:center; justify-content:center` backdrop and no padding or `max-width`, so the card overflows both edges and the `⌘⌫ delete` button can sit off-screen.
- **Covers:** `styles/modals.css:19-21`; `styles.css:730-738`.
- **Automation:** browser smoke at 380 px — assert `getBoundingClientRect().right <= innerWidth` for `.fc-confirm`.
- **Status:** partial

#### LIFE-49 — First run: no delete surface at all
- **Given:** a fresh install, no projects (onboarding/empty rail).
- **When:** press `⌘⌫`, then open `⌘K` and type "delete".
- **Then:** nothing happens on the chord; the palette lists no "Delete task" row (task-view scope inactive) and no "Delete project" row (project deletion is not a registered command at all — it exists only as a rail right-click).
- **Covers:** `CommandPalette.tsx:93-99`; `Nav.tsx:98`.
- **Automation:** RTL with empty stores.
- **Status:** implemented (delete-project having no command is itself the gap — LIFE-G8)

---

## 7 · Navigation, search, keyboard, and layout

Everything the operator uses to *get somewhere*: the 56px rail (project tiles + agent dots + `+` + `⌘` settings), the 244px project flyout (Needs you / Running / Recent, `⌘\`/`⌘B`), the `⌘K` palette (fuzzy command list + FTS over projects and tasks), the single keydown dispatcher in `lib/useCommands.ts` with its five scopes, all 29 registered commands, per-pane tab navigation, the Keys pane in App settings, and the responsive behaviour below ~900px. Ground truth read for this section: `components/Nav.tsx`, `components/CommandPalette.tsx`, `components/SettingsModal.tsx`, `lib/commands.ts`, `lib/registry.ts`, `lib/keychord.ts`, `lib/useCommands.ts`, `store/ui.ts`, `store/sidebar.ts`, `store/tabs.ts`, `styles.css`, `fartcode-core/src/search.rs`, `fartcode-app/src/indexer.rs`.

Two things shape most of the edges below. First, **scope precedence is `modal > editor > task-view > project-view > app-view > global`** and conflict detection only fires *within* a scope (`registry.ts:128-150`), so a task-view chord silently shadows a global one with no warning anywhere. Second, **`projectView` is true whenever a project is selected — including inside a task view** (`useCommands.ts:31-40`), so project-scoped commands fire on the task surface.

---

### Command inventory (all 29 registrations in `lib/commands.ts`)

| # | Command id | Default chord | Scope | Scenario |
|---|---|---|---|---|
| 1 | `open-command-palette` | ⌘K | global | NAV-11, NAV-12, NAV-21 |
| 2 | `open-settings` | ⌘, | global | NAV-30 |
| 3 | `new-project` | ⌘⇧N | global | NAV-06 |
| 4 | `toggle-sidebar` | ⌘B, ⌘\ | global | NAV-07, NAV-08, NAV-09 |
| 5 | `toggle-right-panel` | ⌘⇧. | global | NAV-23 |
| 6 | `toggle-changes` | ⌘⇧1 | global | NAV-24 |
| 7 | `git-fetch` | *(unbound)* | global | NAV-22 |
| 8 | `git-pull` | *(unbound)* | global | NAV-22 |
| 9 | `git-push` | *(unbound)* | global | NAV-22, NAV-40 |
| 10 | `git-publish` | *(unbound)* | global | NAV-22, NAV-40 |
| 11 | `add-task` | ⌘N | global | NAV-10, NAV-19 |
| 12 | `toggle-project-chat` | ⌘⇧2 | project-view | NAV-25 |
| 13 | `delete-task` | ⌘⌫ | task-view | NAV-26 |
| 14 | `resume-agent` | ⌘T | task-view | NAV-27 |
| 15 | `new-terminal` | ⌘⇧T | task-view | NAV-27, NAV-33 |
| 16 | `toggle-drawer` | ⌘J | task-view | NAV-27 |
| 17 | `stop-agent` | ⌘. | task-view | NAV-27 |
| 18 | `new-terminal-right-split` | ⌘D | task-view | NAV-34 |
| 19 | `open-omp` | ⌘⇧O | task-view | NAV-28 |
| 20 | `open-conversation` | ⌘⇧A | task-view | NAV-27 |
| 21 | `send-context` | ⌘↵ | task-view | NAV-29 |
| 22 | `previous-task` | ⌘⌥↑ | task-view | NAV-17, NAV-18 |
| 23 | `next-task` | ⌘⌥↓ | task-view | NAV-17, NAV-18 |
| 24 | `close-tab` | ⌘W | task-view | NAV-35, NAV-36 |
| 25 | `split-pane` | ⌘\ | task-view | NAV-09, NAV-34 |
| 26 | `next-tab` | Ctrl+Tab | task-view | NAV-32 |
| 27 | `previous-tab` | Ctrl+⇧Tab | task-view | NAV-32 |
| 28 | `jump-to-tab-1…9` | ⌘1–⌘9 | task-view | NAV-31 |
| 29 | `close-modal` | Esc | modal | NAV-20, NAV-21 |
| — | *(palette-only)* `toggle-resource-monitor` | — | — | NAV-23 |

---

### The rail

#### NAV-01 — Land on the first project on a cold start
- **Given:** two projects exist in the DB; no `view-state:app:sidebar` row (fresh install, onboarding already dismissed).
- **When:** the app launches.
- **Then:** the rail shows the fC mark, one tile per project (first letter of each name), a `+` tile and a `⌘` tile; the first project's tile carries the accent bar (`.rail-tile.active`) and the flyout shows that project's name and `…/two/segments · main`.
- **Covers:** `store/sidebar.ts:100-104` ("lands on an empty (or first) project"); left-nav README "Rail order is stable — never reorder by recency".
- **Automation:** RTL: seed `useSidebar` + render `<Nav/>`; backend side is `list_projects` + `getViewState` returning null.
- **Status:** implemented

#### NAV-02 — First run shows the dashed `+` tile and nothing else
- **Given:** zero projects.
- **When:** the app renders (onboarding skipped/finished).
- **Then:** the rail shows only the mark, a **dashed** `+` tile (`.rail-tile.glyph.dashed`) and the `⌘` tile; no flyout renders at all; the main area shows the `fartCode` wordmark and "Add a project to get started — press ⌘⇧N or the + button".
- **Covers:** left-nav README frame 3a; `Nav.tsx:113-121`, `App.tsx:69-79`.
- **Automation:** RTL component test with `projects: []`.
- **Status:** implemented (3a's "Open a folder ⌘O / Clone from GitHub ⌘⇧O" rows are not built — see NAV-G13)

#### NAV-03 — Agent dot on a rail tile reports the project's worst run
- **Given:** project Beta has one task at `review` and one at `todo`; project Alpha has one task at `in_progress`.
- **When:** the rail renders.
- **Then:** Alpha's tile shows a filled amber pulsing dot (`.status-dot.status-in_progress`); Beta's shows the hollow needs-you ring (`.status-dot.status-needs-you`). Flipping Alpha's task to `done` removes Alpha's dot entirely.
- **Covers:** left-nav README "Agent dot"; `Nav.tsx:23-32, 104-108`.
- **Automation:** RTL: set task statuses in `useSidebar`, assert dot classes.
- **Status:** implemented — but the dot is derived from `task.status`, which never changes while an agent runs (MEMORY.md v2 audit: "task.status never changes today"). The dot therefore reports the *lane*, not the agent, and contradicts `TaskHeader`'s live-terminal dot. See NAV-G01.

#### NAV-04 — Clicking a rail tile switches project and drops the task selection
- **Given:** project Alpha selected with task `t-a` open in the task view.
- **When:** the user clicks Beta's rail tile.
- **Then:** the main area leaves the task view and shows Beta's board; `selectedTaskId` is cleared; Beta's tile is `.active`; the flyout header reads "Beta".
- **Covers:** `Nav.tsx:92-97`, `store/sidebar.ts:110-117`.
- **Automation:** RTL (`Nav.test.tsx` already covers the project half).
- **Status:** implemented

#### NAV-05 — Clicking a rail tile re-opens a collapsed flyout
- **Given:** the flyout is collapsed (`⌘\` pressed earlier), project Alpha selected.
- **When:** the user clicks Beta's rail tile.
- **Then:** Beta is selected **and** the flyout reappears showing Beta's groups.
- **Covers:** MEMORY.md "Rail tile click reopens flyout (2026-08-09)" — user-settled, not a deviation; `Nav.tsx:92-97`.
- **Automation:** RTL — `Nav.test.tsx` "reopens a collapsed flyout when a project tile is clicked" already asserts this.
- **Status:** implemented

#### NAV-06 — `+` tile and ⌘⇧N both open Add project
- **Given:** any state.
- **When:** the user clicks the `+` rail tile, or presses ⌘⇧N, or runs "Add project" from ⌘K.
- **Then:** the Add-project composer opens with a `/path/to/repo` input, a `browse…` button that opens the native directory dialog, `esc cancel` and `↵ add project` in the footer. On success a new tile appears in the rail and it becomes active.
- **Covers:** `commands.ts:138-144`, `Nav.tsx:113-121`, `Modals.tsx:40-120`.
- **Automation:** RTL for the dialog; the native `open({directory:true})` needs the Tauri dialog plugin — mock it.
- **Status:** implemented — but the `+` tile is **Add project**, while the left-nav spec calls it "+ new task". Held open for design review (MEMORY.md deviations list).

#### NAV-06b — Right-click a project tile to delete it
- **Given:** project Beta exists with tasks and worktrees.
- **When:** the user right-clicks Beta's tile.
- **Then:** a confirm card appears: "Delete Beta?" with "Tasks, worktrees, and rows are torn down. The repository on disk is left untouched.", `esc cancel` / `↵ delete`. Confirming removes the tile; selection falls back to `projects[0]`; a failure keeps the card open with the error inline.
- **Covers:** `Nav.tsx:98-101`, `Modals.tsx:520-609, 780-790`.
- **Automation:** RTL can fire `contextMenu` on the tile; the delete itself is `delete_project` + assert `project:deleted`.
- **Status:** implemented — but right-click is the **only** affordance and its sole discovery is the tile's `title` tooltip. There is no `delete-project` command, so it is not in ⌘K. See NAV-G02.

#### NAV-06c — Rail overflows with many projects
- **Given:** 20 projects, window height 900px.
- **When:** the rail renders.
- **Then:** *(intended)* the project tiles scroll within the rail while the `+` and `⌘` tiles stay pinned to their positions.
- **Covers:** left-nav README ("Project tile ×2–5" — the design never specified more).
- **Automation:** RTL with 20 seeded projects + assert `.rail` scrollHeight vs clientHeight; visual check needs a real window.
- **Status:** not-built — `.rail` (`styles.css:273-285`) has no `overflow` and `.rail-tile` is `flex: none`, so tiles push the `rail-spacer` and the `⌘` settings tile past the bottom of the window. See NAV-G03.

---

### The project flyout

#### NAV-07 — ⌘\ and ⌘B both collapse the flyout, and the state survives a restart
- **Given:** a project is selected and the flyout is visible.
- **When:** the user presses ⌘\ **outside** a task view (board focused), or ⌘B anywhere, or clicks the `‹` control in the flyout header.
- **Then:** the flyout disappears; the rail stays with its agent dots; the board reflows to the freed width. Relaunching the app leaves it collapsed.
- **Covers:** left-nav README 3f + "state persists"; `commands.ts:145-151`, `store/ui.ts:84-98, 131-139`.
- **Automation:** RTL for the toggle + a jsdom `localStorage` assertion on `fc:sidebarVisible`; the true restart needs the app.
- **Status:** implemented

#### NAV-08 — Collapsed flyout still has a mouse path back
- **Given:** the flyout is collapsed.
- **When:** the user clicks the **already-active** project's rail tile.
- **Then:** the flyout reappears for that project (no project switch).
- **Covers:** MEMORY.md rail-tile note; `Nav.tsx:92-97`.
- **Automation:** RTL.
- **Status:** implemented

#### NAV-09 — ⌘\ means something different inside a task view
- **Given:** a task is open in the task view, focus on the terminal.
- **When:** the user presses ⌘\.
- **Then:** the pane **splits** (a right pane with a fresh shell appears); the flyout does *not* collapse. Pressing ⌘B collapses the flyout as expected. With focus in a text input instead, ⌘\ collapses the flyout (task-view `split-pane` is `skipInEditor`, so it falls through to global).
- **Covers:** `registry.ts:31-38, 242-256`; `commands.ts:145-151, 422-434`; FLOWS.md §2 "Collapse nav … keep, verify".
- **Automation:** `dispatchKey` unit test with `taskView:true` / `editorFocused:true`.
- **Status:** implemented, but the double meaning is invisible — the Keys pane shows only `chords[0]` (`⌘B`) for `toggle-sidebar`, so `⌘\` appears nowhere in the UI. See NAV-G04.

#### NAV-10 — Flyout groups order worst-first and cap Recent at 5
- **Given:** a project with 1 `review` task, 2 `in_progress` tasks, 8 other non-archived tasks, and 3 archived tasks.
- **When:** the flyout renders.
- **Then:** three group labels appear in the order **Needs you**, **Running**, **Recent**; Needs you has 1 row, Running has 2, Recent has exactly 5 rows sorted by `lastInteractedAt ?? statusChangedAt` descending; archived tasks appear in none of them. Each row shows a status dot, the task name, and `needs you` / `running` / the humanised status plus a coarse elapsed (`now` / `4m` / `2h` / `3d` / `1w`).
- **Covers:** left-nav README "Flyout (2a)"; `Nav.tsx:158-229, 243-254`.
- **Automation:** RTL — `Nav.test.tsx` already asserts group order and archived exclusion.
- **Status:** implemented — "Recent" is a deliberate addition held open for design review (MEMORY.md deviations list); the spec's third group was "Sessions", dropped per FLOWS §3.5.

#### NAV-11 — Empty flyout says "nothing running" and still offers New task
- **Given:** a project whose tasks are all `done`/archived, or a brand-new project with zero tasks.
- **When:** the flyout renders.
- **Then:** with zero non-archived tasks the flyout shows the project name, the `…/path · ref` line, mono "nothing running", and the `+ New task` button at the bottom. (With only `done` tasks the Recent group appears instead, so "nothing running" is shown only when *every* group is empty.)
- **Covers:** left-nav README "when all are empty the flyout shows … 'nothing running'"; `Nav.tsx:199, 231-238`.
- **Automation:** RTL with `tasksByProject[p] = []`.
- **Status:** implemented

#### NAV-12 — Clicking a flyout row opens that task
- **Given:** the flyout shows a Running row for task `t-run`.
- **When:** the user clicks the row.
- **Then:** the main area switches to the task view for `t-run` (header shows `<project> / <task name>`); the flyout stays open; the selection persists across a relaunch.
- **Covers:** `Nav.tsx:205-226`, `store/sidebar.ts:118-127, 82-91`.
- **Automation:** RTL for the click; the restart half needs the app (or a `getViewState` stub returning the saved ids).
- **Status:** implemented

#### NAV-13 — The flyout is not resizable
- **Given:** the flyout is open.
- **When:** the user drags its right edge.
- **Then:** *(intended)* the flyout widens/narrows within a documented range, like the right sheet's gutter handle.
- **Covers:** `lib/useGutterResize.ts` (the right sheet has this affordance); DESIGN.md:261 fixes the flyout at 244px.
- **Automation:** RTL pointer events on a handle that does not exist.
- **Status:** not-built — deliberate per DESIGN.md (fixed 244px). Recorded so a tester does not read the missing handle as a bug.

#### NAV-14 — No path from the task view back to the board except the rail
- **Given:** a task is open in the task view.
- **When:** the user presses Esc, or looks for a back/close control in the 46px task header.
- **Then:** *(intended, per left-nav frame 2b)* `esc` is right-aligned in the task header and returns to the board.
- **Covers:** left-nav README "Task (2b) … with `esc` right-aligned"; `components/TaskHeader.tsx:57-90` has no such control; `close-modal` is modal-scope only, so Esc in a task view does nothing.
- **Automation:** RTL: render `<TaskView/>`, assert an `esc`/back control exists and that Escape clears `selectedTaskId`.
- **Status:** not-built — the only way back to the board is clicking the project's rail tile (NAV-04). See NAV-G05.

---

### ⌘K palette — search half

#### NAV-15 — ⌘K finds a project by substring and jumps to it
- **Given:** projects `acme-web` and `ade`; the palette closed.
- **When:** the user presses ⌘K and types `cme`, waits ~150ms, then presses ↵ on the `acme-web` row.
- **Then:** the row renders with the plain-text hint `project`; the palette closes and `acme-web` becomes the selected project (rail tile active, flyout header updated).
- **Covers:** left-nav README frame 3b; `CommandPalette.tsx:76-87, 141-167`; `fartcode-core/src/search.rs:101-125` (trigram tokenizer, substring match).
- **Automation:** `cargo test` covers `search::query` (`search.rs:183-235`); the UI half is RTL with `apiSearch` mocked.
- **Status:** implemented

#### NAV-16 — A 1- or 2-character query returns no FTS hits
- **Given:** a task named `fix the navbar` exists.
- **When:** the user presses ⌘K and types `na`.
- **Then:** no task/project rows appear (the trigram index cannot match under 3 characters); only fuzzy-matched **command** rows are listed, and if none match the list shows "No matches".
- **Covers:** `search.rs:98-106` (trigram, quoted phrase); `CommandPalette.tsx:132-139, 213`.
- **Automation:** backend: `search::query(&db, "na", 10)` → empty. UI: RTL.
- **Status:** implemented (behaviour is real, but undiscoverable — the palette gives no "keep typing" hint). See NAV-G06.

#### NAV-17 — ⌘K restores an archived task
- **Given:** task `t-old` was archived from the delete confirm (`a`); it is absent from the flyout and the board.
- **When:** the user presses ⌘K, types enough of its name, and presses ↵ on its row.
- **Then:** the row's hint reads `task · archived — ↵ restores`; on ↵ the palette closes, `task_restore` fires, `task:restored` reloads the stores, the task reappears in the flyout/board, and the task view opens on it.
- **Covers:** design_handoff_v2 README:85 "restore via ⌘K"; `CommandPalette.tsx:143-166`, `store/sidebar.ts:255-276`.
- **Automation:** backend command `task_restore` + assert the `task:restored` event; UI is RTL with `restoreTask` mocked.
- **Status:** implemented

#### NAV-18 — Board cards that were never dispatched are invisible to ⌘K
- **Given:** a project with an issue titled `rework the invite email` sitting in Backlog with no linked task.
- **When:** the user presses ⌘K and types `invite`.
- **Then:** *(intended)* the card appears as a hit and ↵ opens its card detail.
- **Covers:** FLOWS.md §5 frame 8h ("⌘K feature hits (↵ → card detail)"); ADR-0038 (`item_type "feature"`).
- **Automation:** backend: create an issue, then `search("invite")` → expect a hit. Today it returns nothing.
- **Status:** not-built — `fartcode-app/src/indexer.rs:28-59` indexes only `ProjectAdded`/`ProjectDeleted`/`TaskCreated`/`TaskDeleted`. A card only becomes searchable once dispatch creates a task named after it (`fartcode-app/src/dispatch.rs:105-114`). See NAV-G07.

#### NAV-19 — Renaming never reaches the index
- **Given:** a task created as `New task` and later retitled on its card.
- **When:** the user searches ⌘K for the new title.
- **Then:** *(intended)* the hit carries the current title.
- **Covers:** `search::update_title` (`search.rs:68-83`) exists for exactly this.
- **Automation:** backend integration test: create → rename → `search`.
- **Status:** unreachable-entirely — `update_title` has **no caller** anywhere (`grep search::update_title` → only `search.rs`), and no task/project rename command exists. See NAV-G08.

#### NAV-20 — Palette results are capped at 8 with no paging or type filter
- **Given:** 30 tasks whose names all contain `fix`.
- **When:** the user presses ⌘K and types `fix`.
- **Then:** at most 8 search rows render below the matching command rows; there is no "more results", no scroll-to-load, and no way to restrict the query to tasks or projects.
- **Covers:** `CommandPalette.tsx:84` (`apiSearch(q, 8)`).
- **Automation:** RTL with a mocked `apiSearch` returning 8; the cap itself is a constant.
- **Status:** implemented (as designed); the ceiling is invisible to the user. Low-severity gap NAV-G09.

---

### ⌘K palette — command half

#### NAV-21 — Empty ⌘K lists every command valid in the current context
- **Given:** (a) no project selected; (b) a project selected, board view; (c) a task open.
- **When:** the user presses ⌘K and types nothing.
- **Then:** the row count is (a) 11, (b) 12, (c) 36 — 10 global commands (`open-command-palette` and `close-modal` are hidden), plus `toggle-project-chat` once a project exists, plus the 24 task-view commands once a task is open, plus the palette-only "Toggle resource monitor (enable/disable)". Rows appear in registration order; the first row is **Open settings**, so ⌘K ↵ opens settings. Each bound command shows its chord as a key cap that reflects any remap.
- **Covers:** FLOWS.md F10 "the palette is also the command registry"; `CommandPalette.tsx:49, 93-130`.
- **Automation:** RTL — render `<CommandPalette/>` with `paletteOpen`, seed `useSidebar`, count `li` elements.
- **Status:** implemented

#### NAV-22 — Fuzzy filter ranks prefix over substring over scattered
- **Given:** the palette open.
- **When:** the user types `tog`.
- **Then:** the toggle commands rank above anything matched only by scattered letters; typing `tgd` still matches "Toggle script drawer" (subsequence); typing `zzz` shows "No matches".
- **Covers:** `CommandPalette.tsx:20-37`.
- **Automation:** unit-test `fuzzyScore` (currently not exported — would need an export or an RTL assertion on row order).
- **Status:** implemented

#### NAV-23 — Arrowing past row 8 loses the selection off-screen
- **Given:** the palette open with a task selected (36 rows) and no query.
- **When:** the user presses ↓ fifteen times.
- **Then:** *(intended)* the highlighted row scrolls into view.
- **Covers:** left-nav README 3b ("selected row `background #202026`").
- **Automation:** RTL — assert the selected `li` is within the scroll container's visible box.
- **Status:** not-built — `.palette-results` is `max-height: 320px; overflow-y: auto` (`styles.css:845-851`) and the keydown handler (`CommandPalette.tsx:184-194`) never calls `scrollIntoView`. The selection walks off the bottom and ↵ runs an invisible command. See NAV-G10.

#### NAV-24 — Git plumbing verbs are palette-only and unbound
- **Given:** a task with a worktree is open (or a project with a repository workspace, no task).
- **When:** the user opens ⌘K and runs "Git: fetch" / "Git: pull" / "Git: push" / "Git: publish branch".
- **Then:** the palette closes and the corresponding `useCommitState` action runs against the active workspace (task worktree first, project root otherwise); the Changes panel's git footer reflects the new ahead/behind counts. Each row renders **without** a key cap (no default chord). With neither a task worktree nor a project workspace, the row still appears and running it does nothing at all.
- **Covers:** design_handoff_v2 README:49-50 "fetch / pull / push in ⌘K"; `commands.ts:190-220`.
- **Automation:** backend commands + assert the changes/commit-state events; UI is RTL with the store mocked.
- **Status:** implemented — but failures are swallowed into `console.error` (`commands.ts:213`) and push/publish have no confirm. See NAV-G11, NAV-G12.

#### NAV-25 — Toggle resource monitor from ⌘K flips the persisted setting
- **Given:** the resource monitor is disabled (default).
- **When:** the user runs "Toggle resource monitor (enable/disable)" from ⌘K.
- **Then:** `set_resource_monitor_enabled(true)` persists and the panel opens sampling CPU/MEM once a second. Running it again disables the setting and hides the panel. ⌘⇧. (`toggle-right-panel`) toggles panel visibility only — pressing it while the setting is disabled shows nothing at all.
- **Covers:** FLOWS.md F10 "resource-monitor entry point"; `CommandPalette.tsx:116-130`, `commands.ts:152-162`, `ResourceMonitor.tsx:44`.
- **Automation:** backend `get/set_resource_monitor_enabled` round-trip; UI is RTL.
- **Status:** implemented — ⌘⇧. is a silent no-op while the setting is off, with no hint that a second command controls it. Low-severity gap NAV-G14.

#### NAV-26 — A palette command can stack a modal underneath the palette
- **Given:** a project is selected and the palette is open with the input focused.
- **When:** the user presses ⌘N (a **global**-scope command, so it fires even with a modal open) and then presses Esc.
- **Then:** the New-task composer appears on top of the palette (both backdrops present); Esc closes the **palette** first (`closeTopModal` checks `paletteOpen` before everything else), leaving the composer open behind an already-dismissed overlay.
- **Covers:** `registry.ts:216-228` (global scope survives `modalOpen`), `store/ui.ts:147-157`.
- **Automation:** RTL — open the palette, dispatch ⌘N on `window`, assert both dialogs render, dispatch Escape, assert the composer survives.
- **Status:** implemented-but-wrong. See NAV-G15.

---

### Scopes, modals, and the dispatcher

#### NAV-27 — Modal open suspends task and project scopes
- **Given:** a task is open and the delete-task confirm is showing.
- **When:** the user presses ⌘T, ⌘J, ⌘1, or ⌘W.
- **Then:** nothing happens — no terminal spawns, no drawer opens, no tab changes. Esc closes the confirm; ⌘K (global) still toggles the palette on top of it.
- **Covers:** `registry.ts:216-228` ("view scopes are suspended while a modal is open").
- **Automation:** `dispatchKey` unit test with `modalOpen:true` — already covered in `registry.test.ts`.
- **Status:** implemented

#### NAV-28 — Esc closes the topmost modal in a fixed order
- **Given:** several dialogs are stacked (e.g. settings open, then the palette).
- **When:** the user presses Esc repeatedly.
- **Then:** the order is palette → quick-task → delete-task → create-task → delete-project → project-settings → settings → add-project.
- **Covers:** `store/ui.ts:147-157`.
- **Automation:** unit test on `useUi.getState().closeTopModal()`.
- **Status:** implemented — but the order is by *registry position*, not by what is visually on top (NAV-26 is the failing case).

#### NAV-29 — Esc does nothing during onboarding
- **Given:** first launch; the onboarding card is showing at the Welcome step.
- **When:** the user presses Esc.
- **Then:** *(intended)* the card dismisses and records completion.
- **Covers:** left-nav README "esc closes any overlay or focused view"; `store/ui.ts:159-172` counts `onboardingOpen` in `modalOpen()` but `closeTopModal` (`ui.ts:147-157`) has no branch for it.
- **Automation:** RTL — render `<Onboarding/>` with `onboardingOpen:true`, dispatch Escape, assert it unmounts.
- **Status:** not-built — every step has a `skip` button so this is not a dead end, but Esc is inert while onboarding suppresses every view-scoped key. See NAV-G16.

#### NAV-30 — App chords keep working while a terminal is focused
- **Given:** a task view with a terminal tab focused (cursor in xterm).
- **When:** the user presses ⌘K, ⌘W, ⌘⇧T, ⌘1.
- **Then:** all four fire as app commands (the palette opens, the tab closes, a terminal opens, tab 1 activates) — the keystrokes are **not** delivered to the shell.
- **Covers:** `useCommands.ts:22-29` (the xterm helper textarea is explicitly excluded from "editor").
- **Automation:** RTL — dispatch a keydown whose target carries `class="xterm-helper-textarea"` and assert `dispatchKey` returns the command id.
- **Status:** implemented

#### NAV-31 — `skipInEditor` commands yield to a focused text field
- **Given:** a task view with the PM/task chat composer focused.
- **When:** the user presses ⌘1, ⌘W, ⌘⌥↓, ⌘⌫.
- **Then:** none of them fire (the editor keeps them); ⌘K, ⌘,, ⌘↵ and Ctrl+Tab still fire.
- **Covers:** `registry.ts:84-88, 249`.
- **Automation:** `dispatchKey` unit test with `editorFocused:true` — covered in `registry.test.ts`.
- **Status:** implemented

#### NAV-32 — ⌘⇧2 fires in the task view and opens the wrong panel
- **Given:** a task is open in the task view.
- **When:** the user presses ⌘⇧2 (or runs "Toggle project chat panel" from ⌘K, which is offered because `projectView` is true).
- **Then:** *(intended)* nothing happens — the PM chat is a project-scope surface. **Actual:** the right sheet opens showing **Changes**, because the command sets `changesOpen:true` + `projectChatOpen:true`, but `ChangesSidebar` gates the chat on `!taskId` (`ChangesSidebar.tsx:85`).
- **Covers:** `useCommands.ts:31-40` (`projectView: sb.selectedProjectId !== null`), `commands.ts:223-239`, `CommandPalette.tsx:96`.
- **Automation:** RTL — select a task, dispatch ⌘⇧2, assert the sheet's contents.
- **Status:** implemented-but-wrong. See NAV-G17.

#### NAV-33 — Key repeat never double-fires a command
- **Given:** any view.
- **When:** the user holds ⌘K down for two seconds.
- **Then:** the palette toggles exactly once (`e.repeat` events are dropped).
- **Covers:** `registry.ts:238`.
- **Automation:** `dispatchKey` unit test with `repeat:true` — covered in `registry.test.ts`.
- **Status:** implemented

---

### Keybinding customisation (Settings → Keys)

#### NAV-34 — Keys pane groups every command by scope label
- **Given:** settings open.
- **When:** the user clicks **Keys**.
- **Then:** rows appear under **Everywhere** (11), **Project open** (1), **Task open** (24), **Dialogs** (1); the four git verbs render with the button label `unbound`; the footer reads "click a binding · press the new chord" with a "clear custom bindings" action.
- **Covers:** `SettingsModal.tsx:17-26, 80-151`.
- **Automation:** RTL — render `<SettingsModal/>`, click Keys, count rows per group.
- **Status:** implemented (there is no **Editor focused** group because no command is registered in `editor` scope).

#### NAV-35 — Remap, then verify the new chord fires and the old one does not
- **Given:** Keys pane open.
- **When:** the user clicks the `⌘J` chip on "Toggle script drawer", presses ⌘⌥J, closes settings, opens a task, and presses ⌘J then ⌘⌥J.
- **Then:** the row shows `⌘⌥J` with a `custom` tag and a `↺` reset control; in the task view ⌘J does nothing and ⌘⌥J toggles the drawer; the task header's script-launcher tooltips now read `(⌘⌥J)`. Relaunching the app keeps the remap.
- **Covers:** `SettingsModal.tsx:37-78`, `useCommands.ts:87-102` (`view-state:app:keybindings`), `store/ui.ts:145` (`bumpBindings` re-renders hints).
- **Automation:** RTL for the capture + `getViewState/setViewState` assertions; the restart half needs the app.
- **Status:** implemented

#### NAV-36 — A conflicting chord is refused with a named reason
- **Given:** Keys pane open; ⌘J is bound to "Toggle script drawer".
- **When:** the user clicks the chip on "Stop agent" and presses ⌘J.
- **Then:** an error line appears: `⌘J is already bound to "Toggle script drawer".` and the row keeps `⌘.`; Esc cancels the capture.
- **Covers:** `SettingsModal.tsx:54-72`.
- **Automation:** RTL.
- **Status:** implemented

#### NAV-37 — Swapping two chords silently reverts the second edit
- **Given:** Keys pane open, defaults in force.
- **When:** the user (1) rebinds "Jump to tab 2" from ⌘2 to ⌘0, then (2) rebinds "Jump to tab 1" from ⌘1 to ⌘2.
- **Then:** *(intended)* tab 1 = ⌘2, tab 2 = ⌘0. **Actual:** step 2 is accepted by the UI's conflict check (nothing currently holds ⌘2), saved, and then thrown away — `saveOverride` calls `resetToDefaults` and replays overrides in **registration order**, so `jump-to-tab-1`'s ⌘2 is compared against `jump-to-tab-2`'s freshly reset **default** ⌘2, hits a conflict, and is dropped with only a `console.warn`. The row re-renders as `⌘1`, no `custom` tag, no error shown.
- **Covers:** `useCommands.ts:87-102`, `registry.ts:180-207`.
- **Automation:** pure unit test — `resetToDefaults` + `applyUserOverrides` with `{jump-to-tab-1:["⌘2"], jump-to-tab-2:["⌘0"]}` and assert the resulting bindings.
- **Status:** implemented-but-wrong. See NAV-G18.

#### NAV-38 — A task-scope remap can shadow ⌘K with no warning
- **Given:** Keys pane open.
- **When:** the user rebinds "Close tab" (task scope) to ⌘K, then opens a task and presses ⌘K.
- **Then:** *(intended)* the remap is refused, or at minimum the user is warned that it shadows the palette. **Actual:** the remap is accepted (conflict detection is same-scope only), and inside any task view ⌘K closes the active tab instead of opening the palette. Settings is still reachable via the `⌘` rail tile, so recovery is possible.
- **Covers:** `registry.ts:128-150, 191-198` (`other.scope === cmd.scope`), `registry.ts:31-38` (precedence).
- **Automation:** unit test on `applyUserOverrides` + `dispatchKey` with `taskView:true`.
- **Status:** implemented-but-wrong. See NAV-G19.

#### NAV-39 — There is no way to unbind a command or give it two chords
- **Given:** Keys pane open; "Toggle project flyout" shows `⌘B` (its second default `⌘\` is invisible).
- **When:** the user tries to remove a binding entirely, or to add a second chord to a command.
- **Then:** *(intended)* an explicit unbind control and a way to add alternates.
- **Covers:** `SettingsModal.tsx:73` (`saveOverride(commandId, [oneChord])` always writes exactly one), `registry.ts:171` (`hint` shows only `chords[0]`).
- **Automation:** RTL — assert an unbind control exists.
- **Status:** not-built — `↺` restores defaults, which is the only escape. See NAV-G04, NAV-G20.

#### NAV-40 — "clear custom bindings" restores every default
- **Given:** three commands remapped.
- **When:** the user clicks "clear custom bindings".
- **Then:** every row loses its `custom` tag and shows its default chord; `view-state:app:keybindings` is written as `{}`; the old chords stop working immediately (no relaunch needed).
- **Covers:** `useCommands.ts:104-109`.
- **Automation:** RTL + `setViewState` assertion.
- **Status:** implemented — there is no confirm on this bulk reset. Low-severity gap NAV-G21.

---

### Task-view command coverage

#### NAV-41 — ⌘1–⌘9 jump to a pane's Nth tab
- **Given:** a task with 3 terminal tabs in the left pane, tab 1 active.
- **When:** the user presses ⌘3, then ⌘7.
- **Then:** ⌘3 activates the third tab (its chip gets `.active`); ⌘7 does nothing (no seventh tab). With a split, the jump applies to whichever pane is active (`activePaneByTask`).
- **Covers:** design_handoff_v2 README:15 "⌘1–9 stays tab nav"; `commands.ts:461-476`, `store/tabs.ts:304-319`.
- **Automation:** RTL with `useTabs` seeded, dispatch chords on `window`.
- **Status:** implemented

#### NAV-42 — Ctrl+Tab cycles tabs with wraparound
- **Given:** 3 tabs, tab 3 active.
- **When:** the user presses Ctrl+Tab, then Ctrl+⇧Tab twice.
- **Then:** the active tab goes 3 → 1 → 3 → 2. Neither command is `skipInEditor`, so they also fire from a focused text field.
- **Covers:** `commands.ts:435-460`, `store/tabs.ts:321-336`.
- **Automation:** RTL / direct `useTabs.cycleTab` unit test.
- **Status:** implemented

#### NAV-43 — ⌘W closes the active tab and kills its shell with no confirm
- **Given:** a task whose only tab is a live agent terminal mid-run.
- **When:** the user presses ⌘W.
- **Then:** *(intended)* a confirm naming the running agent, mirroring the delete-task confirm's "kills the running agent" line. **Actual:** the tab vanishes, `terminal_close` kills the PTY immediately, the pane falls back to the "nothing running" empty state, and the run is gone.
- **Covers:** `commands.ts:407-421`, `store/tabs.ts:292-301`, `lib/terminals.ts` `killTerminal`; contrast `Modals.tsx:468-473` (the delete confirm *does* itemise this).
- **Automation:** RTL asserting a confirm renders; today `terminalClose` is called synchronously.
- **Status:** implemented without a confirm. See NAV-G22.

#### NAV-44 — ⌘W on a diff tab with unsaved edits discards them silently
- **Given:** a diff tab showing a dirty dot (`dirtyByTab`), unsaved editor changes.
- **When:** the user presses ⌘W.
- **Then:** *(intended)* a save/discard prompt. **Actual:** the tab closes, `useDiffs.dirtyByTab[tabId]` is dropped, and the edits are lost.
- **Covers:** `store/tabs.ts:253-302` (only terminals get special handling), `store/diffs.ts:128-137`, `TabBar.tsx:39-43`.
- **Automation:** RTL — seed a dirty diff tab, close it, assert a prompt.
- **Status:** implemented without a confirm. See NAV-G23.

#### NAV-45 — ⌘⌥↓ walks tasks across project boundaries
- **Given:** project Alpha with tasks A1, A2 and project Beta with tasks B1, B2; A2 selected; the flyout shows only in-flight work so B1/B2 are not visible anywhere.
- **When:** the user presses ⌘⌥↓.
- **Then:** the app switches to **B1 in project Beta** — the rail's active tile changes and the flyout re-renders — even though nothing in the UI suggested B1 was "next". Pressing ⌘⌥↓ from the last task wraps to the first.
- **Covers:** `commands.ts:107-117`, `store/sidebar.ts:205-221` (`visibleTaskOrder` walks every project).
- **Automation:** unit test on `visibleTaskOrder` + `switchTask`.
- **Status:** implemented — the ordering contract is invisible: it is the *old* tree order (pinned first, collapsed projects skipped), and neither pinning nor collapsing has any UI. See NAV-G24.

#### NAV-46 — Pinning and collapsing, which define ⌘⌥↑/↓ ordering, cannot be reached
- **Given:** any project with tasks.
- **When:** the user tries to pin a task or collapse a project.
- **Then:** *(intended)* pinned tasks sort first in the ⌘⌥↑/↓ walk and collapsed projects are skipped.
- **Covers:** `store/sidebar.ts:132-135` (`toggleCollapsed`), `:186-196` (`togglePin`), both consumed by `visibleTaskOrder:205-221`.
- **Automation:** the store functions are unit-testable; there is no UI to drive.
- **Status:** unreachable-entirely — neither `togglePin` nor `toggleCollapsed` has a caller outside the store and its own test (`grep` confirms). The backend `task_toggle_pin` command exists and is unused. See NAV-G25.

#### NAV-47 — Task-view commands fail silently when the underlying call rejects
- **Given:** a task view; the `omp` binary is not installed on the host.
- **When:** the user presses ⌘⇧O.
- **Then:** *(intended)* an inline error naming the missing agent, with a path to the Agents list in settings. **Actual:** nothing visible happens — the rejection lands in `console.error("omp open failed", …)`. Same shape for ⌘T (`resume-agent`), ⌘⇧T (`new-terminal`), ⌘D, and the four git verbs.
- **Covers:** `commands.ts:279-281, 294-296, 342-343, 357-359, 213`.
- **Automation:** RTL with `terminalOpenAgent` rejecting; assert something renders.
- **Status:** implemented without error surfacing. See NAV-G11.

#### NAV-48 — ⌘T refuses while setup is running and opens the drawer after a failed setup
- **Given:** a task whose `setup` lifecycle script is (a) still running, then (b) exited non-zero.
- **When:** the user presses ⌘T.
- **Then:** (a) nothing spawns and the empty state still reads "Waiting on setup before starting…"; (b) the ⌘J drawer opens on the **setup** tab instead of spawning an agent.
- **Covers:** design_handoff_v2 7b; MEMORY.md v2 audit ("⌘T refuses during setup and opens the drawer after a failed one"); `commands.ts:60-80`.
- **Automation:** RTL with `useScripts` seeded; the spawn path needs `terminalOpenAgent` mocked.
- **Status:** implemented

#### NAV-49 — ⌘↵ only sends on the ACP path
- **Given:** (a) a task whose conversation resolved to the ACP runtime with a non-empty draft; (b) a task on the TUI/PTY path.
- **When:** the user presses ⌘↵ from the composer.
- **Then:** (a) the draft is sent and appears in the transcript; (b) nothing happens — by design, the TUI terminal path stays byte-identical.
- **Covers:** `commands.ts:369-390`.
- **Automation:** RTL with `useConversations` seeded both ways.
- **Status:** implemented — case (b) is an intentional silent no-op with no user-facing explanation.

#### NAV-50 — Two concurrent projects each keep their own agent dot and tab state
- **Given:** project Alpha has a running agent in task A1; project Beta has a task B1 in `review`; both tasks have been opened at least once.
- **When:** the user clicks Alpha's tile, opens A1, splits with ⌘D, clicks Beta's tile, opens B1, then returns to A1.
- **Then:** A1 still shows its split with both terminals alive and the same active tab; Beta's rail dot stays hollow amber while Alpha's stays filled; neither project's tab state leaks into the other (state is keyed by task id under `view-state:task:<id>:tabs`).
- **Covers:** `store/tabs.ts:79-85, 201-211`; `Nav.tsx:23-32`.
- **Automation:** RTL for the store keying; the live-PTY half needs the app.
- **Status:** implemented

---

### Layout and responsiveness

#### NAV-51 — The rail does not narrow below ~900px
- **Given:** the window resized to 820px wide (the Tauri `minWidth` is 800).
- **When:** the layout reflows.
- **Then:** *(intended)* the board collapses to one column **and the rail narrows to 48px with 30×30 tiles**.
- **Covers:** DESIGN.md:261-265 ("Under ~900px the board collapses to one column and the rail narrows to 48px"); left-nav README "Narrow (4g)"; `fartcode-app/tauri.conf.json:19` (`minWidth: 800`).
- **Automation:** needs a driver we lack for the real window; a jsdom test could assert a `@media` rule or a measured class exists.
- **Status:** partial — the **board** half is built and measured, not media-queried (`BoardView.tsx:80, 305-316`, `NARROW_PX = 900`, ResizeObserver, `.board-narrow` strip in `styles/board.css:502-592`). The **rail/flyout** half is not: `styles.css` contains no `@media` rule other than two `prefers-reduced-motion` blocks (`styles.css:604, 1179`). See NAV-G26.

#### NAV-52 — At the minimum window width the chrome crowds out the work surface
- **Given:** the window at 800px, the flyout open, and the right sheet open at its 400px default.
- **When:** the board renders.
- **Then:** rail 56 + flyout 244 + sheet 400 = 700px of chrome leaves ~100px for the board; the board's ResizeObserver correctly flips it to narrow mode, but the single column is unusable.
- **Covers:** `styles.css:172-180` (`grid-template-columns: auto minmax(0,1fr) auto`), `styles/project-chat.css:11-12` (400/280), `ChangesSidebar.tsx:72` (`useGutterResize(400, 280, 640, -1)`).
- **Automation:** needs a driver we lack (real window resize); the widths are assertable in a jsdom layout test only loosely.
- **Status:** partial — nothing auto-collapses the flyout or the sheet as width drops; recovery is manual (⌘B, ⌘⇧1). See NAV-G27.

#### NAV-53 — Panel widths do not survive a relaunch
- **Given:** the user drags the right sheet's gutter to 560px (or uses ←/→ on the focused separator, or double-clicks to reset).
- **When:** the app is relaunched.
- **Then:** *(intended)* the sheet reopens at 560px. **Actual:** it reopens at 400px.
- **Covers:** `lib/useGutterResize.ts:1-4` ("Widths are in-memory only"); contrast the flyout's collapsed state, which *does* persist (`store/ui.ts:84-98`).
- **Automation:** needs the app for the restart; the in-memory-only fact is assertable from the hook.
- **Status:** not-built. See NAV-G28.

#### NAV-54 — Keyboard focus reaches every rail and flyout control
- **Given:** the app focused, nothing else focused.
- **When:** the user presses Tab repeatedly.
- **Then:** focus walks the rail's project tiles, the `+` tile and the `⌘` tile, then the flyout's collapse `‹`, its rows and `+ New task`, each showing the 2px accent focus ring (`styles.css:146-151`); Enter/Space activates the focused control.
- **Covers:** DESIGN.md focus token; `Nav.tsx` (all controls are real `<button>`s).
- **Automation:** RTL `userEvent.tab()`.
- **Status:** implemented

#### NAV-55 — Clicking a rail tile does not drag the window
- **Given:** the real Tauri window; `.rail` carries `data-tauri-drag-region="deep"`.
- **When:** the user mouse-downs on a project tile and moves the pointer.
- **Then:** the project is selected and the window does **not** move; mouse-down on the rail's empty background *does* drag the window, and a double-click there toggles maximize.
- **Covers:** `Nav.tsx:48, 183`; Tauri 2.11.5 `window/scripts/drag.js:51-70` (clickable elements block the drag before the `deep` ancestor is reached).
- **Automation:** needs a driver we lack (real window); RTL cannot exercise Tauri's drag script.
- **Status:** implemented (verified by reading Tauri's drag script; worth one manual pass).

---

### App settings

#### NAV-56 — ⌘, and the `⌘` rail tile open the same surface
- **Given:** any state.
- **When:** the user presses ⌘, or clicks the `⌘` rail tile.
- **Then:** the settings card opens on the **App** section; the rail tile renders `.active` while it is open; the pane header shows `App` and an `esc` button that reflects the current `close-modal` binding; the left nav lists App, Keys, and one row per project.
- **Covers:** `commands.ts:131-137`, `Nav.tsx:124-132`, `SettingsModal.tsx:158-224`.
- **Automation:** RTL.
- **Status:** implemented — the design (3e) specifies a full-window pane with the rail tile active, not a floating card over a scrim; held open for design review (MEMORY.md deviations list).

#### NAV-57 — The App pane lists detected agents and re-detects on every open
- **Given:** `claude` installed, `codex` not.
- **When:** the user opens Settings → App.
- **Then:** a **Detected** group lists each agent: installed rows show `version · ~/bin-dir` and the default carries a green `default` tag; missing rows show `not found · install`; the tail line reads `+ N more in the registry · M acp`. Installing an agent outside the app and reopening the pane shows it as installed.
- **Covers:** design_handoff_v2 7d; `AgentsList.tsx:83-117`.
- **Automation:** RTL with `useDependencies` seeded; the real detection is a backend command.
- **Status:** implemented — an in-flight install shows a bare `installing` with no progress (the runner emits no progress events; MEMORY.md known gaps).

#### NAV-58 — The app-wide default agent can only be changed from a project pane
- **Given:** at least one project exists.
- **When:** the user opens Settings → App and looks for a default-agent picker.
- **Then:** *(intended)* the app-wide default agent (`settings::DEFAULT_AGENT_ID`, consumed by ⌘T at `commands.ts:69-72`) is settable from the **App** pane. **Actual:** the App pane only *displays* which agent is default; the picker lives in Settings → *&lt;project&gt;* → Agent → "default agent" (`ProjectSettings.tsx:243-252, 505-530`), so with zero projects there is no way to set it at all.
- **Covers:** `fartcode-app/src/commands/settings.rs:97-105`; `fartcode-core/src/settings/registry.rs:204`.
- **Automation:** RTL — assert a picker in the App pane.
- **Status:** partial / misplaced. See NAV-G29.

#### NAV-59 — Telemetry opt-out is nowhere in settings
- **Given:** Settings open.
- **When:** the user looks for a telemetry / privacy toggle.
- **Then:** *(intended per PRD E15-01)* a Settings toggle plus the `TELEMETRY_ENABLED` env override, with dev builds silent.
- **Covers:** PRD.md:442-450 (E15); FLOWS.md F12 "App settings: keybindings (E14-02), telemetry opt-out (E15)"; `fartcode-telemetry/src/lib.rs:1` ("Phase 0 placeholder"), `fartcode-core/src/tasks/lifecycle.rs:30-33` (hooks log to `tracing::debug!` only).
- **Automation:** RTL — assert a toggle exists.
- **Status:** not-built — nothing is transmitted today, so this is a missing *consent surface*, not a leak. See NAV-G30.

#### NAV-60 — The standalone project-settings modal cannot be opened
- **Given:** any state.
- **When:** the user tries to reach the `projectSettingsOpen` dialog (a gear, a command, a palette entry).
- **Then:** *(intended)* some affordance opens it.
- **Covers:** `store/ui.ts:53, 130, 154, 165` and `Modals.tsx:771-779` render and close it; `grep setProjectSettingsOpen` finds **no** caller outside `Modals.tsx` itself. The former `ProjectHeader.tsx` (which held the gear) is deleted in the working tree.
- **Automation:** static — grep for callers.
- **Status:** unreachable-entirely (dead state + a dead `closeTopModal` branch). Project settings are still reachable via Settings → project pane. See NAV-G31.

#### NAV-61 — No keyboard-shortcut sheet
- **Given:** any view.
- **When:** the user presses `?`.
- **Then:** *(intended, left-nav frame 4h)* a two-column sheet opens with groups Do / Review / Move / Window, label left and mono chord right.
- **Covers:** left-nav README:168 "Keys (4h)"; no `?` handler exists anywhere in `app-frontend/src`.
- **Automation:** RTL — dispatch `?` and assert a sheet.
- **Status:** not-built — ⌘K's command list and Settings → Keys cover the information, so this is a discovery gap rather than a dead end. See NAV-G32.

---

## 8 · Cross-cutting: persistence, failure, concurrency, consent

This section covers the seams no single surface owns: what survives a relaunch (and what is
deliberately in-memory so a gate cannot be bypassed), how the app behaves when the agent CLI,
the network, the keyring, the worktree or the DB is missing, what happens when two agents or
two projects act at once, and — the largest cluster — every place fartCode can start an agent
(i.e. spend tokens) with or without asking. Everything below was read out of the shipped code:
`app-frontend/src/App.tsx`, `store/{ui,sidebar,tabs,scripts,dependencies}.ts`,
`lib/{terminals,commands,columnConfig}.ts`, `components/{Onboarding,TaskView,ChangesSidebar,
PullRequestPanel}.tsx`, `components/board/BoardView.tsx`, `components/projectChat/
ProjectChatPanel.tsx`, and backend `fartcode-app/src/{lib,app,step_engine,dispatch,terminals}.rs`,
`fartcode-core/src/pty/launcher.rs`, `fartcode-git/src/pr_sync.rs`. Scenarios for E19 (feature
dossiers) are written from ADR-0038 + handoff v3 §8e — that epic is filed (#69–#76) and not
built, so those are intended behavior.

---

### Restart, reload, and persistence

#### CROSS-01 — Restore the selected project and task across a relaunch
- **Given:** project P with task T; T is selected in the task view; the app is quit normally.
- **When:** the user relaunches fartCode.
- **Then:** the task view for T under P is on screen after load — no placeholder, no "Add a
  project" hero. The restored ids come from the backend key `view-state:app:sidebar`
  (`collapsed`, `selectedProjectId`, `selectedTaskId`).
- **Covers:** ADR-0017 (view state / onboarding), E1-08 "layout restores after restart".
- **Automation:** backend command `set_view_state("view-state:app:sidebar", …)` to seed, reload
  the webview, assert `useSidebar.getState().selectedTaskId` (store is not exported on `window`,
  but `window.__tabsStore` is — a DOM assertion on the rendered task header is the honest driver).
- **Status:** implemented

#### CROSS-02 — A saved task id that no longer exists degrades to the project view
- **Given:** persisted sidebar view state names task T under project P; T was deleted (by
  another instance, or by a `delete_task` between sessions).
- **When:** the app boots.
- **Then:** P is selected, the board renders, `selectedTaskId` is null, and no error banner
  appears. (`sidebar.ts` only accepts a saved task id that is still in `listTasks(P)`.)
- **Covers:** E1-08 restore validation.
- **Automation:** backend `set_view_state` with a bogus task id + `delete_task`, reload, assert
  the board (not the task view) renders.
- **Status:** implemented

#### CROSS-03 — A saved project id that no longer exists falls back to the first project
- **Given:** persisted state names a deleted project; two other projects exist.
- **When:** the app boots.
- **Then:** the first project in `list_projects` order is selected. With zero projects, the
  "Add a project to get started — press ⌘⇧N" placeholder renders instead.
- **Covers:** `App.tsx` placeholder branch; `sidebar.load` default-selection comment.
- **Automation:** seed view state + delete the project via `delete_project`, reload, assert.
- **Status:** implemented

#### CROSS-04 — The collapsed flyout survives a relaunch
- **Given:** the project flyout is collapsed with ⌘B/⌘\.
- **When:** the app is quit and relaunched.
- **Then:** the flyout is still collapsed; the 56px rail is always visible.
- **Covers:** v1 README "⌘\ toggles it and the state persists"; `store/ui.ts` `fc:sidebarVisible`.
- **Automation:** RTL/browser: set `localStorage["fc:sidebarVisible"]="0"`, reload, assert the
  flyout element is absent.
- **Status:** implemented — but stored in `localStorage`, not the backend view-state KV like
  every other piece of layout (see gap CROSS-G1).

#### CROSS-05 — Persisted terminal tabs respawn and rewrite their ids
- **Given:** task T has two terminal tabs; the app is quit (plain PTYs die with the process).
- **When:** the user reopens T after relaunch.
- **Then:** two terminal tabs appear with fresh, live PTYs; the persisted
  `view-state:task:<T>:tabs` blob now holds the NEW ids (rewritten immediately on restore), so a
  second restart never carries a previous-process id.
- **Covers:** ADR-0021 terminal persistence; `store/tabs.ts` `ensureTabs`/`reconcile`.
- **Automation:** seed the tabs view-state with fake ids, open the task, assert
  `window.__tabsStore.getState().panesByTask[T].left.tabs` ids differ from the seeded ones.
- **Status:** implemented

#### CROSS-06 — An agent tab comes back as a plain shell wearing the agent's name
- **Given:** task T's only tab is the agent terminal (title "claude"), spawned by a board
  dispatch; the app is quit.
- **When:** the user reopens T after relaunch.
- **Then (intended):** the tab reattaches or relaunches the AGENT, or is clearly labelled as a
  dead session with a "resume the agent" affordance.
- **Then (actual):** `reconcile` respawns every non-live terminal tab through
  `terminalOpen(taskId, …)` — a plain `$SHELL` — while keeping the persisted title, so the tab
  reads "claude" and contains a bare shell. Nothing in the UI says the agent is gone.
- **Covers:** ADR-0033 one agent terminal per task; ADR-0021.
- **Automation:** seed a tab `{kind:"terminal", title:"claude", id:"dead"}`, open the task,
  assert `terminal_list_for_task` reports `kind: "shell"` for the new id while the tab title is
  still "claude".
- **Status:** partial (gap CROSS-G2)

#### CROSS-07 — tmux survivors resurface as extra tabs after a crash
- **Given:** project P has `tmux: true`; task T has two shell terminals; the app is
  force-killed (tmux server keeps the sessions).
- **When:** the user relaunches and opens T.
- **Then:** every surviving `{project}:{task}:terminal:{slot}` session is reattached — the
  persisted tabs reattach their slots and any uncovered survivor gets an additional tab, so the
  count of tabs matches the count of live sessions.
- **Covers:** ADR-0025 / ADR-0028; `terminal_surviving` + `TerminalManager::pick_slot`.
- **Automation:** integration test with a real tmux binary: spawn sessions by prefix, call
  `terminal_surviving`, then `terminal_open` N times and assert no new session names appear.
- **Status:** implemented (requires tmux installed and the project setting on)

#### CROSS-08 — A webview reload reattaches live terminals and replays scrollback
- **Given:** an agent is mid-run in task T (backend alive).
- **When:** the frontend reloads (⌘R in dev / HMR), not the process.
- **Then:** the same PTY ids are still live (`terminal_list_for_task`), the tabs reattach
  without respawning, and each xterm replays up to 64 KB of tail before new output — no
  duplicated shell, no lost prompt.
- **Covers:** `TerminalManager::tail` (TAIL_CAP 64 KB); `lib/terminals.ts` subscribe-then-tail order.
- **Automation:** browser smoke: note `panesByTask` ids, reload, assert identical ids and that
  `terminal_tail` returned non-null.
- **Status:** implemented

#### CROSS-09 — The split pane restores only when it still has tabs
- **Given:** task T had a right split with one terminal; the app is relaunched.
- **When:** T is reopened.
- **Then:** the right pane is restored with a respawned terminal and `activePane` honors the
  saved value; if the right pane's tabs all failed to respawn, the split silently collapses to a
  single pane and `activePane` resets to "left".
- **Covers:** `store/tabs.ts` `reconcile` + `activePane` derivation.
- **Automation:** seed a right pane whose respawn is forced to fail (e.g. delete the task's
  workspace path so `terminal_open` errors), reload, assert `panesByTask[T].right === null`.
- **Status:** implemented

#### CROSS-10 — Panel/drawer open-state does not survive a relaunch
- **Given:** the Changes sheet is open on a task and the ⌘J drawer is open.
- **When:** the app is relaunched and the same task is restored.
- **Then (actual):** the drawer is closed, Changes is closed, and the PM chat is open on the
  project view — the `ui.ts` defaults, not the user's last layout.
- **Covers:** `store/ui.ts` initial values (`changesOpen:false`, `drawerOpen:false`,
  `projectChatOpen:true`).
- **Automation:** RTL: assert the store's initial state; browser: open the drawer, reload, assert
  it is closed.
- **Status:** partial — deliberate for some flags, unspecified for others (gap CROSS-G3)

#### CROSS-11 — A parked (queue-mode) step does not survive a restart and re-parks rather than advancing
- **Given:** column "Review" is `kind: agent_step, on_enter: queue, on_settle: advance → Done`;
  card C sits in Review with a pending confirm; the app is killed before the user confirms.
- **When:** the app restarts and any settle trigger fires for C's linked task (an agent PTY exit
  or an ACP turn completion).
- **Then:** C does NOT advance to Done. A `step:queued` event fires for Review, the confirm
  overlay reappears, and the card stays in Review until the user presses ↵.
- **Covers:** ADR-0037 items 2–4; `step_engine.rs` "Restart contract" (`SettleDecision::Repark`).
- **Automation:** backend integration test — fresh `App::init`, seed an issue in a queue column
  with a linked task, call `settle_issues_for_task`, assert `StepQueued` emitted and the issue's
  `column_id` unchanged. (Covered today by `step_engine.rs` tests.)
- **Status:** implemented

#### CROSS-12 — A run-mode step still auto-advances once after a restart
- **Given:** the seeded "In Progress" column (`on_enter: run`, `on_settle: advance → In Review`)
  holds card C with a live agent; the app restarts (launch registry is empty).
- **When:** the agent session settles.
- **Then:** C moves to In Review exactly once; a second settle trigger for the same task no-ops
  (the heuristic leaves a tombstoned registry entry).
- **Covers:** E17-03 auto-flip parity; ADR-0037 item 4.
- **Automation:** backend: two `settle_issues_for_task` calls, assert one lane/column change and
  one `StepSettled`/enter.
- **Status:** implemented

#### CROSS-13 — Boot prunes orphaned view-state rows
- **Given:** `view-state:task:<T>:tabs` exists for a task deleted out-of-band (e.g. DB edited).
- **When:** the app boots.
- **Then:** the orphan row is gone from the KV and no phantom tabs appear anywhere.
- **Covers:** E1-08; `lib.rs::prune_view_state_on_boot` → `view_state::prune_orphans`.
- **Automation:** backend unit test around `prune_orphans` with a seeded orphan row.
- **Status:** implemented

#### CROSS-14 — A second launch focuses the existing window instead of opening a second one
- **Given:** fartCode is running (possibly minimized).
- **When:** the user launches the app again from the Dock/Finder.
- **Then:** the existing window is shown, un-minimized and focused; no second window and no
  second backend appear.
- **Covers:** E1-08 acceptance 3; `tauri_plugin_single_instance` in `lib.rs`.
- **Automation:** OS-level double-launch — needs a driver we lack (no window-count assertion in
  the test harness today).
- **Status:** implemented (unverifiable in CI)

#### CROSS-15 — Onboarding shows once and its completion survives a relaunch
- **Given:** a first-ever run (no `view-state:app:onboarding`).
- **When:** the user skips through welcome → add project → agents → GitHub, then relaunches.
- **Then:** the onboarding overlay appears on the first run and never again; `{done:true}` is
  written to `view-state:app:onboarding`. Every step is skippable, so the app is reachable
  fully offline with no agent installed.
- **Covers:** ADR-0017; `Onboarding.tsx`.
- **Automation:** clear the key, reload, assert the overlay; finish, reload, assert absent.
- **Status:** implemented

---

### Failure and degraded states

#### CROSS-16 — Dropping onto a run-mode step with no agent CLI installed moves the card and then fails
- **Given:** no provider binary on PATH (`claude`, `omp`, … absent); column "Implement" is a
  run-mode agent step.
- **When:** the user drags card C into Implement.
- **Then (actual):** the backend enter succeeds — the card MOVES and a worktree + task are
  provisioned — then `terminal_open_agent` rejects with `agent not installed: <id>` and the
  board renders that string in its error line. The card is now in a step column with no session
  and no retry affordance beyond re-dragging.
- **Then (intended):** the move should be refused (or reversed) with a "no agent installed —
  install one in Settings" affordance, before a worktree is created.
- **Covers:** E3-02 host dependencies; ADR-0037 item 2 (worktree provisions on first step entry).
- **Automation:** backend `issue_enter_column` with an empty PATH → assert `EnterOutcome.step ==
  "launched"`; frontend `terminal_open_agent` → assert the error string. Both drivable.
- **Status:** partial (gap CROSS-G4)

#### CROSS-17 — Expired provider auth is discovered by the agent, not by the app
- **Given:** the `claude` CLI is installed but logged out / the OAuth token expired.
- **When:** the user dispatches a card into a run-mode step.
- **Then (actual):** a worktree + task are created, the agent terminal opens, the prompt is
  bracket-pasted, and the CLI's own "please log in" output is the only signal. The card sits in
  the step column; when the CLI exits, the settle fires and the card ADVANCES as though the step
  succeeded (see CROSS-26).
- **Then (intended):** `provider_auth_status` is consulted before spending — a card entering a
  step with an unauthenticated provider surfaces the sign-in affordance instead of dispatching.
- **Covers:** E3-07 provider accounts; `commands/provider_accounts.rs::provider_auth_status`
  (exists, and is called only from the settings/accounts surface).
- **Automation:** backend `provider_auth_status("claude")` with a logged-out CLI → assert
  `authenticated:false`; then assert `issue_enter_column` still returns `"launched"`.
- **Status:** not-built (gap CROSS-G5)

#### CROSS-18 — A worktree deleted from disk offers re-provisioning
- **Given:** task T's workspace row exists but its directory was `rm -rf`'d outside the app.
- **When:** the user opens T and the Changes sheet.
- **Then:** the sheet shows "This task's workspace isn't on disk yet." with a **Provision
  workspace** button; pressing it runs `provision_task` and refetches, and a failure replaces the
  copy with the error rather than clearing the panel.
- **Covers:** ADR-0016 worktree directory validation; `ChangesSidebar.tsx` (the
  `workspace has no local path` branch).
- **Automation:** delete the worktree dir, call `git_status(workspaceId)` → assert the
  `workspace has no local path` error; RTL on the panel branch.
- **Status:** implemented

#### CROSS-19 — A missing worktree at boot silently skips agent rehydration
- **Given:** a PTY conversation with a stored `session_id` whose task worktree no longer exists.
- **When:** the app boots and `rehydrate_all` runs on its background thread.
- **Then (actual):** the conversation is counted as `skipped` and a `tracing::warn!` is emitted;
  nothing reaches the UI. The task looks identical to one that resumed.
- **Then (intended):** the task shows a "workspace missing — re-provision" state rather than an
  empty pane.
- **Covers:** E2-07 boot rehydration ("the non-tmux degradation"); `pty/launcher.rs`
  `rehydrate_all`.
- **Automation:** backend: seed a conversation + task + workspace row pointing at a nonexistent
  path, call `rehydrate_all`, assert `RehydrateSummary { skipped: 1, .. }`.
- **Status:** partial (gap CROSS-G6)

#### CROSS-20 — A task with no workspace at all reads as "not provisioned yet"
- **Given:** a task created without provisioning (BYOI / project-root variant).
- **When:** the Changes sheet is opened.
- **Then:** "This task has no workspace yet — changes appear once it's provisioned." No error
  styling, no retry button (there is nothing to retry).
- **Covers:** `ChangesSidebar.tsx` `!workspaceId` branch.
- **Automation:** RTL component test with `workspaceId = null`.
- **Status:** implemented

#### CROSS-21 — Network down: PR data renders from cache and the scheduler backs off
- **Given:** a task with an open PR previously synced; the machine goes offline.
- **When:** the user opens the PR tab and waits through several scheduler cycles.
- **Then:** the cached PR row, checks, files and comments render immediately from the
  `pull_requests` cache; each failed sync doubles that workspace's interval
  (`base * 2^failures`, capped); no error toast appears and the panel never blanks.
- **Covers:** E4-09 PR sync; `fartcode-git/src/pr_sync.rs` (`backoff_interval`, "cached rows
  render offline immediately").
- **Automation:** backend: point the client at an unreachable API base, call `sync_workspace`
  repeatedly, assert `failure_count` climbs and `pr_section_get` still returns the cached row.
- **Status:** implemented — with no offline indicator anywhere in the UI (gap CROSS-G7)

#### CROSS-22 — A GitHub rate limit ends the sync cycle invisibly
- **Given:** the account's REST quota is exhausted (403/429 with a reset header).
- **When:** the scheduler reaches a workspace.
- **Then (actual):** `Error::GithubRateLimited` breaks the whole cycle with a `tracing::warn!`;
  the PR panel keeps showing stale data with no "last synced / rate limited until HH:MM" line.
- **Then (intended):** the PR panel names the staleness and the reset time.
- **Covers:** `pr_sync.rs` scheduler `break` on `GithubRateLimited`.
- **Automation:** backend: stub a 403 + `x-ratelimit-reset`, assert the loop breaks and
  `record_failure` fired once.
- **Status:** partial (gap CROSS-G7)

#### CROSS-23 — No GitHub token: the PR tab asks, the board's issue import does not
- **Given:** no token in the keyring and no `gh` CLI login.
- **When:** the user opens the PR tab, then returns to the board.
- **Then:** the PR tab renders the token gate ("Connect GitHub to see pull requests. The token
  lives in your OS keyring") with **Import from gh** and **Paste token**. The board's autorun
  GitHub issue import fails and is swallowed to `console.warn("github issue sync failed:")` —
  the board shows nothing at all.
- **Covers:** E4-07 token source; `PullRequestPanel.tsx` token gate; `BoardView.tsx`
  `syncGithubIssues` catch.
- **Automation:** `github_token_clear()` then `pr_section_get` → assert `tokenPresent:false`;
  RTL on the gate. The board's silence is asserted by the absence of any error node.
- **Status:** implemented (PR tab) / partial (board import silence — gap CROSS-G8)

#### CROSS-24 — An unavailable keyring surfaces only where a token is read
- **Given:** the OS keyring is locked or denies access (`Error::CredentialStore`).
- **When:** the user opens the PR tab.
- **Then:** the status probe rejects; `PullRequestPanel` catches and leaves the gate/state as-is
  — the user sees "connect GitHub" rather than "your keyring is locked".
- **Covers:** `fartcode-core/src/github/token.rs`; `PullRequestPanel.tsx:435`
  (`.catch(() => {})`).
- **Automation:** hard to fake a locked keyring in CI — needs a driver we lack; the swallowed
  catch is assertable by unit-testing the component with a rejecting `githubTokenStatus`.
- **Status:** partial (gap CROSS-G9)

#### CROSS-25 — An unopenable database kills the launch with no message
- **Given:** the app-data DB file is corrupt, or `FARTCODE_DB_FILE` points somewhere unwritable.
- **When:** the user launches fartCode.
- **Then (actual):** `App::init` returns `Err` from the Tauri `setup` hook; the process fails to
  start and the user sees nothing (or a raw crash dialog) — no window, no recovery path.
- **Then (intended):** a window opens with a readable failure card naming the DB path and
  offering to reveal it in Finder.
- **Covers:** `lib.rs::run` setup hook; ADR-0001 migrations (a hash-frozen migration mismatch
  reaches the same place).
- **Automation:** launch with `FARTCODE_DB_FILE=/dev/null/x` and assert a non-zero exit — the
  absence of a window is the assertion.
- **Status:** not-built (gap CROSS-G10)

#### CROSS-26 — An agent that exits non-zero settles the step exactly like a successful one
- **Given:** card C in the seeded "In Progress" run-mode step (`on_settle: advance → In Review`);
  its agent CLI exits immediately with code 1 (bad flag, expired auth, OOM).
- **When:** the PTY pump observes the exit.
- **Then (actual):** the terminal prints `[process exited with code 1] — close this tab`, and
  `flip_for_exited_agent` → `settle_issues_for_task` advances C to In Review. No exit code is
  consulted anywhere in `step_engine.rs`.
- **Then (intended):** a failed step holds the card and marks it needs-you rather than advancing
  it into review.
- **Covers:** ADR-0037 item 4 (on_settle); `terminals.rs` pump → `dispatch::flip_for_exited_agent`.
- **Automation:** backend: open an agent terminal whose program is `/bin/sh -c 'exit 1'`, assert
  the linked issue's column advanced.
- **Status:** implemented-as-specified, but the spec has no failure branch (gap CROSS-G11)

#### CROSS-27 — A failed setup script blocks the agent and says so
- **Given:** project P has a setup lifecycle script that exits non-zero; task T is opened.
- **When:** the empty pane renders.
- **Then:** while setup runs, the pane shows "Waiting on setup before starting…"; after a
  non-zero exit it shows `setup failed · exit <code>` in the failed style, over the three
  key-labelled rows (Resume the agent ⌘T / Split with a shell ⌘D / New terminal ⌘⇧T). The view
  never auto-spawns a shell.
- **Covers:** 7b setup gate; `TaskView.tsx` `PaneEmpty`.
- **Automation:** RTL with a seeded `useScripts` state (`setup: {running:false, exitCode:2}`).
- **Status:** implemented

---

### Concurrency

#### CROSS-28 — Two agents in two tasks settle only their own cards
- **Given:** cards C1 and C2 in the same project, each with its own linked task and live agent
  session, both in run-mode step columns.
- **When:** C1's agent exits while C2's keeps running.
- **Then:** only C1's card settles/advances; C2 stays put and its agent is untouched (the board
  never kills). The settle is keyed by task id AND session identity (`pty:<terminal>` /
  `acp:<conversation>`).
- **Covers:** ADR-0037 item 11; `step_engine.rs` launch registry / session binding.
- **Automation:** backend: two issues, two tasks, call `settle_issues_for_task(t1, Some("pty:x"))`
  and assert only issue 1 moved.
- **Status:** implemented

#### CROSS-29 — A stale session's settle cannot move a card a second time
- **Given:** card C ran a step in "Implement", advanced to "Review" (queue-mode), and the SAME
  first session fires another settle trigger (long-lived PTY, second turn-complete).
- **When:** the stale trigger arrives.
- **Then:** C stays parked in Review awaiting the confirm — it never lands in Done. The
  consumed-session set and the park guard both refuse.
- **Covers:** the verifier scenario permanently pinned as
  `stale_settle_does_not_bypass_queue_confirm_gate`.
- **Automation:** backend test (exists in `step_engine.rs`).
- **Status:** implemented

#### CROSS-30 — A settle racing a manual drag loses to the drag
- **Given:** card C is in a run-mode step with a live agent; the user drags C onto a shelf column
  at the moment the agent exits.
- **When:** both the drag and the exit land.
- **Then:** C ends on the shelf and stays there — the settle reads C's CURRENT column, sees a
  non-`agent_step` kind, and does nothing (E17-03 parity). The agent keeps running.
- **Covers:** `settle_issues_for_task` (`column.kind != AgentStep → continue`).
- **Automation:** backend: `issue_enter_column` to a shelf, then `settle_issues_for_task`, assert
  the column is unchanged.
- **Status:** implemented

#### CROSS-31 — Two confirms of one parked step launch exactly one agent
- **Given:** card C parked in a queue-mode column; the confirm overlay is visible; the user
  presses ↵ twice (or two surfaces confirm at once).
- **When:** both `step_confirm` calls run.
- **Then:** exactly one `StepLaunch` is emitted and one session opens; the loser gets the typed
  `no parked step` error, which the board renders in its error line.
- **Covers:** ADR-0037 item 2; `confirm_step` atomic park-take.
- **Automation:** backend threaded test (exists: `concurrent_confirms_launch_exactly_once`).
- **Status:** implemented

#### CROSS-32 — One launch directive arriving twice opens one session
- **Given:** a run-mode drop: the frontend receives the launch both as `EnterOutcome.launch` and
  as the `step:launch` event.
- **When:** both handlers run within 4 s.
- **Then:** one agent terminal opens and one prompt is pasted (`claimLaunch` dedupes on
  `issueId:columnId` for `LAUNCH_DEDUPE_MS = 4000`).
- **Then (edge, falsifiable):** a settle-chained launch back into the SAME column within 4 s is
  swallowed by the same dedupe and never opens.
- **Covers:** `BoardView.tsx` `claimLaunch`.
- **Automation:** RTL/browser: dispatch the event immediately after the command resolves, count
  `terminal_open_agent` invocations. The 4 s edge needs a fake clock.
- **Status:** implemented, with a time-based edge (gap CROSS-G12)

#### CROSS-33 — Two projects can run agents at the same time
- **Given:** project A has a live agent in task T1; the user switches to project B and dispatches
  a card there.
- **When:** both are running.
- **Then:** both agents run; the rail shows A and B each with a state dot (running beats
  needs-you); switching projects kills nothing and re-selecting A shows T1 still live.
- **Covers:** `Nav.tsx` `agentState`; ADR-0037 item 11.
- **Automation:** two projects seeded, two `terminal_open_agent` calls, assert
  `terminal_list_for_task` reports running for both.
- **Status:** implemented

#### CROSS-34 — Deleting a project clears its parked steps
- **Given:** project P has card C parked in a queue-mode column, overlay showing; the user
  deletes P.
- **When:** the deletion completes.
- **Then:** a `step:queue_cleared` fires for C, the overlay disappears, and no engine state for
  P's issues survives (parks, launch registry, consumed sets).
- **Covers:** `step_engine::on_project_deleted` (fix round finding 4).
- **Automation:** backend: park, `delete_project`, assert `StepQueueCleared` and
  `peek_park(C).is_none()`.
- **Status:** implemented

#### CROSS-35 — There is no second window
- **Given:** the app is running.
- **When:** the user looks for New Window (⌘N) or drags a task out.
- **Then (intended, if ever specified):** a second window with its own project/task selection.
- **Then (actual):** no command, no menu item, no API — the single-instance plugin actively
  refuses a second instance, and all layout state is global to one webview.
- **Covers:** nothing — the design never settled multi-window.
- **Automation:** n/a.
- **Status:** unreachable-entirely (gap CROSS-G13)

---

### Spend and consent

The enumeration below is the point of this cluster: **every path that can start an agent.**
Three of them ask; six do not.

#### CROSS-36 — Dropping a card onto a run-mode agent step spends with no confirm
- **Given:** column "Implement" is `kind: agent_step, on_enter: run`; card C is unblocked and has
  no live agent.
- **When:** the user drags C into Implement.
- **Then:** the card moves, a worktree + task are provisioned if this is C's first step, the
  agent terminal opens and the prompt packet is bracket-pasted — with no dialog. The only prior
  warning is the column header subline (`claude · sonnet — run → In Review`) rendered brighter
  (`#9a9aa1`) than a queue column's.
- **Covers:** handoff v3 README §8a "Confirm-free spend is brighter"; DESIGN.md:324;
  `columnConfig.ts::columnSublineTone`.
- **Automation:** backend `issue_enter_column` → assert `step:"launched"`; RTL for the subline
  tone class.
- **Status:** implemented (by design)

#### CROSS-37 — The same spend is one keystroke away, with the same absence of a confirm
- **Given:** card C is focused on the board (j/k), the column to its right is a run-mode step.
- **When:** the user presses ⇧L.
- **Then:** identical to CROSS-36 — `requestMove` → `enter` → launch. No confirm, no undo.
- **Covers:** frame 4b keyboard; `BoardView.tsx` keydown handler.
- **Automation:** browser smoke: focus a card, send ⇧L, assert a `step:launch` event.
- **Status:** implemented (by design)

#### CROSS-38 — A queue-mode column asks before spending
- **Given:** column "Review" is `on_enter: queue`.
- **When:** a card enters it (drag, ⇧L, or a settle-chained advance).
- **Then:** the card MOVES, the step parks, `step:queued` fires and the templated confirm overlay
  appears; ↵ fires `step_confirm` (launch), esc dismisses the overlay only — the backend park
  survives, so re-dragging the card re-asks.
- **Covers:** ADR-0037 items 2–3; handoff v3 §8c.
- **Automation:** backend assert `StepQueued` + `EnterOutcome.step == "queued"`; RTL for the
  overlay's ↵/esc handling.
- **Status:** implemented

#### CROSS-39 — A blocked card entering a step asks first
- **Given:** card C has an unfinished blocker (`blocked` derived from `countsAsDone`).
- **When:** the user drags C into any agent-step column.
- **Then:** the card does NOT move; a confirm names the target column and the blocker; esc keeps
  the card where it was; ↵ proceeds through the normal enter path.
- **Covers:** ADR-0032 "confirm, never a hard stop"; `requestMove` `kind:"blocked"`.
- **Automation:** RTL with a blocked issue fixture; assert no `issue_enter_column` call on esc.
- **Status:** implemented

#### CROSS-40 — Moving a live-agent card into a done column asks, then moves, and never kills
- **Given:** card C has a live agent session; column "Done" has `countsAsDone: true`.
- **When:** the user drags C into Done.
- **Then:** a confirm appears; on ↵ the card moves and **the agent keeps running** (the board
  never kills); on esc nothing moves.
- **Covers:** ADR-0037 item 11; `requestMove` `kind:"live-agent"`.
- **Automation:** RTL + backend assert the agent terminal is still `running` after the move.
- **Status:** implemented

#### CROSS-41 — A settle-chained advance into a run-mode step spends with zero user gestures
- **Given:** "Implement" is `on_settle: advance → Refactor`, and "Refactor" is
  `on_enter: run`.
- **When:** the Implement agent finishes while the user is in another project entirely.
- **Then:** the engine enters Refactor, launches a second agent session in the same task/worktree
  and emits `step:launch`; the board (if mounted for that project) opens the agent terminal and
  pastes the prompt. Nothing was clicked. A chain of N run columns spends N times.
- **Covers:** ADR-0037 item 4 ("chains are legal"); `settle_issues_for_task` → `enter_column`.
- **Automation:** backend: two chained run columns, one `settle_issues_for_task`, assert two
  `StepLaunch` events.
- **Status:** implemented (by design) — no per-project budget or chain-depth cap exists
  (gap CROSS-G14)

#### CROSS-42 — Opening a project starts an ACP agent session unasked
- **Given:** the PM chat panel is open (its `ui.ts` default is `true`).
- **When:** the user selects any project.
- **Then:** the first ACP-capable provider is resolved, a project-scoped conversation is
  created/fetched and `acp_start` spawns the adapter — before the user types anything. The panel
  reads "Starting the project agent…". No prompt is sent, so no tokens are billed until the user
  writes, but a provider process is running against their account.
- **Covers:** E17-04; `ProjectChatPanel.tsx`.
- **Automation:** browser: select a project, assert `acp:transcript`/adapter process exists
  without any user input.
- **Status:** implemented (by design) — see gap CROSS-G15

#### CROSS-43 — Boot resumes previously-spawned agent conversations unasked
- **Given:** a PTY-type conversation with a stored `session_id` under a task with a live worktree.
- **When:** the app boots.
- **Then:** `rehydrate_all` relaunches that provider CLI with resume flags on a background
  thread, with `auto_approve` defaulting to false; the initial prompt is NOT re-sent. The user is
  never asked whether to resume, and the summary only reaches the log.
- **Covers:** E2-07 boot rehydration; `pty/launcher.rs::rehydrate` + `lib.rs` boot thread.
- **Automation:** backend: seed a conversation with a session id, call `rehydrate_all`, assert
  `resumed == 1`.
- **Status:** implemented (by design) — no user-visible "resumed N sessions" signal
  (gap CROSS-G16)

#### CROSS-44 — Entering a project fires background network work
- **Given:** project P is selected for the first time in ≥30 s (git pull) / ≥60 s (issue import).
- **When:** the user clicks P in the rail.
- **Then:** `project_git_pull(P)` runs `--ff-only` in the background, and the board imports every
  open GitHub issue not already on it. Both are cooldown-gated in memory (a relaunch re-runs
  both immediately). Failures are `console.warn` only.
- **Covers:** `sidebar.ts::autoPullProject`; `BoardView.tsx::syncGithubIssues`.
- **Automation:** backend spies on `project_git_pull` / `issue_import_github` counts across two
  rapid project selections.
- **Status:** implemented — silent on failure and on success (gap CROSS-G8)

#### CROSS-45 — Nothing anywhere shows what a session cost
- **Given:** several dispatches have run in a project.
- **When:** the user looks for spend — board, card detail, task header, settings.
- **Then (intended, per ADR-0038 item 7):** local-only usage signals (context tokens saved, etc.)
  on a settings → project → Memory dashboard.
- **Then (actual):** no token count, no cost, no session ledger exists anywhere; `fartcode-telemetry`
  is a placeholder crate with one `assert_eq!(1+1, 2)` test.
- **Covers:** ADR-0038 item 7; handoff v3 §8g.
- **Automation:** grep-level assertion only — no surface to drive.
- **Status:** not-built (gap CROSS-G17)

#### CROSS-46 — Auto-approve exists in the backend and has no switch
- **Given:** a user who wants agent tool-calls auto-approved (or explicitly NOT).
- **When:** they look in App settings / project settings / the conversation menu.
- **Then (actual):** nothing. `auto_approve` is a conversation config field and a rehydrator
  parameter hardwired to `false` at boot (`App::init`, `// auto-approve defaults off on boot`);
  no frontend file references it.
- **Covers:** ADR-0013 auto-approve plumbing.
- **Automation:** `grep -r autoApprove app-frontend/src` returns nothing — the assertion is the
  absence of any caller.
- **Status:** unreachable-by-mouse (backend exists, no caller) (gap CROSS-G18)

#### CROSS-47 — ACP tool permission prompts are the one in-flight consent that works
- **Given:** an ACP conversation (PM chat or task chat) where the agent requests a tool
  permission.
- **When:** the request arrives.
- **Then:** a composer-docked "Allow <tool>?" card appears with a queue counter ("1 of N"); the
  decision routes through `acp_resolve_permission`; a store reload re-syncs surfaced prompts from
  the snapshot so a re-render never loses a pending decision.
- **Covers:** E2-11-5; `ConversationView.tsx` + `store/conversations.ts` `permissions`.
- **Automation:** emit `acp:permission_request` from a stub adapter, assert the card and the
  resolve call.
- **Status:** implemented (ACP paths only — a PTY agent's own TUI prompts are the user's problem)

---

### Notifications and telemetry

#### CROSS-48 — An agent that needs you while the app is in the background says nothing
- **Given:** task T's agent hits a permission prompt (or settles into a needs-you column) while
  fartCode is behind another app.
- **When:** the state changes.
- **Then (intended):** an OS notification ("fartCode · <task> needs you") that focuses the task
  on click, with a per-project or global mute.
- **Then (actual):** nothing — the only in-app signal is the rail/flyout dot. `tauri-plugin-notification`
  is not a dependency and the string "notification" appears nowhere in the frontend or app crate.
- **Covers:** nothing — never specified.
- **Automation:** n/a until a notification API exists.
- **Status:** not-built (gap CROSS-G19)

#### CROSS-49 — There is no telemetry, and therefore no opt-out
- **Given:** a privacy-conscious user opening Settings.
- **When:** they look for "usage data" / "analytics" / "share diagnostics".
- **Then (actual):** no such row, because nothing is collected or transmitted:
  `fartcode-telemetry` is an empty placeholder and no HTTP client is wired to any first-party
  endpoint. The only network callers are the GitHub client and git itself.
- **Then (intended, per ADR-0038 item 7):** if the memory metrics ship, they are computed
  **locally** and the dashboard says so ("computed locally, never leaves this machine") — the
  opt-out question only becomes real if that ever changes.
- **Covers:** ADR-0038 item 7; handoff v3 §8g subline.
- **Automation:** assert the absence of any outbound host other than `api.github.com` in the
  crate graph.
- **Status:** not-built (correctly — record it so it stays a deliberate choice) (gap CROSS-G20)

---

### E19 feature-dossier consent (ADR-0038 item 3 + handoff v3 §8e) — not built

#### CROSS-50 — First agent-step entry in a project asks before writing to the repo
- **Given:** project P has never dispatched a card; card C is dragged onto its first agent-step
  column.
- **When:** the entry fires, at the same moment the worktree provisions.
- **Then:** an overlay card appears BEFORE any queue confirm: "This feature will keep a dossier —
  write the convention files to your repo?", listing `docs/features/<slug>.md`,
  `.claude/skills/feature-log/`, `AGENTS.md · one pointer line`, with the meta line
  "provenance-tagged · commits ride the feature branch"; footer `esc run without memory` /
  `↵ write to repo`.
- **Covers:** ADR-0038 item 3; handoff v3 §8e.
- **Automation:** backend needs a `dossier_consent` project setting + a gate in
  `enter_column`/`provision_issue_task`; neither exists.
- **Status:** not-built (gap CROSS-G21)

#### CROSS-51 — Declining the dossier still dispatches
- **Given:** the consent card from CROSS-50 is showing.
- **When:** the user presses esc.
- **Then:** the step launches normally with no dossier and no skill scaffold; nothing is written
  to the repo; the card is never shown again for P.
- **Covers:** ADR-0038 item 3 ("declining runs the step without memory"); v3 §8e.
- **Automation:** assert the project setting is written `false` and `StepLaunch` still fires.
- **Status:** not-built

#### CROSS-52 — Consent ordering when a queue confirm is also due
- **Given:** the project's first agent-step column is `on_enter: queue`.
- **When:** a card enters it for the first time.
- **Then:** the consent card renders first; only after it resolves does the queue confirm appear
  — two overlays never stack, and esc on the consent does not also dismiss the queue confirm.
- **Covers:** v3 §8e ("BEFORE the queue confirm when both would show").
- **Automation:** RTL over the pending-overlay state machine (`BoardView.PendingConfirm` would
  need a fourth kind).
- **Status:** not-built

#### CROSS-53 — The decision is reversible from project settings, in both directions
- **Given:** project P declined (or accepted) dossiers at first dispatch.
- **When:** the user opens settings → project → Memory.
- **Then:** a shared-style `feature dossiers · on|off` row reflects the decision and flips it;
  turning it on later seeds the convention files at the next step entry; turning it off stops
  further writes without deleting what exists.
- **Covers:** ADR-0038 item 3 ("project settings carries the same switch in both directions");
  v3 §8d/§8g placement.
- **Automation:** backend project-settings round-trip once the key exists.
- **Status:** not-built

#### CROSS-54 — A deleted unmerged branch takes its dossier with it
- **Given:** dossiers are on; card C's feature branch has an unmerged `docs/features/c.md`.
- **When:** the task is deleted with `delete_branch`.
- **Then:** the dossier is gone with the branch — same risk profile as the code — and the card's
  `dossier_path` link in card detail renders nothing rather than a broken link; the issue row
  itself survives.
- **Covers:** ADR-0038 item 5; ADR-0023 task deletion.
- **Automation:** backend: create the file on the branch, delete the task with branch deletion,
  assert the file is unreachable and `issue.dossier_path` resolution is null-safe.
- **Status:** not-built

---

## Gap register

Every finding from all eight sections, in one table, sorted high → medium → low. The 187 raw rows
deduplicate to 153: where two or more authors reached the same defect from different surfaces the
rows are merged, the **Source** column names every scenario and section-local finding id that
contributed, and the merged row keeps the **highest** severity any author assigned. No finding was
dropped and no severity was lowered.

**Type** classifies the shape of the UI-flow failure:

| Type | Meaning |
|---|---|
| `no-affordance` | The capability exists (or is specified) but nothing in the UI offers it. |
| `dead-end` | A flow reaches a state with no way forward, no way back, or no feedback that it ended. |
| `unreachable` | Code — a command, a branch, a store action, a DTO field — that no user gesture can reach or produce. |
| `missing-confirm` | A destructive, irreversible or spending action fires with no confirmation, no itemisation, and no undo. |
| `unspecified` | The design never settled the behaviour, or the code contradicts the frame that did. |
| `unrepresentable-state` | A real backend state the UI cannot show — or shows wrongly. |

### High

| ID | Gap | Type | Severity | Source | Evidence | Suggested resolution |
|---|---|---|---|---|---|---|
| GAP-01 | Installing an agent CLI runs a `curl … \| bash` one-liner with **no confirmation** — a remote-code-execution action behind a single click; and the 7d `installing · 62%` progress state has no backend feed, so the row shows a bare `installing` | missing-confirm | high | FIRST-27 | `fartcode-core/src/dependencies/mod.rs:164-171` (security note), `AgentsList.tsx:41-49`, `fartcode-app/src/commands/dependencies.rs:5-10` | Add a confirm sheet naming the exact command and manager before `install`/`update`; land the PTY-backed runner (E2-06 seam) so the row can show real progress |
| GAP-02 | Deleting a project performs **no process teardown**: no ACP stop, no terminal close, no tmux sweep, no watch unregister — the SQL cascade emits no per-task `TaskDeleted`, so orphaned PTYs and tmux sessions keep running against a pool directory that was just `rm -rf`'d, and the confirm never mentions a running agent | missing-confirm | high | FIRST-57 · LIFE-G5 · LIFE-41 | `fartcode-app/src/commands/projects.rs:26-34` vs `commands/tasks.rs:302,312`; `terminals.rs:591` (task-scoped only); `watchers.rs:51`; `indexer.rs:57`; `Modals.tsx:780-789` | Mirror `delete_task`'s teardown in `delete_project` (`acp.stop_task` + `terminals.close_task` per task) or emit `TaskDeleted` per cascaded task so existing subscribers do their jobs; itemise live agents in the confirm the way `DeleteTaskConfirm` does |
| GAP-03 | Two projects whose directories share a **name** share one worktree pool; deleting either `remove_dir_all`s the other project's on-disk worktrees **and their uncommitted work**. The surviving project's task rows then point at missing paths | missing-confirm | high | FIRST-58 · LIFE-G7 · LIFE-43 | `fartcode-core/src/projects/mod.rs:320-334`, `projects/provider.rs:169-177` (`safe_path_segment(&project.name)`) | Key the pool segment on `project.id` (or `name-<short id>`) with a one-time migration; until then detect the collision, block the pool teardown, and name it in the delete confirm |
| GAP-04 | The project-settings **"Worktree directory"** field is stored, validated and displayed but consumed by nothing — worktrees always use the app-level `localProject.defaultWorktreeDirectory`, and the row renders that path as its placeholder, which makes it look authoritative | dead-end | high | FIRST-38 | `projects/provider.rs:169-177`; the only readers of `ProjectSettings.worktree_directory` are `service.rs:202` and `commands/settings.rs:45` | Have `worktree_pool_path` prefer the project setting and fall back to the app default, or remove the row |
| GAP-05 | Clone-from-URL is fully implemented in core and completely unreachable — no Tauri command, no UI, no way to add a project by cloning | unreachable | high | FIRST-16 | `fartcode-core/src/projects/mod.rs:376`; absent from `lib.rs:134-136` | Register a `create_project_clone` command and add a clone mode (URL vs path) to `CreateProjectDialog` |
| GAP-06 | **No app-level settings surface at all**: no `get_app_setting` / `set_app_setting` / `settings_reset` commands are registered, so `localProject`, `terminal`, `notifications` and `browserPreview` are uneditable and `SettingsStore::reset` is dead. The only app setting the app can write is `defaultAgent` | no-affordance | high | FIRST-39 | `fartcode-app/src/lib.rs:219-234`; `SettingsModal.tsx:208-214` | Add generic app-setting read/write commands and an App-settings group in the ⌘, App pane |
| GAP-07 | `BoardView` is the **only** subscriber to `step:*`, and it unmounts the moment a dispatch navigates into the task view — so a settle-chained `step:launch` opens nothing. Full-auto chains silently stall with the card in the new column and no session | dead-end | high | BOARD-22 | `components/board/BoardView.tsx:232-291` (listener inside the mount effect); grep shows no other `step:launch` consumer | Move the launch-directive listener into an app-level wire (`App.tsx`, alongside `wireChangesEvents`) or a `store/steps.ts`, so launches are honoured regardless of which view is mounted |
| GAP-08 | The step-done and queued flags live in component state cleared on every mount and project switch; in the normal flow (dispatch → task view → back) the "step finished, drag it on" state is **never seen** | unrepresentable-state | high | BOARD-21 · BOARD-43 | `BoardView.tsx:185, 210-216` (`setSteps({})`) | Hoist step flags to a per-project zustand store that survives unmount, or derive step-done from a backend-readable signal (last settle per issue+column) |
| GAP-09 | A blocker moved into a **non-seeded** `counts_as_done` column never unblocks its dependents — `BLOCKED_SQL` resolves the blocker's column via `seed_lane = lane`, and non-seeded entries leave `lane` stale. ADR-0037's "multiple terminal columns are legal" is false today | dead-end | high | BOARD-25 | `fartcode-core/src/issues/mod.rs:250-256`, `:712-728`; ADR-0037 item 6 | Part of the E18-07 authority flip already on #66: switch the join to `c.id = b.column_id`, with the lane fallback only for mirrorless rows |
| GAP-10 | **No UI anywhere edits columns**: `columnCreate/Update/Delete/Reorder` have zero callers and settings has no Columns section, so ADR-0037's whole premise — columns as configurable steps — is reachable only from a devtools console | no-affordance | high | BOARD-46 | `app-frontend/src/lib/tauri.ts:1297-1357`; `components/ProjectSettings.tsx` (no `column` match) | Build handoff §8d (#67), reusing `columnConfigSummary` as the single formatter |
| GAP-11 | **Model, effort and tool selection never reaches a launched session.** `terminal_open_agent` takes only a provider, so a column's `step_model`/`step_effort`/`step_tools` are advertised in the header subline (seeded Quick claims `claude · haiku`) and ignored at launch; the ⌘N composer's `agent` row is likewise a static string with no picker and no model half | no-affordance | high | BOARD-19 · TASK-05 | `fartcode-app/src/commands/terminals.rs:138-145`; `BoardView.tsx:392-412`; `components/Modals.tsx:257-266`; design_handoff_v2 §5h | Extend `terminal_open_agent` (or add `terminal_open_step`) with model/effort/tool arguments, thread `StepLaunchInfo`'s fields and the composer's choice through `create_task` → `launch_default_agent`; until then omit unhonoured fields from the subline |
| GAP-12 | Dropping a **live-agent** card onto another `agent_step` gets no confirm, and `terminal_open_agent` reattaches to the running agent — so the new step's prompt is bracket-pasted into a mid-turn session under the wrong provider | missing-confirm | high | BOARD-28 | `BoardView.tsx:452-457` (gate only covers `countsAsDone`); `fartcode-app/src/terminals.rs:519-527` | Extend the live-agent confirm to any `agent_step` target, and make a step launch open a *new* agent entry rather than reattaching when the target column pins a different provider |
| GAP-13 | Approval-card state is component-local and unpersisted, so any remount (panel toggle, card-detail open, tab switch) restores an approved card to approvable — a second click writes **duplicate issues**, with no dedupe in `apply_proposal` and no undo for either write | unrepresentable-state | high | PM-31 | `ProposalCard.tsx:36-41`; `issue_proposal.rs:89-105` | Persist an `appliedProposals` set keyed by a hash of the block payload (view-state or a `proposal_applications` table); render applied blocks as a read-only receipt linking to the created cards |
| GAP-14 | The active-turn → settled-turn transition swaps `TurnBlock` for `memo(TurnBlock)`, remounting the subtree and discarding the card's applied/edited/dropped state mid-turn — inviting the duplicate write above | unrepresentable-state | high | PM-32 | `TranscriptItems.tsx:462-502`; `ConversationView.tsx:271-285` | Render both tiers through the same component type (memoise at the call site or always use `SettledTurn`), or lift card state into the conversations store keyed by turn+block index |
| GAP-15 | Proposal-card keys leak to `BoardView`'s window keydown: `↵` approves **and** opens the board's focused card detail (swapping the sheet away and destroying the card), `j`/`k` move two cursors, `a` opens the board's add-issue composer while focus is in the chat | unspecified | high | PM-33 | `ProposalCard.tsx:147-164` (no `stopPropagation`); `BoardView.tsx:537-538, 555, 628` | Call `e.stopPropagation()` in the card handlers, and/or have `BoardView` bail when `e.target.closest('.proposal-card, .project-chat')` |
| GAP-16 | `issue_update` takes no project id and does no ownership check, so a ticket-edit block naming **another project's** issue applies successfully while the card claims "apply will fail" | unspecified | high | PM-42 | `TicketEditCard.tsx:128-138`; `fartcode-app/src/commands/issues.rs:89-108` | Pass `projectId` to `issue_update` and reject cross-project ids in core; disable the apply button when the issue is not on this board |
| GAP-17 | `approve N → Backlog` and `✓ N issues added to Backlog` are hardcoded while the backend lands on the project's `is_landing` column — the card names the **wrong destination** on any customised board | unrepresentable-state | high | PM-28 | `ProposalCard.tsx:67, 242` vs `issue_proposal.rs:119-121`; design_handoff_v3 README "Migration/seed notes" | Read the landing column from `useColumns` (as `pmPrompt.ts` already does) and interpolate its name |
| GAP-18 | **Clicking a pane does not activate it.** `activePaneByTask` only moves via a tab-chip click, so the accent underline can point at a pane the caret is not in and ⌘W / ⌘⇧T / ⌘T act on the wrong pane | unrepresentable-state | high | TASK-21 | `components/TerminalView.tsx:86-96`; `store/tabs.ts:389-395`; `TabBar.tsx:24-26`; ADR-0033 §5 | Call `useTabs.setActivePane` from `TerminalView`'s (and `DiffView`'s) focus/click handler, keyed by the pane prop already passed through `TabRenderProps` |
| GAP-19 | **Destructive terminal teardown with no confirm.** ⌘W on the sole agent tab (no tab bar, so no `×` either), and ⌘\ collapsing a split that holds the agent, both kill a running agent silently — while §7a insists the delete path itemise "kills the running agent" | missing-confirm | high | TASK-24 · TASK-22 · NAV-G22 · NAV-43 | `store/tabs.ts:293-301`, `:341-355`; `lib/terminals.ts:135-139`; `commands.ts:407-421`; contrast `Modals.tsx:468-473`; design_handoff_v2 §7a | Gate a close/collapse that would kill a **running agent** behind the §7a overlay-card confirm (`esc keep` / `↵ close — kills the agent`); plain shells stay unconfirmed |
| GAP-20 | **Restart mislabels agents as shells.** `detach_all` kills the agent PTY on window close; `ensureTabs`/`reconcile` respawn a plain `$SHELL` for the dead id and keep the persisted title, so a tab reading `claude` is a bare shell — invisible because a single tab hides the bar. Agent terminals never run under tmux, so they cannot survive a restart at all | unrepresentable-state | high | TASK-35 · CROSS-G2 · CROSS-06 | `fartcode-app/src/lib.rs:57-63`; `fartcode-app/src/terminals.rs:620-626`; `store/tabs.ts:118-139`; ADR-0021/0028/0033 | Persist the tab's `agent` provider alongside its id and respawn via `terminal_open_agent`; or drop agent-kind tabs into the 5b "nothing running · Resume the agent ⌘T" empty state |
| GAP-21 | **`task.status` never changes.** `TaskStore::update_status` has no production caller and E3-05's hook server is unbuilt, so every task fartCode has ever created is `in_progress` forever. Everything keyed on it is therefore dead or wrong: the hollow needs-you ring (`TaskHeader`), the rail dot, the flyout's Needs-you/Running/Recent grouping, the board's failed / stopped / queued vocabulary and frame 4a's `↵ read`, honest elapsed on a running card, and any notion of a task being "done" | unreachable | high | TASK-30 · TASK-42 · BOARD-35 · BOARD-45 · LIFE-G9 · NAV-G01 | `fartcode-core/src/tasks/mod.rs:376` (no callers); `components/TaskHeader.tsx:51-55`; `Nav.tsx:23-32, 158-172`; `components/board/runState.ts:6, 75-124`; `pty/env_allowlist.rs:163`; MEMORY.md "task.status never changes today" | Decide it explicitly in an ADR amendment: either build the E3-05 hook server and write `TaskStatus::Review`/`Done`, or re-derive needs-you/failed from terminal exit codes + `useScripts.agentByTask` (hydrated at flyout/rail render, as the board already does) and delete the ring from the dot vocabulary until it can be driven |
| GAP-22 | The project-root workspace is never registered with the fs watcher, so the **project-scope Changes sheet does not live-refresh** — nothing updates until a row action forces a refetch | dead-end | high | SHIP-GAP-10 | `fs_watch/mod.rs:363` `boot_targets` (tasks-only join); `watchers.rs:39-51` | Register `projects.repository_workspace_id` at boot and on project open; or refetch on an interval while that sheet is visible |
| GAP-23 | A refetch error **after** a snapshot exists is swallowed — a deleted or broken worktree keeps showing three stale rows forever, with no error and no retry | unrepresentable-state | high | SHIP-GAP-11 | `ChangesSidebar.tsx:296` (`entry?.error && !snapshot`); `store/changes.ts:56-58` | Render a persistent error strip above a stale snapshot, with retry |
| GAP-24 | Stage / stage-all / unstage / discard failures are **unhandled promise rejections** — every call site is `void store.x(...)` with no `.catch`, so no error surfaces anywhere and the follow-up refetch never runs | dead-end | high | SHIP-GAP-19 | `ChangesSidebar.tsx:133`, `:443`, `:532`, `:551`; `store/changes.ts:78-96` | Catch at the call sites and render into the same inline error slot the commit card uses |
| GAP-25 | Per-file stage/unstage/discard is **mouse-only**: rows are `tabIndex={-1}` with no arrow navigation, so `active` is never set and `s`/`u`/`d` no-op for a keyboard user. Only `a` (stage all) is reachable | no-affordance | high | SHIP-GAP-20 | `ChangesSidebar.tsx:488`, `:492-493`, `:178-189` | Add roving-tabindex row focus (↑/↓ to move, `tabIndex={0}` on the active row) — the key handlers already read `active` |
| GAP-26 | The single-key target is the **last-hovered** row and is never cleared or visually marked, so `d` can open a discard confirm for a file the pointer left long ago | unrepresentable-state | high | SHIP-GAP-21 | `ChangesSidebar.tsx:492-493` (no `onMouseLeave`), `:186-189` | Clear `active` on mouse-leave and render a visible target treatment on the active row |
| GAP-27 | Closing a **dirty diff tab** (× or ⌘W) discards unsaved edits with no confirm — `closeTab` has no dirty check and only terminal tabs get special handling | missing-confirm | high | SHIP-GAP-30 · NAV-G23 · NAV-44 | `TabBar.tsx:39-54`; `store/tabs.ts:253-302`; `store/diffs.ts:128-137` | Gate `closeTab` on `dirtyByTab` with the same inline overlay-card confirm the discard flow uses (save / discard / cancel) |
| GAP-28 | Comment gutter markers are filtered by side but **not by file path**, so every file's diff shows every other file's markers — and unsorted ranges risk a `RangeSet.of` throw | unrepresentable-state | high | SHIP-GAP-38 | `DiffView.tsx:231-252` vs `CommentThread.tsx:47-49`; `line_comments/mod.rs:195` | Filter on `c.filePath === params.path` inside `commentGutter` and sort the ranges by position before `RangeSet.of` |
| GAP-29 | Creating a PR for an already-committed, already-pushed branch is **unreachable**: the only caller of `git_create_pr` is gated on staged files + a commit message; the palette has no PR verb and the PR tab has no create path | unreachable | high | SHIP-GAP-45 | `CommitCard.tsx:29` + `:147-160`; single call site at `CommitCard.tsx:63`; `commands.ts:217-220`; `PullRequestPanel.tsx` | Add a standalone "Open PR" action — the PR tab's no-PR empty state is the natural home — calling `git_create_pr` independently of the commit path |
| GAP-30 | **PR sync failures are never surfaced.** A revoked token, a rate limit or an offline machine leaves the stale cached PR rendering as current; `tokenPresent` stays true so the token gate never returns, and nothing anywhere shows a last-sync time, a failure count, or a rate-limit reset | unrepresentable-state | high | SHIP-GAP-57 · CROSS-G7 · CROSS-21 · CROSS-22 | `commands/github.rs:95-115` ("Sync failures are logged, never surfaced"); `pr_sync/mod.rs:200-205`; `fartcode-git/src/pr_sync.rs:184-197`; `store/pr.ts:43-56` | Return the sync outcome from `pr_section_sync`; render "synced <ago>" plus a failure reason and `reset_at` in the footer; re-show the token gate on `GithubAuth` |
| GAP-31 | Merged/closed PRs never leave the cache as `open` — the branch query is `state=open`, so a merged PR renders as open/mergeable forever and the commit card stays permanently degraded to "PR already open — push instead" | unrepresentable-state | high | SHIP-GAP-58 | `github/client.rs:76-88`; `pr_sync.rs:99-106`; `pr_sync/mod.rs:153-169` | Query `state=all` (or re-fetch known cached PR numbers by id) so a merge transitions the row; add a regression test for the merged transition |
| GAP-32 | Every external link in the PR tab is a raw `<a target="_blank">` under a `default-src 'self'` CSP — the PR number, commit subjects, check `logs` and comment bodies are all **dead clicks**. It is the only panel in the app that does not use `open()` from the shell plugin | dead-end | high | SHIP-GAP-59 | `PullRequestPanel.tsx:195-206`, `:262-266`, `:301-309`, `:381-387`; `tauri.conf.json` CSP; `CommitCard.tsx:10` for the working pattern | Replace with `onClick={() => open(url)}` from `@tauri-apps/plugin-shell` (already permitted by `shell:allow-open`) |
| GAP-33 | **Async command outcomes are console-only.** A missing agent binary on ⌘T, ⌘⇧O with no `omp`, ⌘. with no live agent, `r` on an unconfigured lifecycle script, ⌘⇧T/⌘D rejections, and all four palette git verbs (fetch/pull/push/publish — including a `pull --ff-only` divergence) all resolve or reject into `console.error` with zero user-visible change. Success is equally silent | dead-end | high | NAV-G11 · TASK-16 · TASK-18 · TASK-45 · FIRST-51 · SHIP-GAP-48 · NAV-47 | `lib/commands.ts:47-49, 213, 279-281, 294-296, 319-325, 342-343, 357-359`; `components/Drawer.tsx:33-38`; `commands/lifecycle.rs:104-109` | Add one shared inline error/status surface (the changes panel, the task empty state and the board already have error slots) and route every command rejection **and** git success into it |
| GAP-34 | **Delete and archive have no affordance on any task surface.** `setDeleteTaskTarget` has exactly one caller — the ⌘⌫ registration — so the only mouse path is a ⌘K row. Flyout rows, `TaskHeader`, `TabBar`, board cards and card detail carry nothing; the card-detail footer's destructive key deletes the *issue*, and right-click on a rail tile deletes the *project* | no-affordance | high | LIFE-G1 · LIFE-04 · LIFE-05 | `app-frontend/src/lib/commands.ts:263` (sole caller); `Nav.tsx:204-226`; `TaskHeader.tsx:64-92`; handoff v3 `FLOWS.md:83` "Design has no delete/archive affordance or confirm anywhere" | Add a hover-revealed ⌘⌫ key on flyout rows and a `⌘⌫ delete · a archive` pair in the task-header overflow, both routing to the same confirm; take the frame to design review (F11 is still unspecified) |
| GAP-35 | **Archiving from the task view puts you back inside the archived task.** `task:archived` → `load()` restores `selectedTaskId` from persisted view state that `doArchive` never cleared, and `list_tasks` does not filter archived rows — so you sit in a task the flyout says does not exist, with no "archived" badge anywhere | dead-end | high | LIFE-G2 · LIFE-29 | `Modals.tsx:409-417`; `store/sidebar.ts:87-91, 224-231, 260`; `fartcode-app/src/commands/tasks.rs:248` | Call `persistSidebarView()` (or a dedicated `clearSelection`) in `doArchive`, make `load()` reject archived ids for restore-selection, and add an "archived" badge to `TaskHeader` so the state is representable |
| GAP-36 | **Archive does not stop the agent.** `task_archive` only writes `archived_at`; the terminal stays `running`, the tmux session survives and spend continues — on a task the flyout and rail no longer show, so ⌘. (task-view scope) is unreachable without un-hiding it | unrepresentable-state | high | LIFE-G3 · LIFE-30 | `fartcode-app/src/commands/tasks.rs:269`; `fartcode-core/src/tasks/mod.rs:144` ("the reference reaps the session in 'archive' mode — E2-05") | Either reap sessions on archive (the reference's archive mode) or refuse to archive with a live agent unless the confirm says "stops the running agent" |
| GAP-37 | The delete confirm **never warns about uncommitted work**, even though ADR-0023 justifies its `force = true` dirty-check bypass by saying the dialog carries the warning. Confirming `rm -rf`s the dirty tree | missing-confirm | high | LIFE-G4 · LIFE-12 | `decisions/0023-task-deletion-teardown.md` item 5; `fartcode-core/src/projects/worktrees.rs:282`; `Modals.tsx:467-481` | Read `git_status` for the workspace in the confirm and render `N uncommitted files will be destroyed` in `--fc-bad` above the delete key |
| GAP-38 | Deleted projects' tasks stay in the ⌘K index until the next restart (the cascade fires no `TaskDeleted`); selecting one navigates into a **phantom task view** — empty pane, ⌘T fails into the console, no exit except clicking another rail tile | dead-end | high | LIFE-G6 · LIFE-42 | `fartcode-app/src/indexer.rs:39-59`; `CommandPalette.tsx:157-164`; `store/sidebar.ts:118-127` | Sweep `search_index` by `project_id` on `ProjectDeleted`; make `selectTask` refuse ids absent from `tasksByProject` |
| GAP-39 | **Board issues are not indexed** — ⌘K cannot find a card until dispatch creates a task named after it, so the entire Backlog is invisible to search | no-affordance | high | NAV-G07 · NAV-18 | `fartcode-app/src/indexer.rs:28-59`; `dispatch.rs:105-114`; FLOWS.md §5 frame 8h; ADR-0038 (`item_type "feature"`) | Subscribe the indexer to issue created/updated/deleted events with `item_type: "issue"`; route ↵ to card detail |
| GAP-40 | Re-applying keybinding overrides replays them in **registration order against freshly reset defaults**, so any chord swap the UI accepted is silently discarded with only a `console.warn` — the row re-renders as its default with no `custom` tag and no error | dead-end | high | NAV-G18 · NAV-37 | `useCommands.ts:87-102`; `registry.ts:180-207` | Clear the target chord from every command before the conflict check, or apply all overrides first and resolve conflicts afterwards; surface rejections in the UI instead of `console.warn` |
| GAP-41 | A DB that cannot be opened or migrated (corrupt file, unwritable path, migration hash mismatch) fails inside the Tauri `setup` hook — **no window, no message, no recovery path** | dead-end | high | CROSS-G10 · CROSS-25 | `fartcode-app/src/lib.rs:64-65` (`App::init(...)?` inside `setup`); ADR-0001 migrations | Open the window first, run init after, and render a failure card naming the DB path with a "reveal in Finder" action |
| GAP-42 | **A failed agent settles a step exactly like a successful one.** A non-zero exit advances the card; no exit code, no artifact check, and no failure branch exists anywhere in the settle path — so an expired token or a bad flag lands work in review | unspecified | high | CROSS-G11 · CROSS-26 | `terminals.rs:389-403` (pump → `flip_for_exited_agent` regardless of code); `step_engine.rs::settle_issues_for_task` never reads an exit code | Pass the exit code into the settle; on non-zero hold the card and mark it needs-you (the board already has a "failed card · ↵ read" affordance) |
| GAP-43 | **Unbounded chained spend.** A chain of run-mode columns launches an agent per hop with no user gesture, no depth cap, no per-project budget and no ledger — a settle in a project the user is not even looking at can spend N times | missing-confirm | high | CROSS-G14 · CROSS-41 | `step_engine.rs::settle_issues_for_task` → `enter_column` (chains legal by design); no budget code exists | Cap chain depth per settle epoch, and/or add a per-project "confirm after N automatic launches" setting |
| GAP-44 | The entire **E19 dossier consent surface is unbuilt**: no `dossier_path` field, no consent gate before repo writes, no project-settings switch in either direction, no overlay ordering with the queue confirm. Repo-writing consent has no implementation | missing-confirm | high | CROSS-G21 · CROSS-50 · CROSS-51 · CROSS-52 · CROSS-53 · CROSS-54 | ADR-0038 item 3 + Consequences; handoff v3 README §8e; no `dossier` symbol anywhere in `fartcode-core` / `app-frontend` | Build the gate with the settings key first (declining must be cheap and permanent), then the writer — never ship the writer before the gate |

### Medium

| ID | Gap | Type | Severity | Source | Evidence | Suggested resolution |
|---|---|---|---|---|---|---|
| GAP-45 | Deleting a project is **right-click-only**: no command, no palette entry, no keyboard path, and the sole discovery hint is the tile's `title` tooltip. Its confirm itemises nothing — no task count, no worktree count | no-affordance | medium | FIRST-55 · LIFE-G8 · NAV-G02 · NAV-06b · LIFE-40 | `Nav.tsx:90-101`; no `delete-project` id in `lib/commands.ts` / `registry.ts:40-77`; `Modals.tsx:780-790` | Register a `delete-project` command (project-view scope, palette-visible, rebindable), add a visible affordance in the flyout head, and itemise `N tasks · N worktrees` the way `DeleteTaskConfirm` does |
| GAP-46 | Re-adding an existing project path silently appends a **duplicate rail tile** — the backend returns the existing project without a `project:added` event, and the store appends unconditionally, so one project shows twice until the next `load()` | unrepresentable-state | medium | FIRST-15 | `store/sidebar.ts:148-155`; `projects/mod.rs:350-353` | Dedupe by id in `createProject`, and tell the user the project was already open |
| GAP-47 | A non-git directory is rejected with **no "initialize a repository here" affordance**, even though the backend already takes `init_if_missing` | no-affordance | medium | FIRST-05 | `fartcode-app/src/commands/projects.rs:21` (hardcodes `false`) | Offer an "initialize git repo" confirm on the `not a git repository` error and pass `init_if_missing: true` |
| GAP-48 | `projectSettingsOpen` has **no caller** — the per-project settings entry point (the "sidebar gear") vanished with `ProjectHeader.tsx`, `Modals.tsx` still renders the dialog, and `closeTopModal` carries a branch that can never fire | unreachable | medium | FIRST-35 · NAV-G31 · NAV-60 | `store/ui.ts:53, 130, 154, 165`; `Modals.tsx:771-779`; zero `setProjectSettingsOpen(true)` call sites; `ProjectView.tsx:1-3` comment still references it | Either add a project-scoped settings command/affordance in the unified header, or delete the dead flag, its `Modals.tsx` branch and its `closeTopModal` arm |
| GAP-49 | A **rejected settings save leaves the invalid value rendered** in the row — `commit()` sets optimistic state and never rolls it back, so the UI shows a value that was never persisted | unrepresentable-state | medium | FIRST-37 | `ProjectSettings.tsx:227-239`; `commands/settings.rs:45-63` | Restore the previous settings object in the `catch` before setting the error |
| GAP-50 | The "Default agent · model" row lives in a **project** pane but writes the app-wide `defaultAgent` setting, so it changes every project; with zero projects the app default cannot be set at all, the App pane only displays it, and the row has no model half (its value is the literal `· default`) | unspecified | medium | FIRST-32 · NAV-G29 · NAV-58 | `ProjectSettings.tsx:243-252, 505-535`; `commands/settings.rs:96-117`; `AgentsList.tsx:56-61`; `settings/registry.rs:204` | Move the picker to Settings → App beside the Detected list (leaving the project row a real per-project override or read-only) and add the model half 7c specifies |
| GAP-51 | A configured **teardown script never runs automatically** — not on task delete, not on project delete, only manually from the drawer while the task still exists. Containers and ports outlive the worktree they belonged to | no-affordance | medium | FIRST-52 · LIFE-G11 · LIFE-24 | `commands/lifecycle.rs::auto_run_enabled` (Teardown → `Some(false)`); `fartcode-core/src/tasks/deletion.rs:40-43`; ADR-0014; ADR-0023 line 32 | Run `scripts.teardown` in the worktree before removal with the existing bounded wait, and itemise `runs teardown script` in the confirm |
| GAP-52 | **Esc does nothing while onboarding is open**, even though `onboardingOpen` counts as a modal for scope purposes — so every view-scoped key is suppressed and the one key that should dismiss it is inert | dead-end | medium | FIRST-09 · NAV-G16 · NAV-29 | `store/ui.ts:145-156` (`closeTopModal` has no `onboardingOpen` branch), `:159-172` (`modalOpen` includes it) | Add an `onboardingOpen` branch to `closeTopModal` that calls the same `finish()` path as `skip`, recording completion |
| GAP-53 | Dismissing a queue confirm clears the frontend flag but leaves the **backend park alive and invisible**: the card shows nothing and the step can only be fired by dragging it out and back in | unrepresentable-state | medium | BOARD-15 | `BoardView.tsx:477-485`; `step_engine.rs:639-661` | On dismiss, keep the queued dot (the park is still pending) and give the card a `↵ dispatch` affordance; or drop the park and emit `step:queue_cleared` |
| GAP-54 | Parks are **memory-only**, so a restart silently loses every pending confirm; the card sits in a queue column with no state the UI can represent and no user-reachable way to re-park (the engine only re-parks on a settle trigger the user cannot cause) | unrepresentable-state | medium | BOARD-41 | `step_engine.rs:66-72`; `StepEngine::parked` (in-memory `HashMap`) | Either persist parks, or on board load re-park any card resident in a queue-mode `agent_step` with no live session and surface the confirm |
| GAP-55 | `enter_column`/`move_to` write one card's `position` without shifting siblings, so two cards routinely share a position and the rendered order falls back to the `created_at` tiebreak — **drag-to-reorder is unreliable** | unspecified | medium | BOARD-11 | `fartcode-core/src/issues/mod.rs:598-623`, `:669-737` | Reindex the target column's cards inside the same transaction, or switch to fractional positions |
| GAP-56 | The 4 s `claimLaunch` dedupe is **time-based, not identity-based**, so a legitimate second launch for the same issue+column (a fast out-and-back drag, or a settle-chained re-entry) is silently swallowed — the backend records a launch and emits `step:launch` while the frontend opens no terminal and shows no error | dead-end | medium | BOARD-31 · CROSS-G12 · CROSS-32 | `BoardView.tsx:109-124` (`LAUNCH_DEDUPE_MS`) | Carry a launch id/nonce on both the command outcome and the `step:launch` event so the pair dedupes by identity rather than by a time window |
| GAP-57 | Narrow mode renders only the focused column and gives strip entries **no drop handlers** — there is no mouse path to move a card between columns on a laptop; ⇧h/⇧l is the only route | no-affordance | medium | BOARD-39 | `BoardView.tsx:690-694, 825-846` | Make strip entries drop targets (drop on `<name> <count>` = enter that column), keeping the same gates |
| GAP-58 | Background network work on project entry runs **with no consent, no opt-out, and silence in both directions**: the board imports every open GitHub issue on every board entry (60 s cooldown) and the rail auto-pulls the project root (30 s cooldown); failures are `console.warn` only and success is never announced | missing-confirm | medium | BOARD-09 · CROSS-G8 · FIRST-20 · CROSS-44 · CROSS-23 | `BoardView.tsx:87-107, 224-231`; `store/sidebar.ts:44-54` | Add a project setting for auto-import, a manual "import from GitHub" key to replace the copy the empty state still promises, and route both outcomes through one quiet status line instead of the console |
| GAP-59 | The empty board points at **"the GitHub key above", which no longer exists** — `ProjectHeader.tsx` was deleted and `App.tsx` renders no header at project scope. It also hides the column heads entirely, so the "drag one into Quick" instruction has no visible target | dead-end | medium | BOARD-06 | `BoardView.tsx:808-821`; `App.tsx:62-79` | Rewrite the copy to the affordances that exist, and render the (empty) column heads under the empty-state text |
| GAP-60 | A failed `issue_list` renders **"The board is empty."** with an add-issue key; a failed `column_list` renders **"This project has no columns."** — both assert a state the app does not know. Board errors are also sticky: `setError(null)` is never called and there is no retry | unrepresentable-state | medium | BOARD-08 · BOARD-07 | `BoardView.tsx:768-821` (`shown`), all `setError` sites | Add a distinct read-failed state with a retry key, and clear the error on any successful refetch |
| GAP-61 | `issue_apply_proposal` **never re-runs `parse_proposal`**, so a user-edited duplicate title creates two same-titled issues and `own_titles` (a HashMap) silently mis-wires the blocked-by edge to the last one. No error, no warning | unspecified | medium | PM-22 | `fartcode-app/src/commands/issue_proposals.rs:28`; `issue_proposal.rs:74-82, 131` | Validate inside `apply_proposal` (or run `parse_proposal`'s checks on the deserialised value), and block the rename in the card when it collides |
| GAP-62 | A project with a **null `repository_workspace_id`** cannot open the PM chat at all — ⌘⇧2 silently does nothing, with no panel, no error and no alternative path, even though the PM agent needs only the project root | dead-end | medium | PM-05 | `ChangesSidebar.tsx:102` (`!taskId && !workspaceId` → null); `acp_runtime.rs:325-328`; `projects/provider.rs:64-70` | Gate the sheet on `taskId \|\| workspaceId \|\| projectChatOpen` |
| GAP-63 | Switching projects renders the **previous project's transcript under the new project's owner key**; if the new project's start fails the stale transcript stays alongside the error, sends route to the old conversation, and approving would write project B's board from project A's proposal | unrepresentable-state | medium | PM-06 · PM-48 | `ProjectChatPanel.tsx:17, 36, 52-54` | Reset `conversationId`/`error` to null at the top of the `projectId` effect, and key `ConversationView` on `projectId` |
| GAP-64 | The approval card shows **only titles** — bodies and acceptance criteria are written sight-unseen, weakening ADR-0032's "hard human gate" | missing-confirm | medium | PM-26 | `ProposalCard.tsx:186-232`; ADR-0032 §5 | Add a per-row expander (or `↵`-preview) revealing body + acceptance before approve |
| GAP-65 | Ticket-edit apply replaces the whole body and acceptance list with **no old-vs-new view and no undo** — the user cannot see what is being destroyed, and the previous body is recoverable only by retyping it | missing-confirm | medium | PM-41 | `TicketEditCard.tsx:110-127`; `issues/mod.rs:532-537` | Render body/acceptance as a diff against the current issue (the Title row already does old → new); consider a one-step undo that re-applies the captured previous values |
| GAP-66 | `"acceptance": []` is a valid edit that **silently clears every criterion**, presented only as `Acceptance (0)` above an empty list | missing-confirm | medium | PM-44 | `lib/ticketEdit.ts:38-43`; `TicketEditCard.tsx:117-127` | Label the empty case explicitly ("clears all N criteria") and require a second confirm |
| GAP-67 | The PM agent is the **first ACP-capable provider in the static registry**, ignoring `defaultAgent`; nothing in the panel names the provider or model and there is no picker (the `no ACP-capable provider available` error is dead code — the registry returns regardless of what is installed) | no-affordance | medium | PM-52 | `ProjectChatPanel.tsx:27-30`; `fartcode-providers/src/lib.rs:139` | Resolve the provider from the `defaultAgent` setting with the registry as fallback, and surface it in the panel header |
| GAP-68 | The PM panel has **no mouse close/minimize** — `.project-chat-minimize` is styled and used by `TaskChatPanel`, but `ProjectChatPanel` never renders it, and no project header survives. The only exits are ⌘⇧2, ⌘⇧1, opening a card, or the palette | no-affordance | medium | PM-04 | `ProjectChatPanel.tsx:48-51` vs `TaskChatPanel.tsx:47-56`; `project-chat.css:39` | Render the same chevron button, calling `setChangesOpen(false)` |
| GAP-69 | **PRDs the PM writes cannot be opened anywhere in the app** — the proposal card header is a `title` tooltip, prose mentions are deliberately inert, and card detail renders the path as bare `<code>` | dead-end | medium | PM-13 | `ProposalCard.tsx:177-184`; `project-chat.css:123-129`; `CardDetail.tsx:496-503` | Make the PRD path open a read-only markdown view in the sheet (or shell-open the file) until the E5 file surfaces land |
| GAP-70 | "Ask PM about this ticket" is reachable **only by mouse-selecting body text** — no command, no button, no chord — and a bodyless ticket lets the user select the UI placeholder (`No description — double-click to add one.`) and send it as ticket content | no-affordance | medium | PM-38 | `CardDetail.tsx:151-172, 366-374` | Add an `Ask PM` button/command scoped to the open card (selection optional), and exclude the placeholder node from the selection host |
| GAP-71 | Dropping a proposal row is **keyboard-only** (`x`) with no mouse affordance, and the focused card renders `outline: none` so nothing advertises that the footer's `e`/`x`/`↵` keys are live | no-affordance | medium | PM-24 · PM-18 | `ProposalCard.tsx:156-157, 196-199`; `project-chat.css:301-303` | Add a hover-revealed drop control per row and a visible `:focus-visible` ring on the card |
| GAP-72 | The 5b **"stopped · elapsed" empty state is unreachable on the normal path**: agent completion leaves a dead terminal on screen, and the stop-reason label + key list only render at `tabs.length === 0`, i.e. after the user manually presses ⌘W | unreachable | medium | TASK-29 | `components/TaskView.tsx:36`, `:126-133`; `lib/terminals.ts:96-104`; design_handoff_v2 §5b | On `terminal:exited` for an agent terminal that is the pane's only tab, drop the tab (after a beat) so the pane falls to the 5b state, seeded with `agentByTask.exitedAt` |
| GAP-73 | **⌘\ is double-bound and the shadowing is invisible.** `split-pane` (task-view) outranks `toggle-sidebar` (global), so ⌘\ splits inside a task instead of collapsing the flyout — contradicting the settled keymap — and because `hint()` and the Keys pane render only `chords[0]`, `toggle-sidebar`'s ⌘\ appears nowhere in the UI | unspecified | medium | TASK-27 · NAV-G04 · NAV-09 | `lib/commands.ts:145-151` vs `:422-434`; `lib/registry.ts:31-38, 152-157, 171`; `SettingsModal.tsx:113-126`; design_handoff_v2 README "Keymap" | Drop ⌘\ from `split-pane` (⌘D already covers it), render **all** chords in the Keys row, and add a real-registry conflict test to `registry.test.ts` |
| GAP-74 | The task empty state **fabricates a stop reason**: a task whose agent never launched (no binary installed) shows `stopped · now`, derived from `statusChangedAt` = creation time, and nothing names the missing binary | unrepresentable-state | medium | TASK-07 | `components/TaskView.tsx:126-133`; `commands/tasks.rs:142-147` | When `agentByTask[taskId]` has no ids and no `exitedAt`, render `nothing running` (the string already exists) instead of falling back to `statusChangedAt` |
| GAP-75 | The ⌘J drawer offers **all three lifecycle tabs regardless of configuration** while the header only shows configured launchers — so an unconfigured script advertises a rerun that always fails | dead-end | medium | TASK-45 | `components/Drawer.tsx:59-77` vs `components/TaskHeader.tsx:33-45` | Filter the drawer's tabs by the same configured-script set, and show a `configure in project settings` line when none are set |
| GAP-76 | **Queue-vs-start-now was never settled.** Creation always spawns (`create_task` always calls `launch_default_agent`), so there is no way to create a task without burning a session, and no concurrency limiter | unspecified | medium | TASK-06 | `commands/tasks.rs:57-75`; FLOWS §F5 | Settle it: an explicit second footer action (`⌘↵ create without starting`) or an app-level concurrency cap that parks the launch. Record the call in an ADR |
| GAP-77 | At **project scope, change rows cannot open a diff** — the project checkout is reviewable but not readable; the only signal is a tooltip | dead-end | medium | SHIP-GAP-03 · SHIP-03 | `ChangesSidebar.tsx:489-504` (`no-diff` class, `if (!taskId) return`) | Open project-scope diffs in a project-owned pane, or make the row's disabled state explicit rather than a tooltip |
| GAP-78 | The **>10 000-file truncated state hides the commit card and footer**, leaving no in-app action at all — no stage-all, no commit, no `.gitignore` affordance | dead-end | medium | SHIP-GAP-08 | `status.rs:27` `MAX_STATUS_FILES`; `ChangesSidebar.tsx:303-306` | Keep the commit card + footer rendered in the truncated branch; add a "stage all / open in terminal" escape |
| GAP-79 | Discard of an **untracked** file hard-deletes with no undo and no trash — an hour of un-added agent output is `remove_file`'d on one confirm | missing-confirm | medium | SHIP-GAP-18 · SHIP-18 | `stage.rs:64-67` | Move untracked discards to the OS trash (or a `.fartcode/trash` holding pen) so the action is reversible |
| GAP-80 | **Conflicted rows expose no mouse affordance** to mark a conflict resolved — no `s`, `d` or `u` in either section; the only path is hovering the CHANGED-side row and pressing `s`, which the footer hint never mentions | no-affordance | medium | SHIP-GAP-22 | `ChangesSidebar.tsx:521-522`, `:178-181`; `status.rs:147-181` | Give conflicted rows an explicit `resolve` action (or reuse `s`) and mention it in the footer hint |
| GAP-81 | An external write while a diff tab is **dirty** is silently ignored; the next save overwrites the agent's version with no divergence warning and no reload-vs-keep choice | unrepresentable-state | medium | SHIP-GAP-28 · SHIP-28 | `DiffView.tsx:368-369` | Badge the header ("changed on disk") when a deferred refresh is pending, and offer reload-vs-keep |
| GAP-82 | Deleting a line comment is a **single unguarded click** that also orphans any linked task — the row is the only pointer back to the review context | missing-confirm | medium | SHIP-GAP-36 · SHIP-36 | `CommentThread.tsx:156-162`; `commands/line_comments.rs:118-121` | Reuse the `fc-confirm` overlay and mention the linked task in the body |
| GAP-83 | **No running agent can invoke `add_line_comment`** — the host entry point, guardrails and events all exist, but no MCP tool registration means there is no transport (ADR-0035 deferred it) | unreachable | medium | SHIP-GAP-39 · SHIP-39 | ADR-0035 item 3; `commands/line_comments.rs:72-93`; MEMORY.md §E4 | Track as an E-ticket against `fartcode-integrations`; until then do not advertise "agents add comments" in product copy |
| GAP-84 | **"Commit, push & open PR" opens a GitHub *compare* form** — the PR is not created, and nothing in the app says so; the PR tab keeps reading "no open pull request" until the user submits the browser form and a sync runs | unrepresentable-state | medium | SHIP-GAP-43 · SHIP-43 | `commit.rs:182-231`; `CommitCard.tsx:58-70` | Either create the PR via the GitHub API (client and token already exist) or relabel the row "Commit, push & draft PR in browser" |
| GAP-85 | `CommitState.upstream / ahead / behind / remotes` are fetched on **every** git event and rendered nowhere, despite the DTO comment naming the footer as the consumer | unrepresentable-state | medium | SHIP-GAP-49 · SHIP-49 | `commit.rs:56-63`; `GitFooter.tsx:84-86` | Render `↑n ↓n <upstream>` in the footer hint line, or drop the fields from the DTO |
| GAP-86 | **No merge affordance and no merge hand-off** — the footer states `mergeable` and stops; with GAP-32 the PR number is a dead click, so the app offers no route to landing the change | no-affordance | medium | SHIP-GAP-60 · SHIP-60 | `PullRequestPanel.tsx` (no merge call); no merge command in `fartcode-core/src/github/client.rs` | Ship at minimum an "open on GitHub" action (fixes with GAP-32); decide separately whether in-app merge is in scope |
| GAP-87 | The branch always survives and **nothing can delete it**: the frontend never passes `deleteBranch: true` and no surface offers branch cleanup, so `fartCode/*` branches accumulate one per deleted task | no-affordance | medium | LIFE-G12 · LIFE-23 | `app-frontend/src/lib/tauri.ts:133-144`; `fartcode-core/src/tasks/deletion.rs:156`; ADR-0023 item 6 | Turn the confirm's `branch X is kept` line into a toggle (`b` also deletes the branch), or add a palette command to prune merged `fartCode/*` branches |
| GAP-88 | The delete confirm **omits the tmux session line** the handoff specifies, and its terminal count comes from the in-memory manager — so after a restart with tmux durability it can read `0 terminals` while sessions are alive | missing-confirm | medium | LIFE-G13 · LIFE-11 | handoff v2 `README.md:82`; `Modals.tsx:357-362`; `fartcode-app/src/terminals.rs:193` | Expose a `terminal_list_persisted(task)` (or decode live tmux names by prefix) and itemise `kills tmux <session>` |
| GAP-89 | The delete confirm **never mentions the linked board card**; on confirm the FK clears `linked_task_id` and the card is stranded mid-column, and the step engine has no `on_task_deleted` to clear its park or launch-registry entry | missing-confirm | medium | LIFE-G14 · LIFE-13 · LIFE-46 | `Modals.tsx:467-481`; `migrations/0002_issues.sql:17`; `fartcode-app/src/step_engine.rs:909-929` (only `on_issue_deleted`/`on_project_deleted`) | Add an `unlinks card "<title>"` row to the confirm and a `step_engine::on_task_deleted` that clears the park + registry entry |
| GAP-90 | Deleting a **card** orphans its task, worktree and running agent — the "Delete this issue?" confirm says nothing about any of them | missing-confirm | medium | LIFE-G15 · LIFE-44 | `CardDetail.tsx:532-576`; `fartcode-core/src/issues/mod.rs:741-753` | Itemise the linked task in the issue confirm and offer "also delete the task" (routing through `TaskDeletionService`) |
| GAP-91 | **Archived tasks are enumerable nowhere** — no archive list in the flyout, the board or settings, and a blank ⌘K query returns no task rows, so a task whose name you forgot is lost | no-affordance | medium | LIFE-G16 · LIFE-36 | `fartcode-core/src/search.rs:102` (blank query → `[]`); `Nav.tsx:158-179` | Add an "Archived" section to project settings (or a palette command "Show archived tasks") listing `archivedAt` rows with restore/delete keys |
| GAP-92 | Restoring an archived task after a restart leaves its worktree **unwatched** — `boot_targets` excludes archived rows and only `TaskProvisioned` re-registers a watch, so the Changes panel never refreshes until the next restart | dead-end | medium | LIFE-G17 · LIFE-37 | `fartcode-core/src/fs_watch/mod.rs:360-368`; `fartcode-app/src/watchers.rs:38-51` (no `TaskRestored` arm) | Handle `TaskRestored` in `spawn_workspace_watchers`, and `TaskArchived` → `unregister_task` for symmetry |
| GAP-93 | `.rail` has **no overflow handling** — `.rail-tile` is `flex: none`, so past roughly 15 projects the tiles push the spacer, the `+` tile and the `⌘` settings tile off the bottom of the window, where they become unreachable | dead-end | medium | NAV-G03 · NAV-06c | `styles.css:273-285` (no `overflow`), `:295-311` | Wrap the project tiles in a scrolling region; keep the mark, `+` and `⌘` pinned |
| GAP-94 | **No Esc or back control returns the task view to the board** — `close-modal` is modal-scope only and `TaskHeader` has no `esc` affordance, so the only way out is clicking the project's rail tile | no-affordance | medium | NAV-G05 · NAV-14 | `TaskHeader.tsx:57-90`; left-nav README "Task (2b) … `esc` right-aligned"; `commands.ts` (`close-modal` modal scope) | Add an `esc` control to the task header and a task-view `close-task` command bound to Escape |
| GAP-95 | Arrow-key selection in the palette **never scrolls into view**: past row ~8 the highlight walks off the bottom of the 320 px results box and ↵ runs a command the user cannot see | dead-end | medium | NAV-G10 · NAV-23 | `CommandPalette.tsx:184-194`; `styles.css:845-851` | Call `scrollIntoView({ block: "nearest" })` on selection change |
| GAP-96 | `git-push` / `git-publish` run **with no confirm and no target shown** — a network-mutating action fires straight from a palette row with no remote/branch named | missing-confirm | medium | NAV-G12 | `commands.ts:217-220` | Show the target remote/branch in the palette row and surface success/failure in the git footer |
| GAP-97 | **Global commands fire while a modal is open and stack a dialog on top of it** — ⌘N over the palette, ⌘⇧N/⌘K/⌘, over onboarding — and `closeTopModal` pops by *registry position*, not visual stacking, so Esc dismisses the palette first and strands the composer behind an already-closed overlay | unspecified | medium | NAV-G15 · NAV-26 · FIRST-10 · NAV-28 | `registry.ts:216-228` (only view scopes are suspended by `modalOpen`); `store/ui.ts:147-157` | Close the palette before running any command, decide whether global scope survives onboarding, and make `closeTopModal` pop by actual stacking order |
| GAP-98 | `toggle-project-chat` is project-scope but **`projectView` is true inside a task view**, so ⌘⇧2 (and its palette row) fire on the task surface and open **Changes** instead — the chat is gated on `!taskId` | unspecified | medium | NAV-G17 · NAV-32 | `useCommands.ts:31-40`; `commands.ts:223-239`; `ChangesSidebar.tsx:85`; `CommandPalette.tsx:96` | Define `projectView` as `selectedProjectId !== null && selectedTaskId === null`, and audit the palette's scope filter with it |
| GAP-99 | Keybinding **conflict detection is same-scope only**, so a task-scope remap can shadow ⌘K / ⌘, / ⌘B with no warning anywhere — inside any task view ⌘K would close a tab instead of opening the palette | unspecified | medium | NAV-G19 · NAV-38 | `registry.ts:128-150, 191-198` (`other.scope === cmd.scope`), `:31-38` | Warn (not necessarily refuse) when a chord is already bound in a lower-precedence scope, naming the shadowed command |
| GAP-100 | ⌘⌥↑/↓ walks **every project's** tasks in an order no surface displays — it silently switches projects and lands on tasks the flyout never showed, using the old tree order (pinned first, collapsed projects skipped) | unspecified | medium | NAV-G24 · NAV-45 | `commands.ts:107-117`; `store/sidebar.ts:205-221` | Scope task switching to the current project's flyout order, or add a visible next/previous affordance that reveals the order |
| GAP-101 | **`togglePin` and `toggleCollapsed` — the two inputs to `visibleTaskOrder` — have no UI caller**, and the backend `task_toggle_pin` command is dead, so the pinned-first branch of the documented ordering contract can never fire | unreachable | medium | NAV-G25 · LIFE-G10 · NAV-46 · LIFE-39 | `store/sidebar.ts:132-135, 186-196, 205-221`; `fartcode-app/src/commands/tasks.rs:256`; only test mocks reference `togglePin` | Either build the affordances (pin in the flyout row / card detail, collapse in the rail) or delete both and simplify `visibleTaskOrder` |
| GAP-102 | **No telemetry consent surface exists** — no opt-out row anywhere, and nothing records that "local-only" is a commitment, so the first metric that ships could silently become the first transmission. (Nothing is transmitted today; this is a missing consent surface, not a leak.) | no-affordance | medium | NAV-G30 · CROSS-G20 · NAV-59 · CROSS-49 | PRD.md:442-450 (E15); FLOWS.md F12; `fartcode-telemetry/src/lib.rs:1` (placeholder); `tasks/lifecycle.rs:30-33` | Ship the toggle with E15; until then promote the local-only rule into DESIGN.md and state it plainly in the App pane, so any change requires editing a user-visible claim |
| GAP-103 | A run-mode drop with **no agent CLI installed** still moves the card and provisions a worktree, then fails at terminal-open with a bare error string — spend-shaped side effects happen before the precondition is checked, and the only retry is re-dragging | dead-end | medium | CROSS-G4 · CROSS-16 | `BoardView.tsx:392-431`; `commands/terminals.rs::terminal_open_agent` (`agent not installed:`) | Pre-flight `host_dependency_list` in `enter_column` for agent steps; refuse with a typed error and an "install an agent" affordance before provisioning |
| GAP-104 | **Provider auth is never checked before spending.** `provider_auth_status` exists and is called only from the accounts settings surface, so a logged-out CLI is discovered by the agent's own output — and its exit then settles the card as though the step succeeded | missing-confirm | medium | CROSS-G5 · CROSS-17 | `commands/provider_accounts.rs::provider_auth_status`; no caller in `BoardView.tsx` / `dispatch.rs` / `step_engine.rs` | Gate the launch on a cached auth probe and surface "sign in to <provider>" in the same overlay slot as the queue confirm |
| GAP-105 | Boot rehydration **skips conversations whose worktree vanished** with only a `tracing::warn!` — the task looks identical to one that resumed | unrepresentable-state | medium | CROSS-G6 · CROSS-19 | `fartcode-core/src/pty/launcher.rs:691-699` | Emit an `InternalEvent` for skipped rehydrations and render "workspace missing — re-provision" in the task's empty pane |
| GAP-106 | **Selecting a project starts an ACP provider process** before the user types anything, because `projectChatOpen` defaults to true. No prompt is sent, so nothing is billed — but a provider process runs against the user's account unasked | missing-confirm | medium | CROSS-G15 · CROSS-42 | `store/ui.ts:105`; `ProjectChatPanel.tsx:27-40` (`acpStart` in the mount effect) | Start the adapter lazily on first composer focus/submit, or default the panel closed |
| GAP-107 | Boot rehydration **resumes agent CLIs automatically** with no user-visible signal and no way to decline for this launch; the resumed/skipped summary only reaches the log | missing-confirm | medium | CROSS-G16 · CROSS-43 | `fartcode-app/src/lib.rs:88-100`; `launcher.rs::rehydrate_all` | Emit a resumed/skipped summary event, show it once in the header meta, and add a "don't resume agents on launch" setting |
| GAP-108 | **No spend visibility at all**: no token counts, no cost, no session ledger anywhere — board, card detail, task header or settings. `fartcode-telemetry` is a placeholder crate | no-affordance | medium | CROSS-G17 · CROSS-45 | `fartcode-telemetry/src/lib.rs` (placeholder only); ADR-0038 item 7 | Land local usage capture from provider metadata (the transcript reducer already sees it) before the dashboard frame (#76) |
| GAP-109 | **Auto-approve is plumbed backend-side** (conversation config + rehydrator parameter, hardwired `false` at boot) and has **no UI caller in either direction** — a user can neither enable it nor confirm it is off | unreachable | medium | CROSS-G18 · CROSS-46 | `App::init` (`false, // auto-approve defaults off on boot`); zero `autoApprove` references under `app-frontend/src` | Expose it as a project-settings row with the same shared-style provenance treatment as other repo-affecting switches |
| GAP-110 | **No OS notifications**: an agent that needs you (or hits a permission prompt) while fartCode is backgrounded produces no signal outside the app — `tauri-plugin-notification` is not a dependency | no-affordance | medium | CROSS-G19 · CROSS-48 | no `tauri-plugin-notification` dependency; no "notif" match under `app-frontend/src` or `fartcode-app/src` | Add notifications for needs-you transitions and permission requests, muteable per project |
| GAP-111 | **There is no narrow-width contract outside the board.** The only `@media` rules in the whole stylesheet set are two `prefers-reduced-motion` blocks: the rail never narrows to the 48 px DESIGN.md specifies, nothing auto-collapses the flyout or the sheet, and at the 800 px minimum window with the flyout open and the sheet at 640 px the work surface collapses to ~100 px (board) or ~140 px per split pane (task view). Only the board reacts, and it does so with a `ResizeObserver`, not a media query | unspecified | medium | NAV-G26 · NAV-G27 · SHIP-GAP-65 · FIRST-67 · TASK-47 · PM-51 · NAV-51 · NAV-52 | `styles.css:172-185` (`grid-template-columns: auto minmax(0,1fr) auto`), `:604, 1179` (the only `@media` blocks); `BoardView.tsx:80, 305-316` (`NARROW_PX = 900`); `styles/taskview.css`; `ChangesSidebar.tsx:72` (`useGutterResize(400,280,640,-1)`); `tauri.conf.json:19` (`minWidth: 800`); DESIGN.md:261-265 | Settle the laptop-width contract: below ~1100 px auto-collapse the flyout (remembering the user's explicit state), clamp the sheet to a fraction of the viewport or overlay it, narrow the rail to 48 px, and collapse the task split to one pane keeping both PTYs alive — keyed off the same measured width the board already uses |

### Low

| ID | Gap | Type | Severity | Source | Evidence | Suggested resolution |
|---|---|---|---|---|---|---|
| GAP-112 | Once skipped, **onboarding can never be reopened** — no command, no button, no reset; `setOnboardingOpen` has one non-store caller | no-affordance | low | FIRST-11 | `Onboarding.tsx:41` (single non-store caller) | Register a "Run first-run setup" palette command that clears the view-state flag |
| GAP-113 | `⇧⌘S` with nothing local still flashes **"local values moved into .fartCode.json"** — `share_with_team` returns `false` and the command drops the boolean, so the pane claims a write that never happened | unrepresentable-state | low | FIRST-45 | `commands/settings.rs:83-89`; `ProjectSettings.tsx:261-264` | Return the boolean through `project_settings_share` and branch the notice |
| GAP-114 | The `update ⌄` row branch is **dead** (`latest_version` is a Phase-0 stub returning `None`, so `updateAvailable` is never true) and **uninstall has no command or UI at all** | unreachable | low | FIRST-29 · FIRST-30 | `dependencies/mod.rs:404-408`; `AgentsList.tsx:54, 62-72`; `lib.rs:225-228` | Land E3-05's version checks; register `host_dependency_uninstall` with a confirm |
| GAP-115 | The `GitHub account` row is **free text** (7c specifies a `⌄` picker) and `github_account_id` has no consumer anywhere — PR and issue calls never read it | dead-end | low | FIRST-41 | `ProjectSettings.tsx:360-373`; only registry/serde references in Rust | Turn it into a picker over `list_provider_accounts`/GitHub tokens and make the PR/issue paths read it — or hide the row |
| GAP-116 | Onboarding step three (**"Connect GitHub?"**) is inert — both `skip` and `↵ done` do exactly the same thing; the `github_token_*` commands it could use are already registered | dead-end | low | FIRST-07 | `Onboarding.tsx:162-178`; `lib.rs:163-166` | Wire `github_token_import`/`github_token_status` into the step, or drop the step |
| GAP-117 | With **zero installed agents** the project pane's collapsed row still reads `claude · default` — naming an agent that is not present — while its own menu says "no installed agents" | unrepresentable-state | low | FIRST-33 | `store/dependencies.ts:86-88`; `ProjectSettings.tsx:509, 529-533` | Render `—` (or "none installed") when no dep has `installed && isDefault` |
| GAP-118 | **"Shell setup" only applies to lifecycle scripts** — not to ⌘⇧T shells and not to agent terminals — which the label does not convey | unspecified | low | FIRST-43 | `commands/lifecycle.rs:44-47`; `commands/terminals.rs::terminal_program` (no `shell_setup`) | Either apply it to every task terminal or rename the row ("Script shell setup") |
| GAP-119 | The **auto-run toggles arm an execute-on-create side effect** with no confirm and no undo: flipping `Auto-run run script` on means the next task creation spawns that PTY without further consent | missing-confirm | low | FIRST-48 | `ProjectSettings.tsx:570-587`; `commands/lifecycle.rs::run_auto_lifecycle_scripts` | Add a one-line consequence subline under the row ("runs `<script>` in every new worktree") |
| GAP-120 | Two of the three project sources the design calls for do not exist: **"connect remote SSH host"** and **"new GitHub repo"**. Both are deliberately deferred product areas (E12-04 / E8), listed for completeness | no-affordance | low | FIRST-17 · FIRST-18 | FLOWS.md F1/F2; `ssh_connection_id` always `None` at `projects/mod.rs:371`; `StubRepoHost::create_repository` returns an E8 stub error | Leave deferred; until then the add-project surface should not imply more than one source |
| GAP-121 | Board confirm copy **deviates from handoff §8c**: the blocked confirm hardcodes "still in progress" instead of naming the blocker's column, and the live-agent confirm omits "The agent keeps running — stopping is ⌘." | unspecified | low | BOARD-23 · BOARD-27 | `BoardView.tsx:960-988`; handoff v3 README §8c | Fill both slots from column config (#68 owns the final wording) |
| GAP-122 | The step-done **artifact hint** (`↵ read <artifact> · drag on`) can never render: `stepArtifact()` reads a field no column carries and always returns null | unreachable | low | BOARD-20 | `lib/columnConfig.ts:82-85`; DESIGN.md "Pipeline board" | Ship with the dossier work (#75) — add `step_artifact` to `board_columns` — or drop the hint from the spec until then |
| GAP-123 | The engine's **reattach path** (same-column re-entry) is unreachable from the board: a same-column drag short-circuits to reorder and ⇧h/⇧l always cross columns | unreachable | low | BOARD-30 | `BoardView.tsx:441-445, 652-660`; `step_engine.rs:590-592` | Give a focused card a "re-enter this step" key, or route card detail's "Open task" through `issue_enter_column`, so the documented reattach has a caller |
| GAP-124 | A single ⇧l (or one drop) onto a run-mode step is an **un-undoable spend with no confirmation**, and no board gesture can stop the agent it starts | missing-confirm | low | BOARD-34 · CROSS-36 · CROSS-37 | `BoardView.tsx:600-626`, `:446-458` | Accepted by ADR-0037 item 3 as the point of `run` mode — but consider a post-dispatch "esc undo" toast that stops the just-launched agent within a few seconds |
| GAP-125 | A **rework re-dispatch pastes the original packet again**, with nothing about why the card came back (review comments, PR feedback). The design never settled what a second pass tells the agent | unspecified | low | BOARD-29 | `step_engine.rs:470-479` (`step_prompt_for` rebuilds the same packet) | Settle in design: append unresolved line comments / PR review notes to the packet on re-entry into a step the card has already visited |
| GAP-126 | Dismissing a ticket-edit card replaces it with **raw JSON permanently**, with no way back short of asking the PM to re-emit the block | dead-end | low | PM-46 | `TicketEditCard.tsx:53-55, 79` | Render dismissed cards as a one-line collapsed note with a "show" toggle |
| GAP-127 | A cancelled or truncated turn leaves an **unterminated proposal fence** rendering as raw JSON prose forever, with no retry affordance and no way to delete the message | dead-end | low | PM-14 | `lib/proposal.ts:7`; `ConversationView.tsx:216-219` | Detect an unclosed `fartCode-proposal` fence and render a quiet "proposal incomplete — ask the PM to re-send" strip |
| GAP-128 | A ticket-edit whose only change is a **title identical to the current one** renders a card with a header, no fields at all, and an enabled Apply | unrepresentable-state | low | PM-45 | `TicketEditCard.tsx:103, 136` | Treat a no-op edit as invalid (raw text) or state "no changes" and disable Apply |
| GAP-129 | **Multi-block proposals and duplicate existing titles** have order-dependent / first-wins edge resolution that the design never settled and the UI never surfaces | unspecified | low | PM-17 · PM-35 | `TranscriptItems.tsx:162-164`; `issue_proposal.rs:141-147` | Decide and document: reject multi-block messages, and warn in the card when a `blockedBy` title matches more than one existing issue |
| GAP-130 | The full ~2.5 KB PM prompt is **re-sent as hidden context on every turn** and its context cost is invisible — the usage chip counts it without explaining it | unrepresentable-state | low | PM-53 | `cell.rs:686-688`; `ConversationView.tsx:206-210` | Send it once per session (or on session start) and note the hidden-context contribution in the usage chip's tooltip |
| GAP-131 | **The running agent's identity is invisible** in the common single-tab task view — the provider name lives only in the tab chip, which is hidden at one tab | unrepresentable-state | low | TASK-43 | `components/TaskHeader.tsx:57-63`; ADR-0033 §5 | Put the provider name in the header's id row (mono 11 px `#66666d`, next to the dot) when an agent terminal is live |
| GAP-132 | An **empty tab bar can render**: closing the left pane's last tab while a split is open leaves a 34 px bar with a hairline and no chips | unspecified | low | TASK-25 | `components/TaskView.tsx:58`; `store/tabs.ts:253-291` | Render the bar per-pane on `tabs.length > 0 \|\| pane === "right"`, or drop the bar for an empty pane |
| GAP-133 | The **`conversation` tab kind is unreachable** — `focusConversationTab` / `focusOrOpenTab` have had no callers since ⌘⇧A moved to the sheet; the kind survives only so persisted pre-redesign tabs pass `sanitizePane` | unreachable | low | TASK-49 | `lib/acp-conversation.ts:38-64`; `lib/tab-registry.tsx:59-67` | Delete the two helpers and note in `tab-registry.tsx` that `conversation` exists only for persisted-tab compatibility |
| GAP-134 | **Any ⌥-chord in the ⌘N composer toggles the options block** — the listener fires on the bare `Alt` keydown that precedes the combination | unspecified | low | TASK-08 | `components/Modals.tsx:162-168` | Ignore the toggle when the `Alt` keydown is followed by another key, or bind it to ⌥O / the footer button only |
| GAP-135 | A **green setup rerun does not start the agent**, and nothing says so — the code comment claims a backend gate that only exists for the creation-time waiter | unspecified | low | TASK-15 | `store/scripts.ts:123-126`; `commands/tasks.rs:84-102` | Either launch on a green rerun when the task has no agent terminal, or change the empty-state label to `setup passed · ⌘T to start` |
| GAP-136 | A **detached HEAD is reported as "no GitHub remote on this branch"** — the PR target resolves to `None` for both cases and the copy names only one | unrepresentable-state | low | SHIP-GAP-51 · SHIP-51 | `pr_target.rs:31-33`; `PullRequestPanel.tsx:132-137` | Distinguish the `branch: None` case with its own copy ("HEAD is detached — check out a branch") |
| GAP-137 | **Panel state does not survive a relaunch, and which panels should was never settled**: the Changes sheet's open state, its active tab and every dragged gutter width are in-memory only, the ⌘J drawer always reopens closed, and the PM chat always reopens *open* (re-starting an ACP session) regardless of how the user left it — while the flyout's collapsed state does persist | unspecified | low | SHIP-GAP-66 · NAV-G28 · CROSS-G3 · SHIP-04 · NAV-53 · CROSS-10 | MEMORY.md ("`changesOpen` — NOT persisted"); `ChangesSidebar.tsx:66`; `lib/useGutterResize.ts:1-4`; `store/ui.ts:100-118` | Decide per panel and persist the ones that are user intent (Changes open + tab + width, drawer, chat) under `view-state:app:*` alongside the existing diff-mode key |
| GAP-138 | Opening an archived task from ⌘K **restores it unconditionally** — opening *is* restoring, with no confirm and no undo — and a failed restore is swallowed into `.catch(() => {})`, leaving a selection pointing at a task that may not exist | missing-confirm | low | LIFE-G18 · LIFE-34 · LIFE-35 | `CommandPalette.tsx:161`; `store/sidebar.ts:118` | Surface restore as an explicit second key (`↵` opens, `⇧↵` restores) and render restore failures in the palette footer |
| GAP-139 | The delete confirm's key is **hard-coded to ⌘⌫ while its label renders the user's remapped chord** — a rebound `delete-task` shows a key that does nothing | unspecified | low | LIFE-G19 · LIFE-17 | `Modals.tsx:429` vs `:452` | Dispatch through the registry (`chordFromEvent` + the `delete-task` binding) instead of matching `Backspace` + `metaKey` |
| GAP-140 | A **project-root task's confirm renders an empty consequence list** — a destructive dialog that names no consequence, with the red delete key still armed | unspecified | low | LIFE-G20 · LIFE-10 | `Modals.tsx:352-353`, `:467-481` | Render an explicit line: `no worktree — removes the task record only` |
| GAP-141 | **Partial teardown is silent**: worktree removal failures are `tracing::warn` only, so `delete_task` returns `Ok`, the dialog closes clean, and an orphan directory stays in the pool with no in-app trace | unrepresentable-state | low | LIFE-G21 · LIFE-25 | `fartcode-core/src/tasks/deletion.rs:139-150` | Return a teardown summary from `delete_task` and let the confirm report "task deleted; worktree could not be removed" |
| GAP-142 | The delete confirm is a **fixed 420 px card on a padding-less, `max-width`-less backdrop**, so on a narrow window it overflows both edges and the `⌘⌫ delete` button can sit off-screen | dead-end | low | LIFE-G22 · LIFE-48 | `app-frontend/src/styles/modals.css:19-21`; `styles.css:730-738` | Add `max-width: calc(100vw - 32px)` to `.fc-overlay-card` and padding to `.modal-backdrop` |
| GAP-143 | Queries under **3 characters silently return no FTS hits** (trigram tokeniser) with no "keep typing" affordance — the palette just says "No matches" | dead-end | low | NAV-G06 · NAV-16 | `search.rs:98-106`; `CommandPalette.tsx:213` | Show "type 3+ characters to search" while `q.length < 3` |
| GAP-144 | **`search::update_title` has no caller** — nothing keeps indexed titles current, so a renamed task is still findable only by its original name | unreachable | low | NAV-G08 · NAV-19 | `search.rs:68-83`; grep finds the definition only | Wire it to whatever rename/retitle path lands (issue title edits already exist on the card) |
| GAP-145 | Palette results are **hard-capped at 8** with no paging, grouping or type filter, and the cap is invisible | unspecified | low | NAV-G09 · NAV-20 | `CommandPalette.tsx:84` (`apiSearch(q, 8)`) | Raise the cap, group hits by type, and show "showing 8 of N" |
| GAP-146 | First-run frame 3a's **"Open a folder ⌘O" / "Clone from GitHub ⌘⇧O" rows are not built**, and ⌘⇧O is instead bound to `open-omp` | no-affordance | low | NAV-G13 · NAV-02 | left-nav README first-run 3a; `commands.ts:347-361` | Either build the 3a rows on different chords, or record 3a as superseded by the onboarding card |
| GAP-147 | **⌘⇧. is a silent no-op while the resource-monitor setting is disabled** (the default), and only a palette-only command can enable it — nothing hints that a second command controls it | dead-end | low | NAV-G14 · NAV-25 | `commands.ts:152-162`; `ResourceMonitor.tsx:44`; `CommandPalette.tsx:116-130` | Have `toggle-right-panel` enable the setting on first use, or show a one-line "enable in ⌘K" state |
| GAP-148 | **No way to unbind a command and no way to give one two chords** — the editor always writes exactly one, and `↺` (restore defaults) is the only escape | no-affordance | low | NAV-G20 · NAV-39 | `SettingsModal.tsx:73`; `useCommands.ts:87-102`; `registry.ts:171` | Add an "unbind" action (persist `[]`) and an "add chord" affordance |
| GAP-149 | **"clear custom bindings" wipes every remap with no confirm and no undo**, taking effect immediately | missing-confirm | low | NAV-G21 · NAV-40 | `SettingsModal.tsx:144-150`; `useCommands.ts:104-109` | Add the standard inline `fc-confirm` used elsewhere |
| GAP-150 | **No `?` keyboard-shortcut sheet** (left-nav frame 4h) — no `?` handler exists anywhere in the frontend. ⌘K and Settings → Keys carry the information, so this is a discovery gap rather than a dead end | no-affordance | low | NAV-G32 · NAV-61 | left-nav README:168; no `?` handler in `app-frontend/src` | Either build it (it can be generated from `listBindings()` verbatim) or record 4h as superseded |
| GAP-151 | Flyout collapse is the **only layout state kept in `localStorage`** instead of the backend view-state KV — it survives a relaunch but not a webview data reset, and it is invisible to `prune_orphans` and to any future sync or export of view state | unspecified | low | CROSS-G1 · CROSS-04 | `app-frontend/src/store/ui.ts:84-98` (`fc:sidebarVisible`) vs `store/sidebar.ts:42` (`view-state:app:sidebar`) | Move it into `view-state:app:sidebar` alongside `collapsed` |
| GAP-152 | A **locked or denied keyring is indistinguishable from "no token"** — the status probe's rejection is swallowed, so the user is told to connect GitHub rather than to unlock their keyring | unrepresentable-state | low | CROSS-G9 · CROSS-24 | `PullRequestPanel.tsx:435` (`.catch(() => {})`); `fartcode-core/src/github/token.rs` | Distinguish `CredentialStore` errors from `Ok(None)` in the gate copy |
| GAP-153 | **Multi-window is unspecified and unreachable**: no command, no menu item, no API; the single-instance plugin actively refuses a second process and all view state is global to one webview | unreachable | low | CROSS-G13 · CROSS-35 | `fartcode-app/src/lib.rs:40-49`; `store/ui.ts` (global flags) | Decide explicitly: declare single-window a product rule in DESIGN.md, or scope view state per window before it is attempted |

---

## Recommended test harness

`make test` today is `vitest run` (app-frontend, with React Testing Library) + `cargo test --workspace`.
That covers two of the ten automation classes the catalogue actually asks for. What follows is what
each class needs, and — the useful half — how much of the catalogue can be driven **without any UI
driver at all**.

### What exists today

| Class | Tooling | State |
|---|---|---|
| **A · Pure unit** — `columnConfigSummary`, `parseDiffTabId`, `visibleTaskOrder`, `fuzzyScore`, `buildPmPrompt`, `chordFromEvent`/`dispatchKey`, `resetToDefaults`+`applyUserOverrides` | vitest | Ready. Some helpers (`fuzzyScore`) need exporting first. |
| **B · Component / store (jsdom)** — every `RTL` line in the catalogue: rendering, key dispatch, store transitions, error nodes, disabled matrices | vitest + RTL | Ready. This is the single biggest win available with zero new infrastructure. |
| **C · Rust unit + integration** — `create_local`, `column_list`, `issue_enter_column`, `step_confirm`, `settle_issues_for_task`, `share_with_team`, `prune_orphans`, `delete_task`, `parse_proposal` | `cargo test` with `FARTCODE_DB_FILE` at a temp path | Ready, and already the strongest layer (`task_deletion_integration.rs`, `dispatch_integration.rs`, `agent_terminals_integration.rs`, `lifecycle_terminals_integration.rs`, `step_engine.rs` tests). |
| **D · Git fixtures** — stage/unstage/discard, unborn HEAD, renames, ahead/behind, pre-commit hook failure, push-sets-upstream, binary/oversized diffs | `cargo test` over temp repos + a bare sibling remote | Ready (`stage.rs`, `commit.rs`, `remote.rs` tests exist). |
| **E · Static / grep assertions** — "no caller", "single call site", "no command matches /archive/", "no `@media` outside `prefers-reduced-motion`", "no merge command registered" | a small `cargo test` or vitest file that shells out to `rg` | Trivial to add; decisive for every `unreachable` scenario. |

### What is missing

| Class | Needed for | What it takes |
|---|---|---|
| **F · Tauri command surface** | Asserting the `invoke_handler` registration list (GAP-05, GAP-06, GAP-29, GAP-114), and driving commands end-to-end with events | `tauri::test::mock_app()` harness in `fartcode-app/tests` (already used in one place — generalise it into a fixture module). No new dependency. |
| **G · Fixture ACP adapter** | PM chat: permission prompts, `session/load` on and off, hidden-context suppression, cancelled turns, transcript snapshots (PM-08, PM-12, PM-50, CROSS-47) | A tiny stub binary implementing the ACP handshake, put on `PATH` by the test. ~200 lines, no third-party service. |
| **H · Fixture GitHub API** | PR sync: 401, 403 + `x-ratelimit-reset`, empty `state=open` list after a merge, idempotent upsert, offline backoff (GAP-30, GAP-31, CROSS-21, CROSS-22) | An injectable API base (the client already takes one) plus `wiremock`. |
| **I · tmux-dependent** | Slot reuse, survivor reattach, close-kills-session, delete sweeps sessions (TASK-36, TASK-37, LIFE-19, CROSS-07) | A `tmux` binary in CI and a `#[ignore]`-by-default gate so local runs without tmux still pass. |
| **J · Real webview driver** | **Everything the catalogue marks "needs a driver we lack"**: xterm focus and resize, CodeMirror editing/⌘S/dirty state, text-selection → `Ask PM` / `+ comment`, drag-and-drop physics and the 1 px insertion line, every layout and narrow-width scenario, window drag regions, double-launch focus | See below — this is the real gap. |

### The webview driver, plainly

There is **no end-to-end driver for the Tauri app today**, and `tauri-driver` will not close the
gap on this machine: it supports WebKitWebDriver (Linux) and Edge Driver (Windows), and **has no
macOS support**. Two honest options, and they are complementary rather than alternatives:

1. **`tauri-driver` + WebdriverIO in Linux CI.** Gives a real packaged app: real window sizes, real
   `data-tauri-drag-region`, real single-instance behaviour, real PTYs. This is the only way to
   drive NAV-55 (drag region), NAV-51/52 and GAP-111 (narrow layout), CROSS-14 (second launch
   focuses the window) and TASK-47. Cost: a Linux CI job, a `tauri-driver` dev-dependency, and
   accepting that the suite does not run on the developer's Mac.
2. **Playwright (Chromium) against `vite dev` with the IPC bridge mocked** via
   `@tauri-apps/api/mocks`'s `mockIPC`. Gives real layout, real xterm, real CodeMirror, real
   selections and real drag events — enough for SHIP-27/28/29/33/34, PM-36/37, BOARD-10/12,
   GAP-26, GAP-28, GAP-95 — without a packaged app, and it runs on macOS. It cannot assert
   anything about the Tauri window itself.

Recommendation: build **(2) first** — it unlocks the largest block of currently-undrivable
scenarios at the lowest cost — and add **(1)** as a small Linux smoke suite covering only the
window-level scenarios that (2) structurally cannot reach.

### What needs no UI driver at all

Roughly **half the catalogue is drivable today** from `cargo test` + `vitest` alone. The backend
half of most scenarios is a command call plus an event assertion, and the frontend half of most
*failure* scenarios is "assert nothing rendered". Concretely, these clusters need no new tooling:

- **Seeding and provenance** — FIRST-21/22/24/42/44/46/47/60/62 (`create_local` → `column_list`,
  `.git/info/exclude`, `share_with_team`, `shareable_provenance`, `prune_orphans`).
- **The whole step engine** — BOARD-13/14/16/17/18/24/29/40/42/44/47/48, CROSS-11/12/28/29/30/31/34:
  `issue_enter_column` / `step_confirm` / `settle_issues_for_task` with `EnterOutcome` and
  `step:*` event assertions. This is the highest-value, lowest-cost suite in the document, and it
  falsifies GAP-09 and GAP-42 directly.
- **Task lifecycle** — LIFE-19/20/21/22/26/27/45, TASK-11/12/28/32/38/40 via `delete_task`,
  `terminal_open_agent`, `terminal_list_for_task` and the PTY pump.
- **Git and the ship loop** — SHIP-12/13/14/15/17/26/40/41/42/44/46/47/63 over temp repos.
- **Proposals** — PM-16/21/22/27/29/30/34/35/42 through `issue_parse_proposal` /
  `issue_apply_proposal` / `issue_update` (GAP-16's cross-project write is a two-project fixture
  and three lines of assertion).
- **Every `unreachable` scenario** — FIRST-11/16/29/30/35/39/55, BOARD-30, TASK-49, LIFE-39,
  NAV-19/46/60, SHIP-45, CROSS-46: all provable with class **E** static assertions, which also
  make the gap regress loudly if someone wires the caller and forgets the test.

The residue that genuinely needs a driver is smaller than it looks: layout and narrow-width
(GAP-111 and its eight source scenarios), the three editor surfaces (xterm, CodeMirror, text
selection), drag physics, and the two window-level scenarios (single instance, drag region).

---

## Coverage audit

An adversarial pass over the 449 scenarios above, run against four inventories the catalogue
claims to cover: the 29 registrations in `lib/commands.ts`, the 101 entries in the
`invoke_handler` of `fartcode-app/src/lib.rs`, the 23 arms of `app.rs::event_to_value`, and the
surfaces named in `DESIGN.md` + `design_handoff_v3`. Method: mechanical — every command id,
Tauri command name, camelCase wrapper and event string was matched against the scenario bodies,
then every hit was read to confirm it was a real assertion rather than a passing mention in
prose or in the [Gap register](#gap-register).

**Headline:** command coverage is genuinely complete; *capability* coverage is not. The gap is
concentrated in three places — the Settings → Accounts surface, the small recovery affordances
that rescue a dead end (add-remote, ACP cancel), and the panels that sample rather than act
(resource monitor, PTY reflow). Accessibility is absent as a modality, not merely thin.

### AUDIT-00 — The command inventory table's scenario column is stale

Before the tables below: the [Command inventory](#command-inventory-all-29-registrations-in-libcommandsts)
table at §7 cites scenario IDs from an **earlier numbering** and is off by roughly +2 for most
rows. It is not a coverage record and should not be read as one. Spot-checks:

| Row | Command | Table cites | That ID actually is | Real scenario |
|---|---|---|---|---|
| 2 | `open-settings` | NAV-30 | "App chords keep working while a terminal is focused" | NAV-56 |
| 5 | `toggle-right-panel` | NAV-23 | "Arrowing past row 8 loses the selection off-screen" | NAV-25 *(trailing clause only)* |
| 6 | `toggle-changes` | NAV-24 | "Git plumbing verbs are palette-only and unbound" | — |
| 7–10 | `git-fetch/pull/push/publish` | NAV-22, NAV-40 | "Fuzzy filter ranks prefix…", "clear custom bindings" | NAV-24 |
| 13 | `delete-task` | NAV-26 | "A palette command can stack a modal underneath the palette" | NAV-26 is unrelated; LIFE-* carries it |
| 14 | `resume-agent` | NAV-27 | "Modal open suspends task and project scopes" | NAV-48 |
| 21 | `send-context` | NAV-29 | "Esc does nothing during onboarding" | NAV-49 |
| 22–23 | `previous-task` / `next-task` | NAV-17, NAV-18 | "⌘K restores an archived task", "Board cards … invisible to ⌘K" | NAV-45, NAV-46 |
| 24 | `close-tab` | NAV-35, NAV-36 | remap scenarios | NAV-43, NAV-44 |
| 26–27 | `next-tab` / `previous-tab` | NAV-32 | "⌘⇧2 fires in the task view and opens the wrong panel" | NAV-42 |
| 28 | `jump-to-tab-1…9` | NAV-31 | "`skipInEditor` commands yield to a focused text field" | NAV-41 |
| 29 | `close-modal` | NAV-20, NAV-21 | "Palette results are capped at 8…" | NAV-28, NAV-29 |

**Fix:** regenerate the column from the scenario bodies rather than by hand. Until then the table
overstates traceability — the coverage is real, the index is not.

### Uncovered commands (`lib/commands.ts`)

All 29 registrations are exercised by at least one genuine scenario. **Zero fully uncovered.**
Two are covered only incidentally and are listed as thin rather than missing:

| Command | Chord | Coverage today | What is missing |
|---|---|---|---|
| `toggle-right-panel` | ⌘⇧. | NAV-25, as a **trailing sentence** about a different command | No scenario whose `When` is pressing ⌘⇧.; the enabled-setting path, the panel's own render and the persistence of `resourceOpen` are all unasserted |
| `git-fetch` / `git-pull` | *(unbound)* | NAV-24, folded into one collective gesture with push/publish | No per-verb outcome: fetch's ahead/behind delta, pull's fast-forward vs. conflict, and the divergent-branch case are never separated |

### Uncovered Tauri capabilities (`lib.rs` `invoke_handler`)

Of 101 registered commands, **9 user-facing capabilities have no scenario at all.** (A further 11
— `git_push`, `git_fetch`, `git_publish`, `git_unstage`, `git_stage_all`, `git_file_diff`,
`create_conversation`, `list_conversations`, `github_token_*` — are never cited by their Rust
name but *are* covered behaviourally under §5 and §3; they are not counted as gaps.)

| Capability | Reachable from | Why it is a hole |
|---|---|---|
| `add_provider_account` | `ProviderAccounts.tsx:129` (add button, both auth methods) | The entire Settings → Accounts **write** surface is unscenarioed. Only `provider_auth_status` appears (CROSS-17), and only as a thing that *isn't* consulted |
| `remove_provider_account` | `ProviderAccounts.tsx:148` (remove button) | No scenario; no confirm is asserted for a destructive credential removal |
| `set_default_provider_account` | `ProviderAccounts.tsx:158` (make-default button) | Nothing asserts which account `resume_agent`'s default-agent fallback (`commands.ts:70-72`) actually resolves to |
| `provider_auth_login` | `ProviderAccounts.tsx:115` (sign-in button → login terminal) | The one affordance that fixes CROSS-17's dead end has no scenario of its own |
| `git_add_remote` | `GitFooter.tsx:24,64,73` (name + URL inputs) | SHIP-41 asserts the **disabled** "no remote" state and stops. The recovery flow that exits it is never driven — the catalogue documents the dead end but not the door |
| `acp_cancel` | `ConversationView.tsx:217` | NAV-49 establishes ⌘. is TUI-only; nothing then covers how an ACP turn *is* cancelled. The only interrupt path for the structured-chat runtime is unasserted |
| `resource_sample` + `get_resource_monitor_enabled` | `ResourceMonitor.tsx:30,37` | NAV-25 covers the *toggle*. The panel's content (1s interval, CPU %, mem bar arithmetic), its teardown, and a failing sample are all uncovered |
| `terminal_resize` | `TerminalView.tsx:31` | Fires on every pane/window resize with `.catch(() => {})`. No scenario asserts PTY reflow, and the swallowed error is not in the gap register |
| `issue_link` / `issue_unlink` | `CardDetail.tsx:448,473` (blocked-by edge editor) | 15 scenarios assert **derived** blocked state; none creates or removes an edge. The cycle case (A blocks B blocks A) is untested in either direction |
| `project_github_url` | **nothing** | Registered in `lib.rs:160`, implemented at `commands/git.rs:234`, and has **no caller in `app-frontend/src` or anywhere in Rust**. A textbook `unreachable` — and unlike the 20 catalogued ones, it is not in the register. It is also the exact primitive GAP-86 recommends building ("ship at minimum an *open on GitHub* action"), already sitting there unused |

### Uncovered events (`app.rs::event_to_value`)

Of 23 forwarded arms, **6 have no scenario asserting their observable outcome:**

| Event | Consumer in `app-frontend/src` | Status |
|---|---|---|
| `task:created` | `store/sidebar.ts:256,264` (refetches the project's task list) | Consumed, never asserted — the flyout's live insertion on a background task creation is untested |
| `task:status_changed` | `BoardView.tsx:243`, `CardDetail.tsx:110` | Consumed, never asserted — the board's live status repaint has no scenario |
| `issue:deleted` | `BoardView.tsx:236`, `CardDetail.tsx:100-103` (closes the open detail) | Consumed, never asserted — including the "detail sheet closes under you" case, which is a real concurrency edge |
| `conversation:created` | **none** | Dead channel: emitted and forwarded, no subscriber |
| `conversation:renamed` | **none** | Dead channel — `conversations/mod.rs:379` emits it, nothing listens; a rename never repaints |
| `conversation:deleted` | **none** | Dead channel — `conversations/mod.rs:352` emits it, nothing listens; a deleted conversation stays on screen |

### Surfaces with no scenarios

Checked against `DESIGN.md` §Components / §Pipeline board and the `design_handoff_v3` frames
8a–8h. Frames 8a, 8b, 8c, 8d, 8e, 8g and 8h all have scenarios. Two surfaces do not:

- **§8f · Card-detail dossier section.** Fifteen `dossier` mentions, all from the §8e consent
  block (CROSS-50…54) or the register. The frame's own content — the dossier section rendering
  inside card detail, its empty state before the first write, the `dossier_path` link — has no
  scenario except as a null-safety aside inside CROSS-54.
- **Settings → Accounts (`ProviderAccounts.tsx`).** Named once in prose (FIRST-38, as evidence
  that the App pane renders *only* `<AgentsList />` and `<ProviderAccounts />`) and never driven.
  It is the second-largest settings surface in the app and has zero scenarios.

### Missing modalities

The catalogue is strong on restart (57), narrow layout (33), concurrency (14), consent (19) and
empty states (15) — those modalities are structurally present, usually as a named subsection per
journey area. Four are missing or vestigial:

| Modality | Evidence | Assessment |
|---|---|---|
| **Accessibility / assistive technology** | 0 occurrences of "accessib", "screen reader", "focus trap", "tab order"; `aria-` and `role=` appear 7× each and **every one is an RTL query selector, not an assertion about assistive behaviour** | **Absent.** No scenario asserts that a modal traps focus, that Esc returns focus to the invoker, that async state changes (agent running → needs-you, commit → pushed) are announced, or that the status-dot vocabulary — which `DESIGN.md` calls "the whole state vocabulary" — has a non-colour equivalent. NAV-54 (tab order through rail + flyout) and SHIP-20 (keyboard-only staging) are the only two, and both are about reachability, not AT |
| **Keyboard-only operation as a whole-app property** | 3 "keyboard-only" hits: SHIP-20, PM-24, GAP-71 | **Vestigial.** Two isolated findings, no traversal scenario. The app is "key-first" by design (`DESIGN.md` §Components) — the inverse claim, that every mouse affordance has a key path *and vice versa*, is never tested end to end |
| **Internationalisation / text robustness** | 0 "i18n", 0 "localiz/localis", 0 "long title", 0 "timezone"; 6 "truncat", 1 "ellipsis" | **Absent.** No scenario uses a long or RTL string. Card titles, project names, branch names and PR titles all flow into fixed-width chrome (56px rail, 244px flyout, board columns) with no overflow scenario |
| **Reduced motion / visual preference** | 5 "reduced-motion", 3 "contrast" | **Thin.** The pulsing running dot is the app's primary liveness signal; nothing asserts its `prefers-reduced-motion` fallback still communicates "running" |

Two further modality notes, both narrower than the four above: **schema migration on restart** is
covered (12 "migration" hits) but only forward — no scenario opens a *newer* DB with an older
binary; and **undo** appears 22× exclusively as an absence (there is no undo anywhere in the app),
which is honest but means no scenario ever exercises a recovery path after a destructive gesture.

---

### Additional scenarios to close the biggest holes

Ten scenarios, in the document's format, ordered by the size of the hole they close. IDs use the
`AUDIT-` prefix to avoid colliding with the eight existing journey-area sequences; they should be
renumbered into their home areas (§1, §5, §7, §8) when written.

#### AUDIT-01 — Add, default, and remove a provider account
- **Given:** Settings → App with `ProviderAccounts` rendered and no accounts stored for `claude`.
- **When:** the user adds an account through each of the two auth methods, presses **make default** on the second, then presses **remove** on the first.
- **Then:** `add_provider_account` persists each account and the list re-renders with both; `set_default_provider_account` moves the default marker and `resume-agent`'s fallback (`commands.ts:70-72`, `deps.find(d => d.isDefault)`) subsequently resolves to the second account rather than the hardcoded `"claude"`; `remove_provider_account` deletes the first **with no confirm**, which is a destructive credential action taken on a single click.
- **Covers:** E3-07 provider accounts; `ProviderAccounts.tsx:115-160`; `commands/provider_accounts.rs`.
- **Automation:** backend round-trip through the four `provider_account` commands against a temp DB, asserting the default flag; UI is RTL with `tauri.ts` mocked. No new tooling.
- **Status:** not-built (scenario missing; the code exists and works)

#### AUDIT-02 — Signing in to a provider from Settings clears the CROSS-17 dead end
- **Given:** the `claude` CLI installed but logged out — exactly CROSS-17's precondition.
- **When:** the user presses **sign in** on the provider row.
- **Then:** `provider_auth_login(providerId, null, rows, cols)` spawns a login terminal, the user completes the CLI's own OAuth flow, and on success `provider_auth_status` flips the row to authenticated. **Intended:** the board's dispatch path re-probes and stops burning cards against a logged-out CLI. **Actual:** nothing re-probes — CROSS-17 and GAP-104 stand, and the only thing this scenario proves is that the fix exists but is not wired to the spend path.
- **Covers:** `ProviderAccounts.tsx:115`; `commands/provider_accounts.rs::provider_auth_login`; closes the recovery half of CROSS-17 / GAP-104.
- **Automation:** backend: assert the login terminal spawns and `provider_auth_status` transitions. The OAuth round trip itself needs a driver we lack; the terminal spawn does not.
- **Status:** not-built

#### AUDIT-03 — Adding a remote exits the "no remote" dead end
- **Given:** SHIP-41's fourth precondition — a workspace whose repo has no remotes at all, so push and publish are disabled.
- **When:** the user types a remote name and URL into the git footer and presses ↵ (or the button at `GitFooter.tsx:73`).
- **Then:** `git_add_remote(workspaceId, name, url)` runs, `git_commit_state` refetches, and the push/publish rows enable with the new remote named. A malformed URL leaves the inputs populated and surfaces git's stderr rather than clearing the field.
- **Covers:** `GitFooter.tsx:24-73`; `commands/git.rs::git_add_remote`; the recovery half of SHIP-41.
- **Automation:** temp repo with no remote → `git_add_remote` → assert `git_commit_state` reports the remote and the disabled matrix clears. Pure `cargo test`.
- **Status:** not-built

#### AUDIT-04 — Cancelling an in-flight ACP turn
- **Given:** a task whose conversation resolved to the ACP runtime with a prompt streaming.
- **When:** the user presses the stop control in `ConversationView` (`:217`).
- **Then:** `acp_cancel(conversationId)` interrupts the turn, the partial assistant message is retained rather than discarded, and the composer re-enables. **Intended:** ⌘. (`stop-agent`) reaches this path too. **Actual:** ⌘. only walks `terminal_list_for_task` for a live *agent terminal* and writes `\x03` (`commands.ts:316-325`) — on an ACP conversation with no PTY it is a silent no-op, so the app's advertised interrupt chord does not interrupt the ACP runtime at all.
- **Covers:** `store/conversations.ts:132`; `commands/conversations.rs::acp_cancel`; the ACP half of NAV-49's provider split.
- **Automation:** backend `acp_start` → `acp_send_prompt` → `acp_cancel` against the mock adapter, asserting the turn terminates; the ⌘. divergence is a class-E static assertion over `commands.ts`.
- **Status:** not-built — and the ⌘. gap it exposes deserves its own register entry

#### AUDIT-05 — Modals trap focus and return it on Esc
- **Given:** the task view focused on a terminal tab; the delete-task confirm opened with ⌘⌫.
- **When:** the user presses Tab past the last control in the dialog, then Esc.
- **Then:** focus cycles back to the first control inside the dialog rather than escaping into the chrome behind the backdrop; on Esc the dialog closes and focus returns to the element that opened it. The dialog is `aria-modal` and the backdrop is inert to assistive technology.
- **Covers:** the accessibility modality this catalogue currently has zero scenarios for; `Modals.tsx`; `store/ui.ts::closeTopModal`; NAV-28's Esc ordering, from the AT side.
- **Automation:** RTL `userEvent.tab()` in a loop plus `document.activeElement` assertions. No new tooling — this is the cheapest missing modality to start closing.
- **Status:** not-built

#### AUDIT-06 — Run state is not communicated by colour alone
- **Given:** a project with one running task, one needs-you task and one failed task, rendered in the rail, the flyout and the board.
- **When:** the app is inspected with colour and animation removed (`prefers-reduced-motion: reduce`, plus a greyscale pass).
- **Then:** each state remains distinguishable — the running dot's pulse has a static fallback that is still distinct from a hollow needs-you dot, and every dot carries a text or `aria-label` equivalent. **Intended per DESIGN.md** ("Status dots are the whole state vocabulary"): the vocabulary survives without colour. **Actual (expected):** filled-amber vs hollow-amber vs red vs `#46464d` collapse to near-identical greys, and the pulse is the only signal separating running from idle.
- **Covers:** `DESIGN.md` §Components "Status dots"; the reduced-motion and contrast modalities.
- **Automation:** RTL for the label/`aria` half; the greyscale and motion-preference halves need a driver we lack (harness option 2 covers both).
- **Status:** not-built

#### AUDIT-07 — The resource monitor panel samples, renders, and tears down
- **Given:** the resource-monitor setting enabled (NAV-25's post-state) and the panel open.
- **When:** the panel stays open for three seconds, then ⌘⇧. closes it.
- **Then:** `resource_sample()` is called once per second (`ResourceMonitor.tsx:37`), the CPU figure renders as `Math.round(cpuPercent)` and the memory bar as `memUsedMb / memTotalMb` clamped to 100%; closing the panel clears the interval so no sampling continues behind a hidden panel. A rejected sample leaves the last good reading rather than blanking the panel.
- **Covers:** `ResourceMonitor.tsx:30-75`; `commands/search.rs::resource_sample`; the content half of NAV-25.
- **Automation:** RTL with fake timers and a mocked `resourceSample`; assert call count, clamped width, and that the interval is cleared on unmount.
- **Status:** not-built

#### AUDIT-08 — Resizing a pane reflows the PTY
- **Given:** a task with an agent terminal running a full-screen TUI in the left pane.
- **When:** the user drags the split gutter, then narrows the window below the ~900px breakpoint.
- **Then:** `terminal_resize(terminalId, cols, rows)` fires with the new geometry and the TUI redraws to the new width without corrupting its frame. **Note:** the call is `.catch(() => {})` at `TerminalView.tsx:31`, so a failed resize is invisible and the PTY silently keeps the stale geometry — the terminal then renders wrapped garbage with no error anywhere.
- **Covers:** `TerminalView.tsx:8,31`; `commands/terminals.rs::terminal_resize`; the terminal half of NAV-51/52's narrow-layout work.
- **Automation:** backend resize + `terminal_tail` assertion is drivable today; the xterm reflow half needs a driver we lack. The swallowed error is a class-E static assertion.
- **Status:** not-built

#### AUDIT-09 — Linking and unlinking a blocked-by edge from card detail
- **Given:** cards A and B in the same project, neither linked.
- **When:** the user adds B as a blocker of A from card detail (`:473`), then removes it (`:448`).
- **Then:** `issue_link(A, B)` persists the edge, `issue:updated` fires for both, and A's derived blocked badge appears; `issue_unlink(A, B)` reverses it and the badge clears. Linking B to A while A already blocks B is refused with a named reason rather than persisting a cycle that makes both cards permanently blocked.
- **Covers:** `CardDetail.tsx:448,473`; `commands/issues.rs::issue_link` / `issue_unlink`; the *write* side of the 15 scenarios that assert derived blocked state.
- **Automation:** pure `cargo test` — two issues, link, assert derivation, unlink, assert clearance, then attempt the cycle. Among the cheapest scenarios in this section.
- **Status:** not-built

#### AUDIT-10 — Three conversation events are emitted into a void
- **Given:** the app running with the event forwarder live.
- **When:** a conversation is created, renamed (`conversations/mod.rs:379`), or deleted (`:352`).
- **Then (intended):** the surface showing that conversation repaints — a rename updates the title in place, a delete removes it. **Actual:** `event_to_value` forwards all three as `conversation:created` / `conversation:renamed` / `conversation:deleted` and **no subscriber exists anywhere in `app-frontend/src`**; the types are declared in `tauri.ts:34-46` and never matched. A rename or delete originating outside the current view is invisible until a manual refetch. Same shape as the `project_github_url` finding: working plumbing, no consumer.
- **Covers:** `app.rs:216-226`; `fartcode-core/src/conversations/mod.rs:352,379`; the `unreachable` class.
- **Automation:** class-E static assertion (grep the frontend for the three event strings) plus a backend emission test. No new tooling.
- **Status:** unreachable
