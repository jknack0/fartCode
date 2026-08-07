# ADR-0009: Conversation session supervisor (E2-05)

- **Status:** Accepted
- **Date:** 2026-08-03
- **Ticket:** E2-05
- **Relates to:** E2-01 (conversation rows in the task create tx), E2-04
  (initial-conversation config), E2-06 (PTY/ACP session machinery), E2-07
  (session-id resume)

## Context

E2-05 owns the durable conversation handle: session ids, resume state, and
the local/SSH execution split. The reference (`main/core/conversations/`)
writes session ids with a guarded UPDATE, resolves resume via a 7-provider
native-session-id set, and hydrates/dehydrates conversations across restarts.

## Decision

- **`fartcode-core::conversations`** — `model.rs` (types + config + row mapper) and
  `mod.rs` (store + session logic). Wired as `DbConversationStore(db,
  event_bus)` per ARCHITECTURE §7.
- **Session-id persistence** (`set_session_id`): the reference's single
  guarded UPDATE — trimmed; empty → `Error::EmptySessionId`; 0 rows →
  `Error::ConversationNotFound`. Both variants already existed in `error.rs`.
- **Resume resolution** (`resolve_agent_session_command_args`): ported
  exactly, including branch order — the native id wins when set and
  `!= conversation.id` (even with `requireProviderSessionId=false`); that
  escape applies only when no usable native id exists. The 7-provider set
  `{amp, codex, commandcode, droid, goose, oh-my-pi, pi}` is a const tested
  verbatim.
- **Session id scheme**: `make_pty_session_id` / `parse_pty_session_id`
  (`projectId:scopeId:leafId`) ported as-is; also used by terminals (E2-06).
- **Create semantics**: PTY conversations get `session_id = conversation.id`
  at creation (reference `createConversation`); ACP stays null until the
  session establishes a provider id. `initial_queue()` derives the queued
  ACP prompts (or legacy `initialPrompt` as a single prompt) only while
  `session_id` is null.
- **Hydrate/dehydrate**: `hydrate` restores per task with the correct resume
  state — a null `session_id` is the first spawn (guard write
  `session_id = conversation.id`, idempotent against double-hydrate), any set
  id reports resume. The reference's `startSession`/`detachSession` calls are
  E2-06 hooks; Phase 0 dehydrate validates ownership + no-ops.
- **Versioned config**: `conversations.config` uses the reference's INLINE
  `{"version":"1", ...}` scheme (not the `db::versioned_json` wrapper);
  parsing is loss-tolerant and never panics.
- **Scope**: `'task'` (default) | `'project'` column already exists in the
  schema; Phase 0 rejects project-scoped creation with a typed error.
  Project-scoped conversations (`task_id NULL`) arrive Phase 1.
- **Tracking**: `agent_status` + `agent_status_seen` on the row with
  `update_agent_status` (clears seen) + `mark_seen` (reference
  `markConversationSeen`).

## Consequences

- E2-06 consumes `hydrate`/`dehydrate` (startSession/detachSession hooks),
  `make_pty_session_id`, and `resolve_agent_session_command_args` for the
  agent launch command.
- E2-07 session-id resume round-trips via `session_id` (accepted criterion 1).
- 8 unit + 11 integration tests cover all three acceptance criteria; smoke
  section 15 exercises the supervisor end-to-end.
