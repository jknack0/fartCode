# ADR-0037: Configurable pipeline columns — lane semantics move from identity into data

**Status:** accepted (design review 2026-08-09, handoff v3 — supersedes
ADR-0032 items 2 and 4, which become the seeded default; item 3's "board
never kills" doctrine survives restated). Handoff erratum resolved by the
user: v3's seed line lists In Progress as `on_enter: queue`, which would
break this ADR's behavior-identical migration — the seed stays `run`; the
queue confirm remains a settings flip. Per v3, seeded Quick carries
`claude · haiku` as its step provider/model.

## Context

ADR-0032 settled five lanes (Backlog / Ready / In Progress / In Review / Done)
with behavior keyed to lane **identity strings**. A 2026-08-09 code audit found
that identity baked into ~10 sites: the closed `Lane` enum
(fartcode-core/src/issues/mod.rs:37), the board-order SQL `CASE` (mod.rs:397),
blocked derivation's literal `b.lane != 'done'` (mod.rs:216), the dispatch
trigger and flip target (fartcode-app/src/dispatch.rs:110, :135), Backlog
hardcoded in all three entry paths, the PM system prompt's lane prose
(pmPrompt.ts), the frontend `LANES`/`LANE_LABEL` arrays, `repeat(5)` CSS, and
confirm-overlay copy.

The audit also showed the board is **already a pipeline runner with exactly one
agentic step**: only In Progress (dispatch: worktree + task + one hardcoded
prompt packet) and Done (unblocks dependents) carry semantics; agent settle
auto-flips In Progress → In Review; Backlog and Ready are inert shelves. And
there is only **one live status system** — `task.status` is frozen at
`in_progress` from birth (`update_status` has zero production callers); the
issue lane is the only status that moves.

The product driver: columns should be **steps in a process**, each carrying its
own agent config — e.g. a Plan column running a grill-me system prompt on
fable at high reasoning before an Implement column executes the plan. Today's
board is the special case `[shelf, shelf, step(implement, default agent,
dispatch packet), human gate, terminal]`; this ADR generalizes it.

## Decision

1. **Per-project `board_columns` table; issues reference columns by id.**
   Each column: `name`, `position`, `kind` (`shelf` | `agent_step` |
   `human_gate`), `counts_as_done`, `is_landing`, and for `agent_step` a step
   config: system prompt, provider, model, reasoning effort, tool allowlist,
   `on_enter` (`run` | `queue`), `on_settle` (`hold` | `advance`) with an
   optional `advance_to` target column (NULL = next column). Append-only
   migration adds the table plus `issues.column_id`, backfilled from the five
   lane strings. Board order comes from `position`, replacing the SQL `CASE`.
2. **One task per card, for the card's whole life.** The first `agent_step` a
   card enters provisions the worktree + linked task (generalized dispatch);
   every later step runs as a **new agent session in the same task/worktree**
   with that column's prompt/model. Stage artifacts (plan.md, the diff)
   accumulate where the next stage needs them; `linked_task_id` stays
   singular; reattach-not-respawn and ⌘⌫ teardown stay per-task. *Rejected:*
   ephemeral headless step agents (two agent kinds, two output homes, no
   stable "in-flight" definition) and a hybrid by column kind (most concept
   surface for the least gain).
3. **Step trigger is per-column.** `on_enter: run` fires on drop;
   `on_enter: queue` shows the existing dispatch-style confirm overlay first.
   Expensive-model columns queue; cheap ones run. No global rule.
4. **Settle behavior is per-column, default hold.** `on_settle: hold` leaves
   the card with a step-done state for a human drag; `advance` moves it to
   the next column by default, or to an explicit `advance_to` target column
   when set (nullable FK; NULL = next). Today's auto-flip becomes the seeded
   In Progress column's `on_settle: advance`; seeded Quick advances with
   `advance_to = Done` — without a target, a Quick card would walk into In
   Progress and fire a second unconfirmed dispatch. Full-auto chains are
   therefore expressible but never the default — approval gates remain the
   doctrine.
5. **Mid-step interactivity is the existing needs-you state.** A step that
   asks questions surfaces the needs-you dot (card, flyout, rail); the
   conversation happens in the task's agent terminal. No second chat surface.
6. **`counts_as_done` replaces every `'done'` string test.** Blocked
   derivation, the dispatch prompt's finished-blocker summary, and any future
   terminal lane (Shipped, Won't do) key off the flag. Multiple terminal
   columns are legal.
7. **One `is_landing` column per board.** GitHub import, PM proposal apply,
   and manual add all target it. PM prompt prose and UI copy ("approve N →
   \<column\>") are generated from column config, and the prompt version bumps
   with the proposal-contract parser as ADR-0032 already requires.
8. **Migration seeds the classic five plus Quick** for existing *and* new
   projects, with dispatch/flip config attached where today's behavior
   lives. Seeded order: Backlog · Ready · Quick · In Progress · In Review ·
   Done — the two agentic drop targets adjacent, Backlog leftmost as the
   landing lane. Existing cards behave identically; the only visible change
   on an existing board is the new (empty, removable) Quick column.
9. **Columns are edited in project settings** (a Columns section beside
   scripts/provenance), where prompt/model editing has room. The board itself
   gets no editing surface in v1.
10. **Express is a place, not a flag.** Small changes skip ceremony three
    ways, all inside the model: ⌘N ad-hoc tasks remain the zero-ceremony,
    board-free path; cards may be dropped directly into any column
    (`move_to` stays permissive — no ordered-traversal requirement); and
    the seeded default includes a **Quick** column (`agent_step`,
    `on_enter: run`, `on_settle: advance` into Done) as the gateless lane
    for small-but-tracked work. *Rejected:* a per-card gate-override
    toggle — it makes gating answer to two sources of truth (column config
    × card flag) and breaks the model's spatial property that where you
    drop a card *is* the ceremony decision.
11. **Doctrine restated, not weakened.** The board never kills: no column
    move stops an agent. The flyout's in-flight contract becomes "card whose
    current column is an `agent_step` with a live session, or needs-you" —
    the type-level filtered-list requirement from the left-nav handoff is
    re-derived from column kind instead of lane names.

## Consequences

- **Design review required before build.** Keyboard h/l with N columns, the
  narrow-mode mono strip, per-column card states, confirm copy now templated
  from config, and the settings Columns editor all need frames; the five-lane
  copy in design_handoff_v2 §5g/§7 and the board onboarding text become
  template instances.
- Frontend derives columns from data: `LANES`/`LANE_LABEL`/`LANE_DOT_STATUS`
  and `repeat(5)` grids die; card run-state derives from live session state
  (the audit's TaskHeader-dot finding rides along).
- `TaskStatus` beyond `in_progress` stays dead and is now formally
  redundant — a later cleanup can delete it rather than revive it.
- Cost surface: `on_enter: run` on an expensive column is a spend trigger;
  the per-column queue setting is the guardrail, and the confirm overlay
  must always name provider/model before fire.
- The seeded template is now six columns, and the Quick column's
  confirm-free spend path needs its provider/model visible on the column
  header, not just in settings.
- **Narrow mode scrolls, never caps:** the mono strip and single-column
  view scroll horizontally, h/l walks every column, and the strip follows
  focus — the design generalizes to any column count rather than
  truncating. Frames needed, but the behavior is settled.
- Deliberately deferred (revisit post-v1, not open): per-source landing
  columns and a template picker at project creation.
