# #129 Discarding an untracked file hard-deletes it with no undo and no trash

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: bug, size:S

**Evidence:** `fartcode-git/src/stage.rs` uses `remove_file`; no trash path anywhere in `fartcode-git` / `fartcode-core`.

**Impact:** an hour of un-added agent output is destroyed on one confirm.

**Fix:** move untracked discards to the OS trash or a `.fartcode/trash` holding pen.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_32b00e25-c175-4358-938e-918dfddb30cb`
- source: import · https://github.com/jknack0/fartCode/issues/129
- tracker: https://github.com/jknack0/fartCode/issues/129

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/129
- 2026-08-16 01:11 · dossier created with the worktree · Quick
- 2026-08-16 01:11 · Quick · launched · pi

## Quick — 2026-08-16

Moved untracked discards from `std::fs::remove_file`/`remove_dir_all` to a
holding pen: `discard` now `fs::rename`s each untracked path into
`.fartCode/trash/<epoch-millis>[-<n>]/<worktree-relative-path>` inside the
worktree. Chose `.fartCode/` (capital C) to match the existing project-internals
convention (`.fartCode.json`, the `.fartCode/` gitignore entry). The pen writes
its own `*` gitignore on creation so trashed files stay invisible to `git
status` even in repos that never ignored `.fartCode/` — verified by a test
asserting a clean status snapshot after discard. Rename is same-filesystem
(pen lives inside the worktree), so it is atomic, preserves directory trees,
and needs no copy/delete fallback or new dependency. One batch dir per
`discard` call, created lazily only when an untracked path is present;
millisecond collisions get a `-<n>` suffix via `create_dir` retry. Updated the
ChangesSidebar confirm copy from "discarding deletes it" to "it moves to
.fartCode/trash".

- Tradeoffs: no automatic undo/restore UI and no trash size cap or expiry —
  recovery is manual (copy the file back out of the pen), and the pen grows
  until the user clears it.
- Rejected: OS trash via the `trash` crate — new cross-platform dependency and
  platform quirks (headless/CI, network mounts) for a size:S bug; the in-repo
  pen is dependency-free and keeps the artifact next to the worktree that
  produced it.
