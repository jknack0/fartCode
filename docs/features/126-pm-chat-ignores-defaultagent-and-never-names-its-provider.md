# #126 PM chat ignores `defaultAgent` and never names its provider

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: enhancement, size:S

**Evidence:** `ProjectChatPanel.tsx` has no `defaultAgent` read; it takes the first ACP-capable provider from the static registry (`fartcode-providers/src/lib.rs`).

**Impact:** no picker, no provider/model shown; the `no ACP-capable provider available` error is dead code.

**Fix:** resolve from the `defaultAgent` setting with the registry as fallback; surface it in the panel header.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_25134ad8-911a-4444-bbc2-68aaf8e59d05`
- source: import · https://github.com/jknack0/fartCode/issues/126
- tracker: https://github.com/jknack0/fartCode/issues/126

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/126
- 2026-08-16 01:12 · dossier created with the worktree · Quick
- 2026-08-16 01:12 · Quick · launched · pi

## Quick — 2026-08-16

Resolved the PM chat provider in `ProjectChatPanel.tsx`: filter `listProviders()` to ACP-capable, read `getAppSetting("defaultAgent")`, use it when it names an ACP entry, else fall back to the first ACP provider (the PM chat has no TUI path — backend `conversations.rs` rejects non-ACP providers, so a silent fallback beats a dead-end error). The resolved provider · defaultModel is prefixed onto the existing `pm-chat-scope` header span (`Claude · sonnet · project root · ⌘⇧2`) — no new CSS. The `no ACP-capable provider available` error is now reachable (empty ACP filter) and covered by a test. Resolution happens once per mount; no `setting:changed` re-resolve, since swapping the provider of the one persistent project conversation mid-session is a bigger question than this ticket.

- Tradeoffs: header can go stale until the panel remounts after a defaultAgent change; no picker (the issue only asks to resolve + surface).
- Rejected: extracting a shared resolver for `acp-conversation.ts`/`CardDetail.tsx` — those are task-scoped paths with their own semantics, out of scope for size:S.
- Rejected: erroring when defaultAgent names a non-ACP provider — the issue says "registry as fallback", and the backend already guards the hard case.
