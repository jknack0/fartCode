# ADR-0012: Prompt delivery strategies (E3-03)

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E3-03
- **Relates to:** E3-01 (registry), E3-04 (auto-approve), E2-06 (agent
  launch), E2-05 (session ids / resume)

## Context

Each agent CLI takes its initial prompt differently — a CLI flag, stdin, or
typed into its TUI after launch. E3-03 formalizes the delivery and builds the
launch command. The reference: `buildStandardCommand` (per-provider specs in
`packages/plugins/src/agents/impl/*/index.ts`), `wrapWithStdinPipe`, and
`keystroke-injection.ts` (inject after the TUI produces output and stays
quiet for 800ms, max wait 15s).

## Decision

- **`ade-core::pty`** — `build_command(ctx, provider) -> AgentCommand`
  (faithful `buildStandardCommand` port: defaultArgs, session/resume flags
  incl. `sessionIdOnResumeOnly` / `resumeWithoutSessionFlag` /
  `sessionIdAlways` / `newConversationFlag`, auto-approve incl.
  `omitAutoApproveOnResume`, model, extraArgs, initial prompt
  positional/flag, `deduplicateFlags`), `wrap_with_stdin_pipe`,
  `quote_shell_arg`, the spill, and `PromptInjector`.
- **`PromptDescriptor` extended** (ade-providers) with the reference spec
  fields the E3-01 extraction lacked: `newConversationFlag` (letta `--new`),
  `sessionIdAlways` (antigravity), `omitAutoApproveOnResume` (kimi),
  `initialPromptViaStdinPipe` (amp), `deduplicateFlags` (codex), plus
  keystroke `submitSequence`/`submitDelayMs` (all 5 keystroke providers use
  the `\r` default). Extraction JSONs updated + regenerated.
- **Acceptance deviation — claude**: the ticket says `-p "prompt"`; the
  reference claude spec uses `initialPromptFlag: ''` (positional). Per
  AGENTS.md the reference wins — claude launches with the prompt as the last
  positional arg (the `-p` shorthand in the ticket is not what the reference
  does).
- **Spill** (ticket addition, not in the reference): prompts ≥ 32KB spill to
  `<worktree>/.ade/prompts/<uuid>.md`, passed as `@<path>`; `build_command_with_spill`
  wires the spill into argv-strategy builds (stdin-pipe / keystroke payloads
  keep the raw prompt — no `@path` corruption) and the caller (E2-06) cleans
  the returned file on agent exit via `cleanup_spilled_prompt`. Deterministic
  and tested (100KB prompt, both argv and stdin-pipe providers).
- **Keystroke injector**: `PromptInjector` state machine over a monotonic
  clock anchored at construction (max-wait is elapsed-from-`now_ms` passed to
  `new`, so any process-lifetime clock works) — waits for first output, then
  quiet-period idle (reference QUIET_PERIOD_MS, default 800), max-wait
  fallback (15s), single-shot, skips when resuming (caller obligation), and
  reports a lost prompt on exit. The PTY owner (E2-06) pumps
  `on_data`/`on_tick`/`on_exit`. The ticket's "startup_indicator" is realized
  as the reference's output-idle heuristic (the reference has no text
  indicator).

## Consequences

- E2-06 assembles `AgentCommand` from the provider descriptor + conversation
  state and drives the injector against the real PTY.
- E3-04 adds the auto-approve task setting + trust gating on top of
  `ctx.auto_approve`.
- 15 unit tests cover the provider matrix (claude positional+resume, amp
  stdin-pipe, codex `resume --last`, letta `--new`, antigravity session
  always, kimi resume auto-approve omission), spill composition
  (argv + stdin-pipe), shell quoting, dedupe, and all injector states; smoke
  section 18 exercises claude/amp/spill.
