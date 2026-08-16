# #125 `"acceptance": []` silently clears every criterion

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: bug, size:S

**Evidence:** `lib/ticketEdit.ts` accepts the empty array; `TicketEditCard.tsx` presents it only as `Acceptance (0)`.

**Fix:** label the empty case explicitly ("clears all N criteria") and require a second confirm.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_ff965bd9-5eac-451c-b7a1-c24e499d5541`
- source: import · https://github.com/jknack0/fartCode/issues/125
- tracker: https://github.com/jknack0/fartCode/issues/125

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/125
- 2026-08-16 01:12 · dossier created with the worktree · Quick
- 2026-08-16 01:12 · Quick · launched · pi

## Quick — 2026-08-15

Fixed entirely in `TicketEditCard.tsx`: when the edit's `acceptance` is `[]` and the loaded issue still has criteria, the section label reads "Acceptance — clears all N criteria" (N from the live issue), and apply becomes two-step — the first click arms a "confirm clear" state with an explicit warning line, the second click actually patches. The confirm resets whenever `raw` changes. When the issue already has zero criteria (or is unknown), the label says "Acceptance — empty" and apply stays single-click, since nothing is destroyed. Added `TicketEditCard.test.tsx` (4 cases: label, two-step confirm, one-click non-empty edit, no-confirm-when-already-empty).

- Tradeoffs: N is unknown when `issueList` fails or the card can't find the issue — that path falls back to "Acceptance — empty" with no confirm gate, but apply already warns "Issue not found on this board — apply will fail" there.
- Rejected: rejecting `[]` in `parseTicketEdit` — an empty replacement is a legitimate intent; the bug was silent presentation, not the payload. Also rejected a modal confirm dialog — the card already has a keyboard-driven apply flow; re-labeling the same button keeps it one component with no new UI surface.
