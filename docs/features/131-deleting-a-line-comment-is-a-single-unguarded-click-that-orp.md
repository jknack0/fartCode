# #131 Deleting a line comment is a single unguarded click that orphans its linked task

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: bug, size:S

**Evidence:** `components/CommentThread.tsx` — no confirm; `commands/line_comments.rs` deletes directly.

**Fix:** reuse the `fc-confirm` overlay and mention the linked task in the body.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_7b3a5edb-3818-4449-a802-ba5f113e9f3c`
- source: import · https://github.com/jknack0/fartCode/issues/131
- tracker: https://github.com/jknack0/fartCode/issues/131

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/131
- 2026-08-16 01:04 · dossier created with the worktree · Quick
- 2026-08-16 01:04 · Quick · launched · pi

## Quick — 2026-08-16

Guarded the line-comment delete in `app-frontend/src/components/CommentThread.tsx` by reusing the exported `ConfirmDelete` from `Modals.tsx` — it already renders the `fc-confirm` overlay-card with busy/error handling and ↵-confirm, so no new CSS or modal plumbing. Delete state lifted to `CommentThread` (`pendingDelete`); the row ✗ now just requests deletion. The confirm body names the linked task ("Linked task \"X\" is kept but loses the comment that spawned it"), falls back to "already deleted" when the task is gone, and "can't be undone" for plain notes. Escape peels one layer: cancels the confirm first, then closes the thread. Backend untouched — `delete_line_comment` stays direct; the guard belongs in the UI.

- Tradeoffs: `ConfirmDelete` is a full-screen `modal-backdrop` card rather than an inline `.changes-panel`-style confirm inside the thread — heavier visually, but zero new CSS and consistent with every other destructive confirm.
- Rejected: DiscardConfirm-style inline confirm (ChangesSidebar) — its styles are scoped to `.changes-panel`, so reuse would mean duplicating CSS for a size:S bug.
- Rejected: backend soft-delete / unlink handling in `line_comments.rs` — issue fix explicitly scopes to the frontend confirm.
