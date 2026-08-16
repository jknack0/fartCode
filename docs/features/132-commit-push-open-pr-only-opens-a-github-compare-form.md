# #132 "Commit, push & open PR" only opens a GitHub compare form

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: bug, size:M

**Evidence:** `fartcode-git/src/commit.rs:230` returns `github_compare_url(...)`; the PR is never created.

**Impact:** the PR tab keeps reading "no open pull request" until the user submits the browser form and a sync runs, and nothing in the app says so.

**Fix:** create the PR via the GitHub API (client + token already exist) or relabel the row "Commit, push & draft PR in browser".

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_a466e361-8fbc-463f-8d30-3927bf1922a7`
- source: import · https://github.com/jknack0/fartCode/issues/132
- tracker: https://github.com/jknack0/fartCode/issues/132

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/132
- 2026-08-16 00:13 · dossier created with the worktree · Quick
- 2026-08-16 00:13 · Quick · launched · pi

## Quick — 2026-08-15

Took the issue's first option: create the PR via the GitHub API instead of relabeling the row. Split the flow in two halves along the blocking boundary: `commit::prepare_pr` (guard, publish-if-needed, remote URL, HEAD message → title/body — all git subprocesses, run via `off_main_thread`) and the new async `pr_sync::create_pr_on_github` (token → `GitHubClient::default_branch` + `create_pull_request` → seed the sync cache with `PrSyncStore::upsert` so the PR tab and PR-open guard see the PR immediately; the next sync pass fills files/commits/checks). No token degrades to the old compare-form URL, and `CreatePrOutcome.created` tells the frontend which happened — `CommitCard` refetches the PR store when `created` and still opens the URL (now the real PR page). PR title/body come from the HEAD commit (`%B`), same as `gh pr create --fill`; base is the repo's API-reported default branch. 422s surface GitHub's own `message`/`errors[].message` text inline on the card. Added `FARTCODE_PR_IN_BROWSER` as a force-compare escape hatch — it also keeps the command test hermetic on machines with a keyring token or authenticated `gh`.

- Tradeoffs: the cache-seeded DTO is a skeleton (no files/checks) until the next sync; no `pr:updated` event is emitted from the command, so the frontend refetch carries that; base branch costs one extra API call per creation.
- Rejected: relabel the row "draft PR in browser" — the client and token already existed, so the honest fix was the same size as the cop-out.
- Rejected: draft-PR flag / title-body UI — the row promises "open PR", HEAD-commit fill matches `gh pr create --fill`; add knobs when someone asks.
