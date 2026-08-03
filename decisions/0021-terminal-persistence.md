# ADR-0021 — Terminal persistence + resume (E2-07)

Status: accepted (ticket E2-07)

## Context

Quit and relaunch ade; tasks, terminals, and agent sessions come back
without losing work. The restart-survival contract.

## Decision

1. **Session-id persistence on every launch**: `AgentLauncher` optionally
   holds an `Arc<dyn ConversationStore>` (`with_conversation_store`); each
   `run()` persists the resolved provider session id via the reference
   `setSessionId` semantics (single guarded UPDATE; empty/not-found are
   non-fatal warns). Fresh launches and resumes both write — the row is
   always restart-ready.
2. **Boot rehydration** (`AgentLauncher::rehydrate`): reference
   `hydrateConversation` PTY path — `is_resuming = session_id.is_some()`
   (any previously-spawned conversation resumes), `initial_prompt` is NOT
   re-sent on resume, the model/auto-approve come from the stored config.
   The per-task → per-conversation boot loop is the app shell's job; the
   domain fn is testable.
3. **Tmux durability** (`ade_core::pty::tmux`): session name =
   `ade-` + base64url(sessionId) (UTF-8-safe, round-trips); shell line =
   `(has-session || new-session -d) && (mouse on) && (history-limit 100000)
   && attach` — create-if-missing so a hard kill of ade survives in the
   tmux server. Respawn is already disabled when tmux is enabled (E2-06).
   Non-tmux fallback: best-effort rehydration (documented degradation).
4. **Kill-restart acceptance** (`terminal_persistence_integration`): a fake
   `amp` (native-session-id 7-set, argv stdin-pipe, resume flag
   `threads continue`) records its args; rehydrate asserts the resume flag +
   native session id; a never-native-id conversation starts fresh. The fake
   binaries live in per-fixture temp dirs behind a PATH mutex held for the
   fixture's lifetime (parallel fixtures would otherwise race PATH and
   delete each other's binaries — caught as ENOENT flakes).

## Consequences

- `EnvPolicy::AllowlistedOnly` + allowlist env (E3-08/E2-06) apply to
  rehydrated sessions identically — resume is a normal launch with resume
  flags.
- The tmux path needs the `tmux` binary at runtime; absence falls back to
  the non-tmux rehydration path.
- Full tmux attach/kill wiring into the terminal UI lands with the
  interactive shell (E2-08+); the naming + shell-line contract is pinned
  here.
