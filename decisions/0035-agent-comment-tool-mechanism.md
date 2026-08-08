# ADR-0035: Agent comment tool — host-side entry point; adapter tool registration deferred

- **Status:** Accepted
- **Date:** 2026-08-07
- **Ticket:** E4-11 (#51)
- **Relates to:** ARCHITECTURE.md §14 ("Agents call `add_line_comment` tool")

## Context

§14 specifies that agents call an `add_line_comment` tool (the project agent
reviewing a sub-task's diff leaves line-anchored feedback). The ticket says the
mechanism is to be aligned with how agent-facing tools are exposed *in this
codebase*, decided in-ticket, and recorded if it deviates from §14.

This codebase talks to agents two ways: PTY terminals (byte streams) and ACP
(`fartcode-acp`). Neither path has a facility for the *host* to register a
custom tool that an agent can autonomously invoke. In ACP that would be an MCP
server surfaced to the adapter; `fartcode-acp` implements permission brokerage
and `fs/*` handling, not MCP tool serving, and `fartcode-integrations` (the
natural MCP home) is Phase 2. There is no existing "agent calls a host function"
hook to align with.

## Decision

Ship the complete host-side capability now; defer autonomous agent invocation.

1. **Validated, attributed host entry point.** `LineCommentStore::add_agent_comment`
   validates against the task's materialized workspace — path containment
   (lexical + canonical, no escape), file existence, and a non-empty in-range
   line span — with typed errors (`Error::InvalidLineComment`,
   `Error::PathEscape`). Attribution lands in `created_by` as
   `agent:<provider>`. Exposed as the `agent_add_line_comment` Tauri command.
2. **Resolution flow reuses E4-10.** Manual resolve only; a linked task
   finishing shows "→ done" via the live task-status badge without
   auto-resolving (the §14 decision, already implemented in #50).
3. **Deferred:** registering `add_line_comment` as a tool the running agent can
   call on its own (MCP server into the adapter). That needs MCP custom-tool
   infrastructure this codebase doesn't have yet.

## Consequences

- The guardrails, attribution, persistence, FK cascade, and the diff-view badge
  are real and tested today; an adapter (or a test) invoking the command gets a
  persisted, attributed comment.
- A running agent cannot *yet* invoke the tool unprompted. When MCP custom-tool
  serving lands (Phase 2), it calls into the same `add_agent_comment` — the
  validation/attribution layer is the stable contract, only the transport is
  new.
