# ADR-0040: Chain guard + step spend ledger — depth cap, cycle hold, project budget

**Status:** accepted (2026-08-09, issue #82). Extends ADR-0037 (the 'Cost
surface' consequence named the per-column queue setting as the only
guardrail) and feeds ADR-0038 item 7 (the ledger is the token-metrics
substrate).

## Context

ADR-0037 made `on_settle: advance` + `on_enter: run` composable and #67's
columns editor made the composition clickable. A chain of run-mode
agent-step columns launches one agent per hop with no depth cap, no
budget, no spend record, and no confirmation after the first; an
`advance_to` edit pointing backwards loops the chain indefinitely. Spend
was unbounded and invisible.

## Decision

1. **Chain guard at the ONE chaining site.** `settle_issues_observed`'s
   advance branch is the only place an automatic launch chains, so the
   guard lives there (`chain_guard`). It fires only when the advance
   target is a RUN-mode agent step — queue columns park behind the
   confirm gate (the human check), shelves and human gates are inert.
   Checks in order: **cycle** (target already run/settled for this card
   in the current chain — includes the settling column, so self/backward
   `advance_to` holds), **depth** (consecutive automatic launches ≥ cap),
   **budget** (project token budget set and the ledger's known spend has
   reached it).
2. **Hold, never kill (ADR-0037 item 11 upheld).** A tripped guard
   refuses the NEXT launch: the card stays on the settled column, a
   durable `step_ledger` hold row records the refusal
   (reason + refused target), and `StepSettled` + `StepChainHeld` are
   emitted (done-dot + the card's "held · <reason>" meta line; the card
   detail's Spend section shows the row).
3. **Chain state is memory-only, ledger is durable** — same doctrine as
   parks. `ChainState { auto_launches, visited }` per issue, reset by any
   human gesture (user entry epoch, `step_confirm`); after a restart the
   chain restarts from zero, and the durable record is the ledger.
4. **`step_ledger` (migration 0011).** One row per real launch
   (column, provider, model, `auto` = settle-chained) and one per hold.
   Token usage (`context_used`, the only usage metadata that reaches the
   process — ADR-0038 item 7's argument) is backfilled at settle onto the
   newest tokenless launch row. FK-cascaded with issue/project.
   `step_ledger_list` exposes it to the card detail.
5. **Config in base project settings** (local, never shareable — spend
   guardrails are the local user's): `step_chain_depth_cap`
   (default 3) and optional `step_budget_tokens`.
6. **Failure posture:** ledger writes/backfills log and never fail a
   launch or settle. Unreadable settings → default cap, NO invented
   budget (the cap alone still bounds spend). An unreadable ledger while
   a budget IS configured fails CLOSED (hold) — "could not check" must
   not spend.

## Consequences

- A three-run-column chain still runs — up to the cap, visibly, and once.
- Budget holds are project-wide and cumulative: the ledger only sums
  *reported* tokens, so the budget is a floor on known spend, not a
  complete meter. PTY sessions that report nothing spend past it.
- Restart forgets chain position (cap-worth of hops can rerun); the
  visible ledger keeps the history honest.
- Design gate (#82): card hold line and Spend section shipped on nearest
  existing patterns (meta line, §8f timeline rows) — frames pending.
