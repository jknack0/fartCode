# #134 Task delete confirm omits the tmux session line and can report 0 terminals while sessions are alive

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: bug, size:S

**Evidence:** no `tmux` copy in `components/Modals.tsx`; the terminal count comes from the in-memory manager (`fartcode-app/src/terminals.rs`).

**Fix:** expose `terminal_list_persisted(task)` (or decode live tmux names by prefix) and itemise `kills tmux <session>`.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_f1e2419d-4d1c-4ec6-a16d-a72266b76218`
- source: import · https://github.com/jknack0/fartCode/issues/134
- tracker: https://github.com/jknack0/fartCode/issues/134

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/134
- 2026-08-15 22:06 · dossier created with the worktree · Plan
- 2026-08-15 22:06 · Plan · launched · pi
- 2026-08-15 22:12 · column · Plan → Grill
- 2026-08-15 22:12 · Grill · launched · pi
- 2026-08-15 22:25 · column · Grill → Implement
- 2026-08-15 22:25 · Implement · launched · pi
- 2026-08-15 22:36 · column · Implement → Adversarial
- 2026-08-15 22:36 · Adversarial · launched · pi
- 2026-08-15 22:52 · column · Adversarial → Implement
- 2026-08-15 22:52 · Implement · launched · pi

## Plan — 2026-08-15

**⚠️ LOUD: this dossier contains no grill section.** There are no recorded grill-session decisions or acceptance criteria — only the issue's Context block. The ACs below are derived strictly from the issue text ("expose `terminal_list_persisted(task)` … and itemise `kills tmux <session>`") and from code reading; nothing here is a guess about undocumented grill intent. If a grill session happened elsewhere, its record never landed here.

### Derived acceptance criteria

- **AC1** — the backend can enumerate a task's LIVE persisted tmux sessions (decoded ids under the `{project}:{task}:terminal:` prefix), including sessions this process does not currently show, on the tmux server where they live (local or remote, matching the teardown path).
- **AC2** — a `terminal_list_persisted(taskId)` Tauri command exposes that list; it returns `[]` when the project's tmux setting is off (no tmux probe, same gate as `terminal_surviving`), errors for an unknown task, and runs off the IPC thread (#80 — it shells out to `tmux list-sessions`).
- **AC3** — the command is registered in `generate_handler!` and `lib/tauri.ts` exports a `terminalListPersisted(taskId): Promise<string[]>` wrapper.
- **AC4** — `DeleteTaskConfirm` itemises one `kills tmux terminal <slot>` row per live session (decoded suffix, never the opaque `fartCode-<base64>` name — ADR-0026 deliberately hides it).
- **AC5** — with zero in-memory terminals but live tmux sessions (the post-restart case), the confirm still shows the kill rows — the misleading "0 terminals while sessions are alive" report is gone.
- **AC6** — with no live sessions the confirm renders exactly as today (no empty/placeholder tmux row).

### Ordered implementation steps

1. **Pure listing + manager method** — `fartcode-app/src/terminals.rs`. Extract the live-listing duplicated in `open()` and `surviving_session_count()` into a private `live_sessions(&self, task_id, prefix)` (routes via `remote_tmux_for_task`, else `fartcode_core::pty::tmux::list_tmux_sessions_by_prefix`). Add a pure `fn persisted_session_ids(live: &[TmuxSessionInfo], prefix: &str) -> Vec<String>` (filter by prefix, sort by slot number) and a `pub fn persisted_sessions(&self, project_id, task_id) -> Vec<String>` that feeds it the live listing. Satisfies **AC1**. The unit test drives ONLY the pure fn with fabricated `TmuxSessionInfo` values — never a real tmux server.
2. **Command layer** — `fartcode-app/src/commands/terminals.rs`. `terminal_list_persisted` (async, `off_main_thread`) + `terminal_list_persisted_blocking` mirroring `terminal_surviving_blocking` exactly: resolve task context (unknown task → `Err`), read project settings, tmux off → `Ok(vec![])`, else `terminals.persisted_sessions(...)`. Satisfies **AC2**.
3. **Registration** — `fartcode-app/src/lib.rs`: add `commands::terminals::terminal_list_persisted` to the `generate_handler!` list (next to `terminal_surviving`). Satisfies the backend half of **AC3**. No new events, so `tests/event_wire_contract.rs` is untouched.
4. **Frontend wrapper** — `app-frontend/src/lib/tauri.ts`: `terminalListPersisted(taskId: string): Promise<string[]>` invoking `terminal_list_persisted`, doc-commented next to `terminalSurviving`. Satisfies the frontend half of **AC3** (exercised through the component tests' mock boundary).
5. **Confirm itemisation** — `app-frontend/src/components/Modals.tsx` (`DeleteTaskConfirm`): in the existing effect, also call `terminalListPersisted(taskId)` (`.catch(() => {})`, same cancellation guard) into a `persistedSessions: string[]` state; render one row per session in `fc-confirm-list` — label `kills tmux terminal <slot>` where `<slot>` is the decoded id's trailing number, falling back to the full decoded id if the suffix doesn't parse. Keep the `deletes N terminals` count as-is (see Tradeoffs). Satisfies **AC4/AC5/AC6**.
6. **Mock sweep + full verification** — `app-frontend/src/components/Modals.test.tsx` gets `terminalListPersisted: vi.fn(() => Promise.resolve([]))` in its `lib/tauri` module mock, then the three new tests below. Run the whole frontend suite: any other test file whose `vi.mock("…/lib/tauri")` mounts a tree reaching `DeleteTaskConfirm` (candidates found by grep: `BoardView.test.tsx`, `DossierConsent.test.tsx`, `Nav.test.tsx`, `TaskHeader.test.tsx`, `lib/commands.test.ts`, `store/steps.test.ts`) will throw `TypeError: terminalListPersisted is not a function` if it omits the new export — add the stub to each failing mock. Then `cargo test -p fartcode-app` for the backend half.

### Test list — one named failing test per criterion

- **AC1** → `fartcode-app/src/terminals.rs` `tests::persisted_session_ids_filters_decodes_and_sorts_by_slot` (fabricated `TmuxSessionInfo` list containing a foreign-prefix session and out-of-order slots; expects only the task's ids, slot-ordered). Bonus adjacent test: attached AND detached sessions are both listed (delete kills both).
- **AC2** → `fartcode-app/src/commands/terminals.rs` `tests::list_persisted_is_empty_when_tmux_is_off` (in-memory `App`, tmux-off project → `Ok(vec![])`; a second test `list_persisted_errors_for_an_unknown_task` covers the error arm).
- **AC3** → **no named unit test exists for this — saying so loudly instead of inventing one.** `generate_handler!` registration is compile-time-checked and mechanically grep-verified; the TS wrapper is a one-line `invoke` shim exercised only through the AC4/AC5 component tests' mock boundary. An end-to-end IPC test harness does not exist in this repo.
- **AC4** → `app-frontend/src/components/Modals.test.tsx` `it("itemises kills tmux terminal <slot> for each live persisted session")`.
- **AC5** → `it("shows the tmux kill rows even when the in-memory terminal list is empty")` (mock `terminalListForTask` → `[]`, `terminalListPersisted` → two ids).
- **AC6** → `it("renders no tmux row when no persisted session is alive")`.

### Risks — riskiest first

1. **Frontend module-mock breakage (cross-cutting):** every `vi.mock` of `lib/tauri` that renders a modal-bearing tree must stub the new export or its tests crash with a synchronous `TypeError`. Step 6 makes the full-suite run the detector; the fix is mechanical but easy to miss a file.
2. **Main-thread stall / environment coupling:** the command shells out to `tmux list-sessions`; it MUST follow the `terminal_surviving` async-`off_main_thread` pattern (#80) and unit tests MUST never touch a live tmux server — a dev machine can have real live `fartCode-` sessions that would flip assertions. Hence the pure-fn test shape in step 1.
3. **Copy double-count:** a currently-shown tmux terminal appears in both `deletes N terminals` and a `kills tmux terminal <slot>` row. Accepted: both statements are true (the tab is deleted AND the session killed), and dedupe would require tagging `TerminalInfo` with its tmux session id — scope creep for a size:S.
4. **Remote-host parity:** listing must use `remote_tmux_for_task` (same routing as `surviving_session_count`) so the confirm reports sessions on the server where teardown will kill them; using only the local tmux would silently under-report for E12-05 remote tasks.
5. **Label parsing:** the decoded id's slot suffix should always parse (`choose_terminal_slot` mints numeric slots), but the render must fall back to the full decoded id rather than crash or show a blank row.

- Tradeoffs: kept `deletes N terminals` untouched (accepting the mild double-count above) to keep the diff size:S; the confirm gains rows only, no count re-plumbing. Session rows show the decoded `terminal <slot>` suffix, not the full `{project}:{task}:terminal:{slot}` id (redundant in a task-scoped dialog) nor the raw tmux name (ADR-0026 hides it).
- Rejected: decoding live tmux names purely in the frontend (the issue's parenthetical alternative) — the frontend has no tmux access and no base64url decode of server state; the manager already owns prefix listing and remote routing. Rejected: filtering tmux-backed terminals out of the count via a `TerminalInfo.tmuxSessionId` field — touches the DTO, every mock, and the tabs/restore stores for a cosmetic dedupe. Rejected: a DB-persisted terminal registry — the tmux server IS the durable registry; a second one can drift.

## Grill — 2026-08-15

Five load-bearing questions were put to the human; everything else fell out mechanically. **One decision here overturns the earlier Plan section:** the Plan's AC2 gated `terminal_list_persisted` on the project tmux setting — the grill rejected that gate (Q1), because the delete sweep kills by prefix regardless of the setting, so a setting-gated confirm reincarnates the exact bug being fixed (toggle tmux off after creating sessions → confirm silent, delete still kills). The Plan's step list otherwise stands; an implementer follows the ACs below where they conflict.

### Sharpened problem statement

The task delete confirm (`DeleteTaskConfirm` in `app-frontend/src/components/Modals.tsx`) itemises consequences from the in-memory `TerminalManager` only (`terminal_list_for_task`). Durable tmux sessions (`{project}:{task}:terminal:{slot}`, ADR-0025) outlive the process: after an app restart, or for sessions detached/orphaned by a crash, the manager knows nothing — the confirm reports no terminals at all while `delete_task` will sweep and kill every live session under the task's prefix (local or on the remote host, orphans included). The user consents to a delete whose most destructive consequence — killing live shells with running processes — is never shown.

### Decisions

1. **No setting gate — always probe by prefix.** The confirm's session list is produced by listing the tmux server (the one teardown will use) for live sessions under `{project_id}:{task_id}:terminal:`, regardless of the project's current tmux setting. The confirm must describe what delete DOES, not what settings currently say. Cost: one `tmux list-sessions` per confirm open (fast no-op without a server).
2. **Overlap accepted.** A tmux-backed terminal this process shows appears both in `deletes N terminals` and as its own kill row. Both statements are true (tab deleted AND session killed); the count is not re-plumbed.
3. **Label = slot suffix.** Rows read `kills tmux terminal <slot>` (decoded numeric suffix). Fallback: the full decoded session id when the suffix doesn't parse. Never the raw `fartCode-<base64url>` name (ADR-0026 hides it).
4. **Remote parity, best-effort silence.** Remote-workspace tasks probe the HOST's tmux via the same routing teardown uses (`remote_tmux_for_task`). An unreachable host renders as no rows — identical to every other probe in this dialog (`.catch(() => {})`), and the sweep itself is best-effort there.
5. **Additive only.** New command `terminal_list_persisted`; `terminal_surviving` (restore's setting-gated survivor count, ADR-0028) is untouched. Both may share the manager's internal live-listing helper.

### Acceptance criteria (write failing tests from these)

1. `TerminalManager` exposes a listing of the task's live persisted tmux sessions: decoded session ids whose id starts with `{project_id}:{task_id}:terminal:`, including sessions this process does not currently show, including ATTACHED sessions (delete kills those too), sorted ascending by slot number. Foreign-prefix and malformed (non-`parse_tmux_session_name`-decodable) names never appear.
2. For a task routed to a remote workspace, the listing queries that host's `RemoteTmux` (same routing as `surviving_session_count`/teardown); the local tmux server is not consulted for remote tasks.
3. `terminal_list_persisted(taskId)` returns the listing WITHOUT reading the project's tmux setting: with the setting off but live prefix-matching sessions present, they are still returned. Unknown task → `Err`. No tmux binary / no server / unreachable remote → `Ok([])`, never an error.
4. The command is `async` with its body in `off_main_thread` (a `_blocking` fn drives tests), per the #80 threading convention — it shells out to `tmux list-sessions`.
5. The command is registered in `generate_handler!` and `app-frontend/src/lib/tauri.ts` exports `terminalListPersisted(taskId: string): Promise<string[]>`.
6. `DeleteTaskConfirm` renders one `fc-confirm-list` row per returned id, copy `kills tmux terminal <slot>` (`<slot>` = numeric suffix after the prefix; unparsable suffix → the full decoded id), in slot order.
7. With `terminalListForTask` resolving `[]` and the persisted listing resolving ≥ 1 id, the kill rows still render — the "reports 0 terminals while sessions are alive" half of the bug is dead.
8. With the persisted listing resolving `[]`, the dialog renders exactly as today — no empty or placeholder tmux row.
9. A rejected persisted-listing promise renders no rows and no error state; the dialog still opens and delete/archive still work.
10. The `deletes N terminals` count and `terminal_surviving` behave byte-for-byte as before this change.

- Tradeoffs: accepted a per-confirm `tmux list-sessions` shell-out even for tmux-off projects; accepted the double-mention of shown tmux terminals; accepted silent under-reporting when a remote host is down (no uncertainty row).
- Rejected: gating on the project tmux setting (terminal_surviving pattern) — reincarnates the bug for sessions created before the toggle flipped. Rejected: excluding tmux-backed terminals from the count — needs a `TerminalInfo` DTO change plus mock churn for a cosmetic dedupe. Rejected: an 'unknown — host unreachable' row — new error-state plumbing out of proportion for size:S. Rejected: consolidating `terminal_surviving` onto the new listing — touches ADR-0028 restore semantics, real regression risk. Rejected: raw tmux names in copy — ADR-0026.

[fartcode:asked=human] Gate the confirm's session list on the project tmux setting, or always probe by prefix? — always probe; the confirm must match what delete actually kills
[fartcode:asked=human] How to handle a shown tmux terminal appearing in both the count and a kill row? — accept the overlap; both statements are true
[fartcode:asked=human] What identifier does each kill row display? — slot suffix (`kills tmux terminal 0`), full decoded id as fallback
[fartcode:asked=human] What does the confirm show when a remote session host is unreachable? — best-effort silence, same as every other probe in the dialog
[fartcode:asked=human] Additive command or consolidate with `terminal_surviving`? — additive only; restore semantics untouched

## Implement — 2026-08-15

Test-driven against the grill ACs, red→green each step: (1) pure `persisted_session_ids` + `TerminalManager::persisted_sessions` with the shared `live_task_sessions` extraction; (2) `terminal_list_persisted` command + `_blocking` body, registered in `generate_handler!`; (3) `terminalListPersisted` wrapper + `DeleteTaskConfirm` probe/state/rows with the `tmuxSessionLabel` slot helper. Per grill decision 1 the command reads NO settings — the Plan section's setting-gated AC2 is dead as the grill declared.

AC coverage: AC1 → `terminals::tests::persisted_session_ids_filters_decodes_and_sorts_by_slot` (near-miss prefix `t10`, attached kept, non-numeric suffix sorted last) plus the real-tmux `tests/persisted_sessions_integration.rs` (orphan sessions this process never opened, pid-unique prefix, SweepGuard). AC3 → `list_persisted_errors_for_an_unknown_task` + `list_persisted_probes_a_tmux_off_task_and_returns_ok` + the integration test's live arm. AC6–AC9 → five new `Modals.test.tsx` tests (slot rows + ordering, empty in-memory list still shows rows, non-numeric suffix falls back to the full decoded id, no sessions → no row, rejected probe → silent dialog). AC10 → full suites green with `terminal_surviving` untouched. Final gate: 65/65 cargo workspace suites, 298/298 vitest, tsc clean.

Deviations from the plan/ACs, honestly: **AC2 (remote routing) has no dedicated test** — there is no SSH/remote harness; the routing is the identical 3-line match previously shipped (equally untested) in `surviving_session_count`, now shared via `live_task_sessions` so the two paths cannot drift. **AC4 (threading)** is structural: the `off_main_thread` wrapper mirrors `terminal_surviving` byte-for-byte and tests drive the `_blocking` fn — no test asserts the async shape itself. **AC5 registration** is compile-time-checked only, as the grill declared. The command-level no-gate is proven structurally (the blocking fn contains no settings read) plus the manager-level integration test; a command-level live-session test would need to fabricate settings AND a live server for a fn that visibly cannot consult settings. The Plan's predicted mock sweep across other frontend test files was unnecessary — only `Modals.test.tsx` mounts the confirm, and the full suite confirmed it.

- Tradeoffs: the integration test self-skips where tmux is absent (Windows CI), leaving AC3's live arm covered only on tmux-capable runners; kill rows render after the async probe resolves (same pop-in as every other row in this dialog).
- Rejected: unit-testing `live_task_sessions`' remote branch via a mock `RemoteTmux` — it is a concrete struct owned by fartcode-ssh; introducing a trait seam for one 3-line match is scope creep beyond size:S. Rejected: asserting exact slot ORDER via two live tmux sessions in the hermetic unit test — the pure fn already proves ordering; the integration test proves it against the real server anyway.

## Adversarial — 2026-08-15

Assumed the diff wrong; two findings survived verification by MUTATION (apply a plausible regression, run the covering tests, watch them stay green, revert). Worktree contains no mutation residue — both were reverted via `git checkout` and the final `git status` is clean.

### Findings, ranked

1. **HIGH — grill AC3's central clause is not test-enforced at the command layer (mutation-proven).** Reintroducing the forbidden settings gate into `terminal_list_persisted_blocking` (`fartcode-app/src/commands/terminals.rs:427`, body at :438) passes ALL covering tests: `list_persisted_probes_a_tmux_off_task_and_returns_ok` cannot distinguish gate from no-gate (a gated impl also returns `Ok([])` for a tmux-off project with no live sessions), and `tests/persisted_sessions_integration.rs` targets `TerminalManager::persisted_sessions`, never the command. The test's own doc admits the gap but its NAME ("probes…") claims the very behavior it cannot detect. A future "consistency" refactor copying `terminal_surviving`'s gate ships silently. Fix shape (not applied): point the integration test at `terminal_list_persisted_blocking` with a seeded tmux-OFF project + a real live session.
2. **MEDIUM — the "slot-ordered" frontend test does not assert order (mutation-proven).** `Modals.test.tsx:121` — mutating the render to `persistedSessions.slice().reverse().map(…)` keeps 8/8 green; two independent `getByText` calls are order-blind. Grill AC6's "in slot order" is enforced only transitively (backend sort + array map); a frontend reorder regression is invisible. The test name promises more than its assertions deliver.
3. **LOW — comment overstates the remote guarantee.** `live_task_sessions`' doc (`fartcode-app/src/terminals.rs:560`) says sessions are "listed on the server where they live", but when route resolution fails, `remote_tmux_for_task` returns `None` (`terminals.rs:598`, `.ok()??`) and the listing silently falls back to the LOCAL tmux server — wrong server, empty answer. Behaviorally sanctioned by grill decision 4 (best-effort silence) and inherited unchanged from `surviving_session_count`, but the comment claims a guarantee the code does not hold.
4. **LOW — the integration test self-skips as a pass on tmux-less runners** (`tests/persisted_sessions_integration.rs:29`). On Windows/minimal CI, AC3's live arm has zero real coverage while the suite reports green. Repo precedent (`tmux_durability_integration.rs`), but the dossier's "AC3 → integration test" claim holds only on tmux-capable machines.
5. **LOW — AC9's test name claims "no error state" but never queries the error element** (`Modals.test.tsx:171`–183: asserts absent rows + dialog presence only; `fc-modal-error` is unchecked). Statically the probe's `.catch(() => {})` cannot reach `setError`, so residual risk ≈ 0 — still an assertion gap versus the name.
6. **INFO — stale rows on target switch:** `DeleteTaskConfirm` never resets `persistedSessions` when `taskId` changes while mounted; previous task's rows linger until the new probe resolves. The `cancelled` guard prevents cross-writes; pattern-identical to the pre-existing `terminalCount`/`commentCount` staleness.
7. **INFO — the rows describe delete, but the dialog also offers archive**, which kills no sessions; a user may read "kills tmux terminal 0" as applying to `a` too. Pre-existing ambiguity shared by every row ("removes the worktree"), not introduced by this diff.
8. **INFO — instant ⌘⌫ can complete before the probe resolves**, killing sessions that were never displayed. Accepted in the grill tradeoffs (async pop-in, same as every row here).

### Checked and found clean

Byte-slice safety in `persisted_session_ids` (the `starts_with` filter guarantees the `prefix.len()` boundary); duplicate-slot determinism ("0" vs "00" tie-broken by id); near-miss prefix `t1`/`t10` covered by the AC1 test; React `key={id}` uniqueness (tmux session names are unique); XSS (React escapes; ids are only local-user-forgeable and task-prefix-filtered); `invoke` camelCase arg parity with `terminal_surviving`; **no capability gating** — `capabilities/default.json` carries no per-command allowlist, so the unregistered-permission runtime failure mode does not exist here; delete-path honesty — `commands/tasks.rs:526` `close_task` sweeps the IDENTICAL prefix the confirm lists; `surviving_session_count` refactor is behavior-identical (shared helper, same match arms); mock bleed between the #135 and #134 describes is benign (each test sets its own resolved values); full pre-review gate re-confirmed (65/65 cargo suites, 298/298 vitest, tsc clean).

- Tradeoffs: findings 1–2 are reported, not fixed — the review contract is find-only; the mutation runs were reverted rather than kept as permanent mutation tests.
- Rejected: reporting the `sleep 30` in the integration test as a flake risk — the listing assertion runs ~50ms after spawn and the SweepGuard reaps regardless; a 600× margin is not a finding. Rejected: flagging `terminal_list_persisted` as an information-disclosure surface — it is task-scoped via a DB lookup and exactly as privileged as the shipped `terminal_surviving`.

## Implement — 2026-08-15 (hardening pass)

The plan's build steps had already shipped; this pass closes the Adversarial section's actionable findings, test-first. For a hardening test the honest red phase is a MUTATION red: write the test, apply the exact regression it exists to kill, watch it fail, revert, watch it pass — both loops were run and logged.

1. **Finding 1 (HIGH) closed** — `tests/persisted_sessions_integration.rs::list_persisted_command_ignores_the_tmux_setting`: seeds a tmux-OFF project (settings default, exactly what a gate reads as "skip") plus a REAL live session, and drives `terminal_list_persisted_blocking` itself. Under the reintroduced gate it fails with `left: []`; against shipped code it returns the session. The AC3 no-gate clause is now enforced at the command layer, not just the manager.
2. **Finding 2 (MEDIUM) closed** — the "slot-ordered" test now collects `getAllByText(/^kills tmux /)` text in DOM order and `toEqual`s the exact sequence; the `.reverse()` render mutation now fails it (1 failed under mutation, 8/8 after revert).
3. **Finding 5 (LOW) closed** — the rejected-probe test asserts `queryByRole("alert")` is null, so "no error state" is asserted, not implied.
4. **Finding 3 (LOW) closed** — `live_task_sessions`' doc now states the real contract: a FAILED remote route falls back to the local server (best-effort under-report per grill decision 4) instead of claiming "the server where they live".

Not addressed, deliberately: finding 4 (silent skip on tmux-less runners — repo-wide integration-test convention, not this issue's to change) and infos 6–8 (pre-existing dialog patterns the grill accepted). Final gate: 65/65 cargo workspace suites, 298/298 vitest, tsc clean, worktree clean.

- Tradeoffs: the command-level killer test depends on a real tmux server, so on tmux-less runners AC3's command-layer enforcement still rests on the hermetic `Ok([])` tests (finding 4's known ceiling); two mutation cycles cost two extra compile runs.
- Rejected: converting the mutation runs into a permanent `cargo-mutants` setup — tooling adoption is a project decision, far beyond size:S. Rejected: asserting UI order via DOM-node comparison utilities — `getAllByText` already returns document order; anything fancier restates it.





