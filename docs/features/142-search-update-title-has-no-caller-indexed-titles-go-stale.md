# #142 `search::update_title` has no caller — indexed titles go stale

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: bug, size:S

**Evidence:** `fartcode-core/src/search.rs:69` defines it; no call sites.

**Impact:** a renamed task or issue stays findable only under its old title.

**Fix:** call it from the task/issue update paths, or drop the function.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_0bfd8af8-dbd6-4e98-9a39-68e69518420a`
- source: import · https://github.com/jknack0/fartCode/issues/142
- tracker: https://github.com/jknack0/fartCode/issues/142

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 18:14:47 · created · import · https://github.com/jknack0/fartCode/issues/142
- 2026-08-14 18:50 · dossier created with the worktree · Implement
- 2026-08-14 18:50 · Implement · launched · pi
- 2026-08-14 19:30 · column · Implement → Adversarial
- 2026-08-14 19:30 · Adversarial · launched · pi

## Implement — 2026-08-14

The fix this issue asks for is already on `main` (`e7fa4a4`, "fix(search): index task rename via update_title"): `indexer.rs` handles `InternalEvent::TaskRenamed` by calling `search::update_title`, and `the_spawned_indexer_retitles_a_renamed_task` in `fartcode-app/tests/dossier_index_integration.rs` pins it end to end. Rather than re-implement it, I verified the coverage is real by mutation: stubbing out the `update_title` call turns that test red ("the spawned subscriber never retitled the renamed task") and restoring it turns it green. An acceptance criterion with a test that cannot fail is not covered, so this check was the work.

On the issue's other half — "a renamed task **or issue**" — I traced the write paths and found nothing to fix. Issue titles are never rows in `search_index`: the only issue-derived rows are `feature` rows, whose titles are dossier section headings, written by `dossier_index` through `replace_group`/`upsert_group`, and those already re-derive on every reindex. Projects have no rename API at all (`ProjectStore` emits only `ProjectAdded`/`ProjectDeleted`), so there is no stale project title either. `TaskRenamed` is therefore the whole of the stale-title surface, and `update_title` now has its caller — the "or drop the function" branch of the issue does not apply.

Deviation from the plan: running the full suite as instructed surfaced four failures that predate this branch and are unrelated to search — `main` shipped migration `0013_closed_column` without updating the migration-count expectations. I fixed them rather than declare a red suite green. `db_integration.rs` and `migrations.rs` now derive the expected count from the embedded `_journal.json` (or compare re-init against the first count) instead of hardcoding a number, because the test's own comment records this literal having gone stale once before at 0008. The dossier-path upgrade test's rewind now deletes `created_at >= 1800000000009` instead of enumerating four timestamps: the runner rejects a missing journal entry that sits below a recorded one as tampering, so the enumerated list was guaranteed to break on the next migration. `0013` is guarded by `NOT EXISTS`, so replaying it is safe.

- Tradeoffs: I did not add issue-title rows to the search index, so ⌘K still cannot find a card by its title — that is a real gap, but it is a feature, not this bug, and inventing it here would have shipped an untested index-write path under a bug fix. Deriving migration counts from the journal costs a little indirection in exchange for a test that stops going stale.
- Rejected: dropping `update_title` — it has a caller, and the rename path genuinely cannot use `upsert`, which would blank the `project_id`/`task_id` columns the palette navigates with.
- Rejected: re-implementing the fix to satisfy a literal reading of the issue — the code and its mutation-verified test already exist; duplicating them would only add a second caller.
- Rejected: leaving the four migration tests red as "not mine" — the instruction is a green suite, and the failures are a two-line staleness, not a design question.
