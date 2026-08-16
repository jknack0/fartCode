# #130 External write while a diff tab is dirty is silently ignored

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: bug, size:S

**Evidence:** `components/DiffView.tsx` — no `changed on disk` badge or external-change state.

**Impact:** the next save overwrites the agent's version with no divergence warning.

**Fix:** badge the header when a deferred refresh is pending and offer reload-vs-keep.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_b938a225-48df-44c5-ba3c-c350be657528`
- source: import · https://github.com/jknack0/fartCode/issues/130
- tracker: https://github.com/jknack0/fartCode/issues/130

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/130
- 2026-08-16 01:11 · dossier created with the worktree · Quick
- 2026-08-16 01:11 · Quick · launched · pi

## Quick — 2026-08-16

Detection lives where the silent ignore lived: DiffView's build effect already deferred rebuilds while dirty, so that branch now compares the fresh payload against a `builtPayloadRef` (the payload the mounted view was built from) and sets a store-level `externalByTab` flag only when the *disk* content genuinely moved — comparing against the editor doc would false-positive on every refresh event, since a dirty editor always differs from disk. The header badges `changed on disk` (amber, matching the dirty dot's voice) with two mono-word actions: **reload** clears dirty+external and lets the effect fall through to the normal rebuild (payload is already fresh from the deferred refresh — no refetch), **keep** re-baselines `builtPayloadRef` to the pending payload so the badge doesn't instantly re-flag, and leaves the edits; the badge returns if disk moves again. A successful ⌘S clears the flag in the store — saving while badged is the informed overwrite the issue asked for. `external` joined the build effect's deps so resolving the badge re-runs the deferral logic. Pinned the store contract in `src/store/diffs.test.ts` (4 tests).

- Tradeoffs: no three-way merge or diff-of-divergence view — reload-vs-keep is binary, and "keep" silently discards knowledge of the disk version until the next external write.
- Rejected: refetching on reload (`refresh(tabId)`) — the deferred refresh already stored the fresh payload; a rebuild from store state is enough and avoids a redundant IPC round trip.
- Rejected: a modal confirm on ⌘S while diverged — the persistent header badge warns without blocking the save flow, and the store test pins that save resolves the flag.
