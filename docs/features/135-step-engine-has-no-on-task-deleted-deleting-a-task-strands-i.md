# #135 Step engine has no `on_task_deleted` — deleting a task strands its park and registry entry

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: bug, size:S

**Evidence:** `fartcode-app/src/step_engine.rs` has `on_issue_deleted` / `on_project_deleted` only; the delete confirm never mentions the linked board card, and the FK clears `linked_task_id` silently.

**Fix:** add `step_engine::on_task_deleted` clearing park + launch registry, and an `unlinks card "<title>"` row in the confirm.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_159c0155-ceec-43bb-9bf1-f480407b67a2`
- source: import · https://github.com/jknack0/fartCode/issues/135
- tracker: https://github.com/jknack0/fartCode/issues/135

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/135
- 2026-08-15 12:10 · dossier created with the worktree · Plan
- 2026-08-15 12:10 · Plan · launched · pi
- 2026-08-15 12:14 · column · Plan → Implement
- 2026-08-15 12:14 · Implement · launched · pi
- 2026-08-15 12:29 · column · Implement → Adversarial
- 2026-08-15 12:29 · Adversarial · launched · pi
- 2026-08-15 12:39 · column · Adversarial → Implement
- 2026-08-15 12:39 · Implement · launched · pi
- 2026-08-15 12:53 · column · Implement → Adversarial
- 2026-08-15 12:53 · Adversarial · launched · pi
- 2026-08-15 12:57 · column · Adversarial → Implement
- 2026-08-15 12:57 · Implement · launched · pi
- 2026-08-15 13:48 · column · Implement → Adversarial
- 2026-08-15 13:48 · Adversarial · launched · pi
- 2026-08-15 14:10 · column · Adversarial → Review

## Plan — 2026-08-15

Scope: add `step_engine::on_task_deleted` sweeping park + launch registry for issues linked to a deleted task, and an `unlinks card "<title>"` row in the task delete confirm. No schema changes; the FK (`ON DELETE SET NULL`) stays as-is.

### Acceptance criteria

The issue's Fix line, expanded into checkable criteria (the dossier carries no numbered ACs, so these are derived — each traces directly to the Fix sentence or an established project convention):

- **AC1** — Deleting a task drops the parked step of any linked issue and emits `StepQueueCleared` (mirrors `on_issue_deleted`).
- **AC2** — Deleting a task sweeps every launch-registry trace (`launches`, `consumed`, `chains`) of linked issues, including launched-but-unparked ones (mirrors `forget_project`, final round 4a).
- **AC3** — A task deletion that fails *before the row is gone* leaves park + registry untouched (fail-closed, per the #66 convention already documented on `on_lane_move_committed`).
- **AC4** — The delete-task confirm shows one `unlinks card "<title>"` row per board card whose `linkedTaskId` is the task being deleted.
- **AC5** — With no linked card, the confirm renders exactly as today (no unlinks row).

### Implementation steps (ordered, one TDD unit each)

1. **`on_task_deleted` hook** — `fartcode-app/src/step_engine.rs`. Satisfies AC1, AC2.
   Signature: `pub fn on_task_deleted(app: &App, linked_issue_ids: &[String])` — takes *pre-resolved* issue ids, because by the time the row delete has succeeded the FK has already nulled `linked_task_id`, so the hook cannot query for itself post-delete. Body: for each id, `app.steps.forget_issue(id)`, sending `cleared_event(parked)` for any dropped park — the exact `on_issue_deleted` shape, looped. Tests live in the existing `#[cfg(test)]` module (fixture, `with_task`, `set_linked_task`, `settle_issues_for_task`, `has_state`, `step_events` all already exist; mirror `deletion_sweeps_parks_and_registry`).

2. **Wire into `delete_task_blocking`** — `fartcode-app/src/commands/tasks.rs`. Satisfies AC1 end-to-end, AC3.
   Before `app.deletion.delete_task(...)`: capture `let linked: Vec<String> = app.issues.list_by_linked_task(task_id)` ids (`list_by_linked_task` already exists at `fartcode-core/src/issues/mod.rs:542`). After the `?` succeeds: `crate::step_engine::on_task_deleted(&app, &linked)`. Capture-then-sweep-after-success is what makes AC3 hold: `deletion.delete_task` only returns `Err` before/at the row delete (steps 6–7 are non-fatal warns), so `Err` ⇒ row survived ⇒ no sweep. Test harness: port the `manager()` / `acp_runtime()` / repo-fixture helpers from `commands/projects.rs` tests (`delete_project_closes_the_tasks_terminals` is the template).

3. **Confirm row** — `app-frontend/src/components/Modals.tsx` (`DeleteTaskConfirm`) + new `app-frontend/src/components/Modals.test.tsx`. Satisfies AC4, AC5.
   Add an effect alongside the existing `terminalListForTask` / `listLineComments` fetches: `issueList(projectId)` (already in `lib/tauri.ts`; `IssueDto.linkedTaskId` is already exposed), filter `linkedTaskId === taskId`, keep the titles in state, best-effort `.catch(() => {})` like its siblings. Render one `<div>unlinks card "{title}"</div>` per match inside `fc-confirm-list`, before the counts row. Test file: render `<Modals/>` with `useUi.deleteTaskTarget` set (the `ProjectSettings.test.tsx` state-seeding + `TaskHeader.test.tsx` `vi.mock("../lib/tauri")` preambles are the templates).

4. **Feature-log Implement section + clean worktree** — `docs/features/135-….md`. Housekeeping, no AC.

### Test list (one named failing test per criterion, written first)

- AC1 → `step_engine::tests::on_task_deleted_drops_linked_park_and_emits_cleared` — park a linked issue, call the hook with its id, assert `!has_state`, `StepQueueCleared` observed, `confirm_step` now errs.
- AC2 → `step_engine::tests::on_task_deleted_sweeps_launched_unparked_registry` — run-mode column + `settle_issues_for_task` to populate `consumed`, hook, assert `!has_state` and no spurious event.
- AC3 → `commands::tasks::tests::failed_delete_leaves_the_park_untouched` — park a linked issue, call `delete_task_blocking` with a bad project/task pairing that errs, assert park survives and no cleared event.
- AC1 (wiring) → `commands::tasks::tests::delete_task_unparks_the_linked_card` — full `delete_task_blocking` happy path, assert sweep + event. (Second test on AC1: the unit test proves the sweep, this proves the call site — the #66 lesson is that the wiring is where these bugs live.)
- AC4 → `Modals.test.tsx`: `delete confirm lists unlinks card "<title>" when a board card links the task`.
- AC5 → `Modals.test.tsx`: `delete confirm omits the unlinks row when no card links the task`.

### Risks, riskiest first

1. **Hook placement vs. partial failure inside `deletion.delete_task`** — if a future refactor makes a *fatal* error possible after `tasks.delete` commits, capture-then-sweep-after-success would leak the park (the original bug, rarer). Verified today: post-row steps 6–7 are non-fatal. Implementer must re-read `fartcode-core/src/tasks/deletion.rs` steps 1–7 before wiring and keep the AC3 test as the tripwire.
2. **Frontend test scaffolding** — there is no `Modals.test.tsx` today; `DeleteTaskConfirm` touches `useSidebar`, `useUi`, and four tauri calls. First render in jsdom may need more mocking than expected. Mitigation: copy the `TaskHeader.test.tsx` mock preamble wholesale; every tauri fn it needs (`terminalListForTask`, `listLineComments`, `gitCommitState`, `issueList`) must be in the `vi.mock`.
3. **`StepQueueCleared` for a card that still exists** — unlike issue deletion, the card survives; the event must dismiss any open confirm overlay without other side effects. Precedent says this is safe (`on_column_lost_queue` already clears parks for surviving cards), but the implementer should eyeball the frontend `step:queue_cleared` handler once.
4. **Multiple cards linked to one task** — `list_by_linked_task` returns a `Vec`; the issue text is singular. Planned: loop the sweep, one confirm row per card. Low risk, but tests should use the plural-safe helpers.

### Decisions

- Hook takes pre-resolved issue ids, not a `task_id`: the FK nulls `linked_task_id` before any post-delete query could run, and calling a self-querying hook *pre*-delete would violate the #66 fail-closed convention.
- Full `forget_issue` sweep (not just `drop_parked_step`): the issue text names "park + launch registry", and a dead task means a dead session — an UNDELIVERED-style entry (the `on_column_lost_queue` nuance) has nothing to reattach to.
- Confirm row is fetched client-side from the existing `issueList` — no new tauri command.

- Tradeoffs: the confirm learns about linked cards via a project-wide `issueList` fetch (a few extra rows over IPC) instead of a purpose-built `issues_by_linked_task` command; the hook signature (`&[String]`) is less obvious than `(app, task_id)` and needs its doc comment to explain why.
- Rejected: calling `on_task_deleted(app, task_id)` before the delete with an internal query — simpler signature, but a refused delete would strand the *opposite* way (park swept, task alive), breaking the #66 fail-closed convention.
- Rejected: new `issues_by_linked_task` tauri command for the confirm — `IssueDto.linkedTaskId` is already exposed and `issueList` already exists; a new command is surface area for one filter.
- Rejected: emitting a dedicated `task:unlinked` event — `StepQueueCleared` already carries everything the overlay needs, and `issue:updated` from the SET NULL path already refreshes the board.

## Implement — 2026-08-15

Executed the plan's four steps in order, strictly test-first (every failing run observed before its implementation: E0425 for the missing hook, `park swept` panic for the missing wiring, missing-text / never-called assertions for the confirm row). All five ACs have named covering tests, exactly as listed in the plan.

Deviations from the plan, all small:

- **Fixture needed a workspace row.** `TaskStore::delete` reads `tasks.workspace_id` as non-null (it captures the workspace before the row vanishes), so the step-engine-style raw task insert failed. The `commands::tasks` fixture now inserts a `project-root` workspace — that kind also skips every worktree branch, keeping the test hermetic. The plan's risk list flagged re-reading the deletion internals; this is what it caught.
- **AC3's test is green-by-construction, not red-first.** With capture-before/sweep-after-success, fail-closed holds before the wiring even exists, so `failed_delete_leaves_the_park_untouched` (SQLite `RAISE(ABORT)` trigger blocks the row delete) passed on its first run. Kept as the planned tripwire rather than contorting the code to make it fail once.
- **`list_by_linked_task` errors propagate** (`?`) instead of being swallowed: a failed capture would otherwise delete the task and silently strand the park — the exact bug being fixed, in a rarer coat. Deletion robustness ("must not make the task undeletable") loses to fail-closed here because this query only fails when the DB is broken enough that the delete would fail anyway.
- **Housekeeping commits:** `cargo fmt --all` surfaced pre-existing drift in six files I never touched (committed separately, whitespace-only, `fmt-check` now green); the app appended its own Timeline entries (committed untouched). The 11 eslint errors and one first-run vitest flake are pre-existing, in files outside this change, and unaffected by it.

- Tradeoffs: the AC3 criterion is guarded by a test that never failed first — its value is as a regression tripwire, not as a TDD driver; the tasks.rs test module duplicates ~40 lines of `manager()`/`acp_runtime()` harness from projects.rs instead of extracting a shared test util.
- Rejected: forcing AC3 red-first by wiring the sweep before the delete and then moving it — writing a known-wrong implementation just to watch a test fail proves nothing the trigger test doesn't already pin.
- Rejected: shared test-helper module for the command harnesses — two copies is below the extraction threshold, and a `#[cfg(test)]` cross-module helper crate is more surface than the duplication it removes.

## Adversarial — 2026-08-15

Hostile review of the #135 diff (`e800179..6fbea0a` + housekeeping). Every finding below was verified against the code before reporting; none were fixed — this section only finds.

### Findings, ranked

1. **[Medium] TOCTOU window between link capture and row delete** — `fartcode-app/src/commands/tasks.rs:518` (capture) and `:523` (delete) are separate DB critical sections; no transaction spans them, and `delete_task_blocking` runs off the IPC thread while dispatch (`fartcode-app/src/dispatch.rs:195`) can link cards concurrently. A card linked to the task inside the window is missed by the sweep — the original #135 strand, now confined to a rare interleave. Inversely, a card *unlinked* in the window still gets swept, dropping a park it legitimately holds. Consequence is bounded (strand ≡ pre-#135 status quo; spurious sweep ≡ a dropped confirm the user can re-trigger by re-entry), but the fail-closed story is per-call, not end-to-end.
2. **[Low] The confirm row is best-effort twice over** — `Modals.tsx:633–643`: the `issueList` fetch is async with `.catch(() => {})`, and nothing gates the delete button (or the ⌘⌫ binding) on pending fetches. On a failed or slow IPC round-trip the user confirms without ever seeing `unlinks card …`. AC4 holds only eventually and only on fetch success. Consistent with the sibling rows (terminal/comment counts degrade identically), so this is inherited design, not a regression — but the criterion as worded ("the confirm shows…") overstates what is guaranteed.
3. **[Low] Multi-linked-card sweep is untested** — the plan's own risk #4 demanded plural-safe tests; every new test uses exactly one linked card (`step_engine.rs:2600,2634`; `tasks.rs:689`; the AC4 test's second card is deliberately *un*linked). The `on_task_deleted` loop (`step_engine.rs:1479`) and the multi-row confirm render are trivially plural, but nothing pins two cleared events or two `unlinks` rows.
4. **[Low] AC3's test asserts less than AC3 claims** — `failed_delete_leaves_the_park_untouched` (`tasks.rs:717`) checks park, link, and event silence; the "registry untouched" half of the criterion is unasserted (`has_state` is private to `step_engine`, `step_engine.rs:513`). Park is a fair proxy only because the hook is all-or-nothing and called from a single site — a future second call site could regress the registry half invisibly.
5. **[Low] Latent cross-project asymmetry** — `set_linked_task` validates task *existence*, not same-project (`fartcode-core/src/issues/mod.rs:877–893`). The backend sweep is task-keyed and would cover a cross-project link; the confirm fetch is project-scoped (`issueList(projectId)`, `Modals.tsx:633`) and would silently omit such a card. Unreachable via today's only production writer (dispatch links within the issue's project), so latent, not live.
6. **[Low] AC5 test-order weakness** — `Modals.test.tsx:88–89`: the absence assertion runs right after `waitFor` observes the *call*, not after the resolved state has painted; a bug that renders the row late could slip past. In practice `mockResolvedValue` + act flushing covers it, but the test does not force settlement before asserting absence.
7. **[Info] Unbounded title in the confirm row** — `Modals.tsx:755` renders the full card title; `truncate()` (`Modals.tsx:579`) middle-ellipsizes the confirm *title* but not this row, so a long imported GitHub title stretches the §7a card. Cosmetic.

### Checked and found clean

- **No deletion path bypasses the sweep**: `commands::tasks::delete_task` is the sole task-delete command (`lib.rs:321`); `deletion.rs:132` is the only production `tasks.delete` caller; project deletion sweeps by project (`projects.rs:87`).
- **Late settles cannot resurrect state**: `settle_issues_observed` looks up by `linked_task_id` (`step_engine.rs:1119`), which the FK has nulled by sweep time — PTY exits from `terminals.close_task` (which runs *after* the sweep) no-op. The restart-contract re-park heuristic is unreachable for the dead task for the same reason.
- **`StepQueueCleared` on a surviving card** only clears the overlay (`store/steps.ts:342–345`); no destructive frontend side effects.
- **Full `forget_issue` vs the `on_column_lost_queue` undelivered-entry nuance**: not applicable — re-entry requires a user gesture, which begins a fresh epoch; no session can reattach because settles are task-keyed.
- **No injection/XSS**: the capture query is parameterized; React escapes the title text node. No dead code introduced; the hook doc and the capture-order comments match the code they describe.
- **AC1/AC2/AC4 tests fail for their criterion**: re-verified each was observed red against the exact missing behavior (E0425, `park swept` panic, missing-text) before its implementation existed.

Decision: findings recorded, none fixed — the brief is find-only. Severity ranking puts the capture-window race first because it is the only finding that can reproduce the issue's original symptom.

- Tradeoffs: finding 1 is reported without a repro test — constructing the interleave deterministically needs a DB-mutex hook the codebase doesn't have, so it rests on verified structure (no spanning transaction) rather than an observed failure.
- Rejected: filing the eslint errors and vitest cold-start flake as findings — re-verified they predate the diff and touch none of its files; a hostile review of *this diff* does not get to claim the neighborhood.

## Implement — 2026-08-15 (second pass)

The plan's four steps were completed and committed in the first Implement pass, so this pass took the Adversarial section's ranked findings as its work items, in severity order, test-first wherever a red state was expressible.

- **F1 (TOCTOU) — fixed structurally.** `TaskStore::delete` now returns the ids of cards whose `linked_task_id` it FK-nulled, captured by a `SELECT` inside the same transaction as the row `DELETE` (`fartcode-core/src/tasks/mod.rs`); `deletion.delete_task` threads the ids through, and `delete_task_blocking` sweeps from the returned value — the racy pre-capture is gone, in both directions (a link written during the delete now blocks on the connection mutex and then fails `task_exists`; an unlink during the delete is serialized the same way). Red state: type mismatch (`()` has no `sort`) in `delete_returns_the_card_ids_it_unlinked`. The trait-signature change is contained: one impl, one production caller. This also deletes the pre-capture's separate failure mode flagged as the Info note.
- **F3 (plural) — pinned.** `delete_task_sweeps_every_linked_card` (two parked cards, two cleared events) and a second `unlinks` row in the AC4 frontend test. Green-by-construction — they are pins, written *before* the F1 refactor so the sweep's behavior was locked while its data source changed underneath.
- **F4 (registry assertion) — fixed.** `has_state` is now `#[cfg(test)] pub(crate)`; `failed_delete_leaves_the_registry_untouched` drives a *launched-but-unparked* card (registry-only trace, invisible to the park proxy) through a trigger-blocked delete. Red state: E0624 privacy error.
- **F6 (AC5 test order) — hardened.** `await act(async () => {})` flushes the resolved fetch before the absence assertion. Test refactor; no red phase applies.
- **F7 (unbounded title) — fixed red-first.** New test demands the 21+…+14 middle-ellipsis for a 50-char title; `truncate(c.title)` applied.
- **F2 (confirm is best-effort) — declined.** Gating the delete button on pending fetches would change the §7a confirm's interaction contract for *all* its rows (terminal/comment counts degrade identically); that is a design decision for the confirm as a whole, not a #135 patch.
- **F5 (cross-project link asymmetry) — declined.** Unreachable via any production writer today; the right fix is a same-project check in `set_linked_task` (core policy), which belongs to its own issue rather than a drive-by here. The backend sweep already covers the hypothetical; only the confirm row would omit it.

Finish line: fartcode-core 366/0, fartcode-app 136+/0, full workspace 64/64 suite summaries ok, frontend 293/293, clippy clean, fmt clean.

- Tradeoffs: `TaskStore::delete` returning issue ids leaks board knowledge into the task-store trait — accepted because the delete transaction is the only place the pre-null link set knowably exists, and the impl's own doc already closes a sibling TOCTOU (workspace COUNT) with exactly this one-boundary technique; the F3 pins and the F6 hardening never failed first, so their value is regression cover, not TDD drive.
- Rejected: holding the DB mutex across a command-level capture-then-delete — the mutex is not reentrant and every store method self-locks, so that shape deadlocks.
- Rejected: an event-driven sweep off `InternalEvent::TaskDeleted` — by event time the FK has already nulled the links, so the consumer cannot reconstruct which cards to sweep; the ids must ride the return path.

## Adversarial — 2026-08-15 (second pass)

Hostile review of the second Implement pass (`846a9f3..44e9713`). Every finding verified against the code; nothing fixed — find-only.

### Findings, ranked

1. **[Medium] The sweep runs behind worktree teardown — a launch born in the window gets destroyed.** The row delete commits at `deletion.rs:135` (tx inside `TaskStore::delete`), but the unlinked ids only surface when `delete_task` *returns* — after steps 6–7 (`remove_worktree_if_unused`, `deletion.rs:142`: git worktree remove + prune, seconds in the worst case). `on_task_deleted` (`commands/tasks.rs:522`) therefore fires seconds after the FK-null. In that window a user's `step_confirm` on the still-parked card takes the park, sees `linked_task_id == None`, provisions a **fresh task** (`step_engine.rs:1033`) and records a launch (`:1040`) — which the late sweep then `forget_issue`s, wiping a *live* launch's registry entry (its eventual settle degrades to the registry-less heuristic and re-parks on queue columns: user confirms twice, two agents provisioned). New links cannot form in the window (`set_linked_task`'s `task_exists` fails), but a confirm does not need one. Not introduced by this pass — pass 1 had the identical latency — and F1's "race closed" claim is scoped to the *capture*; still, no test covers the window, and the ids are knowable at commit time yet surface only at return.
2. **[Low] Service-level idempotent early return untested** — `deletion.rs:114` (`Ok(Vec::new())` on double-delete) has no covering test; only the store-level twin (`tasks/mod.rs:661`) is pinned. A regression that made step 1 return an error — or skip the early return and hit the store path twice — would pass the current suite.
3. **[Low] Tautological bystander assertion** — `tasks/mod.rs:638,651`: `unlinked_card` was never linked, so asserting its `linked_task_id` is null polices nothing; over-capture is already policed by the `got == want` equality. Dead weight in a test that reads as if it proves isolation.
4. **[Info] `TaskDeleted` fires before both the worktree removal and the sweep** — unchanged ordering, but now the event and the sweep are separated by the same seconds-wide gap as finding 1; the frontend refetches tasks on a deletion whose step-engine state is still standing. Benign today (the overlay clears on the later `StepQueueCleared`) and undocumented.

### Checked and found clean

- **No silent discards of the new return value**: `deletion.rs:135` is the sole production caller of `TaskStore::delete` and threads the ids; one trait impl in the workspace; `fartcode-server` exposes no task delete.
- **The serialization claims hold**: `set_linked_task` runs check + update under one guard (`issues/mod.rs:512` `mutate`), so a link racing the delete tx either lands before the capture SELECT (swept) or fails `task_exists` after commit — the dossier's F1 wording is accurate for links.
- **Trigger test still fail-closed under the new shape**: `RAISE(ABORT)` rolls back capture *and* delete together (same tx), so `Err` ⇒ nothing captured ⇒ no sweep; the new registry assertion (`failed_delete_leaves_the_registry_untouched`) drives a registry-only trace through it.
- **Truncate math in the F7 test is exact**: `truncate(·,36)` ⇒ head `ceil(35·0.6)=21`, tail `14` — the test's `21×A + … + 14×B` matches the implementation, not a copied constant.
- **F6 flush is sufficient for these mocks**: one `mockResolvedValue` → one microtask → one `setState`; `await act(async () => {})` flushes exactly that.
- **`#[cfg(test)] pub(crate) has_state`** is unreachable from production builds; doc matches gating.
- **Plural pins assert both sides**: two cleared events (sorted equality, not `contains`) and two DOM rows plus the unlinked sibling's absence.
- **The fmt-only commit** (`4940d87`) is whitespace-only in files this feature owns.

Decision: finding 1 leads because it is the only one with user-visible wrongness (a confirm mid-delete yields a doomed launch); it predates this pass, so it is filed as residual risk with the fix shape noted (surface the ids at commit time — e.g., sweep between deletion steps 5 and 6, or return ids via an out-channel before teardown) rather than as a regression.

- Tradeoffs: finding 1 again ships without a repro test — forcing a confirm into the steps-5→6 gap deterministically needs a teardown hook the service does not expose; the report relies on verified control flow (`deletion.rs:135→142` ordering, `step_engine.rs:1033` None-link branch).
- Rejected: filing the missing `issues.linked_task_id` index as a perf finding — the capture SELECT runs once per task delete inside an already multi-second operation; measurable harm is zero.

## Implement — 2026-08-15 (third pass)

Work items: the second Adversarial section's findings (the plan's steps were completed in pass 1; pass 2 closed the first review's findings). Test-first where a red state was expressible.

- **F1 (late-sweep window) — closed.** `deletion.delete_task` now takes `on_unlinked: impl FnOnce(&[String])` and invokes it at step 5.5 — immediately after the row transaction commits, before the seconds-wide worktree teardown (steps 6–7). `delete_task_blocking` passes the `on_task_deleted` closure, so the gap in which a confirm could provision a fresh task and then have its registry entry destroyed by a stale sweep shrinks from seconds to microseconds. Red state: E0061 (method takes 3 arguments, 4 supplied) in `delete_task_hands_unlinked_ids_to_the_sweep_hook`. Fail-closed is preserved by construction: the hook sits after the `?` on the row delete, and the trigger test still pins event silence on Err.
- **F2 (untested double-delete) — pinned.** `the_sweep_hook_stays_silent_for_a_missing_task`: the idempotent early return succeeds without invoking the hook. Shared the same compile-red as F1.
- **F3 (tautological assertion) — removed.** The never-linked bystander keeps existing (renamed `_bystander`, commented: it is there for the `got == want` equality to catch over-capture) but its meaningless null assertion is gone.
- **F4 (event ordering) — documented.** The `delete_task` doc comment now states `TaskDeleted` precedes the hook by the width of a function return, not the teardown gap.

Deviations and corrections:

- **Pass 2's "one caller" claim had a blind spot.** `fartcode-core/tests/task_deletion_integration.rs` — eight `delete_task` call sites — broke on the arity change; my caller grep had covered `src/` only. Patched with no-op hooks (`|_| {}`); its six tests pass. This also corrects the pass-2 note that no service-level tests existed — they did, in `tests/`, and the finding-2 gap was real only for the *early-return* path, which none of the eight sites exercised.
- **Ordering is structurally enforced, not test-enforced.** The hook's placement before steps 6–7 is a call-site fact plus a load-bearing doc comment; a deterministic repro of the window still needs a teardown seam the service doesn't expose. The contract tests pin *that* the hook fires with the right ids and *when it must not* — not the microsecond placement.

Finish line: workspace 64/64 suite summaries ok (incl. `task_deletion_integration` 6/0), fartcode-core 366/0, fartcode-app 139/0, frontend 293/293, clippy 0 warnings, fmt clean.

- Tradeoffs: `delete_task` grows a closure parameter that most callers stub with `|_| {}` — eight test sites carry that noise so the one production site gets commit-time delivery; the F2 pin was red only via the shared compile error, not via an observed behavioral failure.
- Rejected: splitting the service into `delete_task_rows` + `cleanup_workspace` — a two-phase public API invites a future caller to skip phase 2 and leak worktrees; the closure keeps the teardown unskippable.
- Rejected: keeping the `Vec<String>` return alongside the callback — two delivery paths for the same ids is surface area waiting to disagree; the callback is the only channel now.

## Adversarial — 2026-08-15 (third pass)

Hostile review of pass 3 (`2550ef4..d9e0810`, code in `60da07e`). Every claim below verified in code; nothing fixed — find-only.

### Findings, ranked

1. **[Low] A panicking sweep hook now skips worktree teardown.** `on_unlinked(&unlinked)` (`deletion.rs:147`) runs foreign code in the middle of the service. The production hook — `on_task_deleted` → `forget_issue` → `lock()` — panics on a poisoned engine mutex (`step_engine.rs:219`, `.expect("step engine mutex poisoned")`); the unwind exits `delete_task` past steps 6–7, so the rows are gone but the worktree and branch survive, and the command surfaces as "command did not complete" (`off_main_thread` join error — no hung invoke). Before pass 3 the sweep ran after the service returned, so a sweep panic could never cancel teardown; this placement trade is new. Requires a prior panic while holding the engine lock, so improbable — but the `on_unlinked` doc (`deletion.rs:106–114`) is silent on panic expectations, and the leaked worktree is exactly the resource step 6 exists to reclaim.
2. **[Low] The contract test's comment promises ordering its assertions do not check.** `delete_task_hands_unlinked_ids_to_the_sweep_hook` (`commands/tasks.rs:851`) asserts only *which ids* the hook receives; its doc comment (`:848`) says "right after the rows commit — not seconds later behind worktree teardown", which the test cannot distinguish. The pass-3 dossier section admits the placement is structurally enforced; the test's own comment should not read as if it were pinned here.
3. **[Info] "Invoked exactly once" is untested for the zero-links case.** A future `if !unlinked.is_empty()` guard would pass the entire suite while violating the documented contract (`deletion.rs:106`). Harmless for today's only consumer (`on_task_deleted(&[])` is a no-op) — it matters only if a second consumer ever counts invocations.

### Checked and found clean

- **The "microseconds" ordering claim is true**: between tx commit and the hook sit only the broadcast send (`events.rs:352`, `let _ = tx.send(…)` — never blocks, never panics) and `lifecycle::telemetry` (`lifecycle.rs:33` — a `tracing::info!`, no I/O). The pass-2 Medium window is genuinely reduced to that.
- **Single invocation, correct gating**: one call site (`deletion.rs:147`), after the `?` on the row delete, before step 6; the idempotent early return (`:123`) skips it (tested), and every `?` before step 5 skips it (the trigger test pins event silence on Err).
- **Step-5's "returning the card ids" comment is still accurate** — it describes `TaskStore::delete`'s return, which is unchanged.
- **The eight `|_| {}` stubs in `task_deletion_integration.rs` are correct layering** — the real closure is wired and tested at the command layer (`delete_task_unparks_the_linked_card`, plural pin) through `delete_task_blocking`.
- **No dead code / stale imports** from removing the returned-ids path (clippy 0 warnings); the frontend is untouched by this pass; all pass-1 acceptance tests and pass-2 pins run unmodified and green.
- **A hook panic cannot hang the UI**: `off_main_thread` maps the join error to a plain command error (`commands/mod.rs:45–53`).

Decision: finding 1 leads despite its improbability because it is the only *regression vector* this diff introduces — the price paid for closing the pass-2 window — and it is unrecorded in the service's contract. Findings 2–3 are documentation/coverage honesty, not behavior.

- Tradeoffs: finding 1 ships without a repro test — poisoning the engine mutex deterministically requires panicking a thread while it holds the lock, which the test harness can do only by installing a panic hook mid-suite; the report rests on the verified unwind path instead.
- Rejected: filing the sub-microsecond residual window (commit → event send → tracing → hook) as a finding — it has no schedulable interleave a user action could occupy; reporting it would be theater.
