# #133 `upstream / ahead / behind / remotes` are fetched on every git event and rendered nowhere

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: enhancement, size:S

**Evidence:** `fartcode-git/src/commit.rs` DTO comment names the footer as consumer; `components/GitFooter.tsx` has no ahead/behind reference.

**Fix:** render `↑n ↓n <upstream>` in the footer hint line, or drop the fields from the DTO.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_b59a7a46-4565-4302-85cb-9ae55d464376`
- source: import · https://github.com/jknack0/fartCode/issues/133
- tracker: https://github.com/jknack0/fartCode/issues/133

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 21:59:51 · created · import · https://github.com/jknack0/fartCode/issues/133
- 2026-08-15 23:07 · dossier created with the worktree · Grill
- 2026-08-15 23:07 · Grill · launched · pi
- 2026-08-15 23:27 · column · Grill → Plan
- 2026-08-15 23:27 · Plan · launched · pi
- 2026-08-15 23:31 · column · Plan → Implement
- 2026-08-15 23:31 · Implement · launched · pi
- 2026-08-15 23:34 · column · Implement → Adversarial
- 2026-08-15 23:34 · Adversarial · launched · pi
- 2026-08-15 23:37 · column · Adversarial → Implement
- 2026-08-15 23:37 · Implement · launched · pi
- 2026-08-15 23:43 · column · Implement → Adversarial
- 2026-08-15 23:43 · Adversarial · launched · pi
- 2026-08-15 23:46 · column · Adversarial → Plan
- 2026-08-15 23:46 · Plan · launched · pi
- 2026-08-15 23:54 · column · Plan → Implement
- 2026-08-15 23:54 · Implement · launched · pi

## Grill — 2026-08-15

Sharpened problem statement: `CommitState` carries `upstream`, `ahead`, `behind`, `remotes` on every git event. The audit claim is only three-quarters true — `remotes` IS consumed (`GitFooter.tsx` gates the add-remote mini-form on `remotes.length === 0`); it is `upstream`/`ahead`/`behind` that are computed and rendered nowhere. Decision: make the fields earn their keep — render the sync segment in the footer hint line rather than dropping the fields.

Decisions:

- Direction A: render, don't drop. The data is already paid for on every git event; showing it is the cheaper end of the issue's own fork, and `remotes` couldn't be dropped anyway.
- Placement: prepend `↑n ↓n <upstream>` to the existing `fc-footer-hint` line with the existing `·` separator — `↑0 ↓0 origin/main · d discards after a confirm · fetch / pull / push in ⌘K`. One `<p>`, no new element.
- No upstream (`upstream: null`): the segment is omitted entirely; the hint line is byte-identical to today. This also covers `st === null` (state not yet loaded) and the no-remotes case where the add-remote form shows.
- Upstream present: always render counts, including `↑0 ↓0` when fully synced — literal `↑n ↓n <upstream>` per the issue, no zero-hiding cleverness.
- Staleness accepted as-is: counts reflect the last fetch / last git event. No new fetch triggers, no polling, no staleness indicator.
- A11y: bare mono text, no aria-label — consistent with the hint line's existing terse shorthand (`d discards`).
- Rust side untouched: the DTO already provides everything; this is a frontend-only change plus updating the stale consumer comment in `fartcode-git/src/commit.rs` if it needs correcting.

Acceptance criteria:

1. With `state.upstream === "origin/main"`, `ahead === 3`, `behind === 2`, `GitFooter` renders a hint line beginning `↑3 ↓2 origin/main · ` followed by the existing hint text.
2. With an upstream and `ahead === 0 && behind === 0`, the hint line begins `↑0 ↓0 origin/main · ` — zero counts are shown, not hidden.
3. With `state.upstream === null`, the hint line renders exactly `d discards after a confirm · fetch / pull / push in <paletteKey>` — no arrows, no separator residue, no placeholder text.
4. With `stateEntry` undefined (state not yet loaded), the hint line is identical to criterion 3's output and the component does not throw.
5. The `remotes.length === 0` add-remote form behavior is unchanged (existing behavior regression-guarded).
6. New `GitFooter` component tests cover criteria 1–4 and pass; typecheck/build is green.
7. No changes outside `app-frontend` except, at most, correcting the consumer comment in `fartcode-git/src/commit.rs`; no new fetch/poll logic anywhere.

- Tradeoffs: counts can be stale (they reflect the last fetch) and screen readers get raw arrow glyphs; both accepted to keep the change size:S and consistent with the hint line's existing shorthand.
- Rejected: dropping upstream/ahead/behind from the DTO — the data is already computed per git event and `remotes` must stay regardless, so rendering is the smaller net change and adds user value. Rejected: hiding zero counts or the whole segment when synced — inconsistent presence makes the line jumpy and hides the "you are synced" signal. Rejected: fetch-triggering or staleness indicators — scope blows past size:S for marginal honesty.

[fartcode:asked=human] Render `↑n ↓n <upstream>` in the footer, or drop the fields from the DTO? — A: render in footer
[fartcode:asked=human] What renders when the branch has no upstream? — nothing; hint line unchanged
[fartcode:asked=human] What renders when synced (ahead=0, behind=0) with an upstream? — full `↑0 ↓0 origin/main`
[fartcode:asked=human] Where in the footer does the segment go? — prepended to the existing hint line with a `·` separator
[fartcode:asked=human] Is post-fetch staleness of the counts acceptable, or must this change address it? — accepted as-is, no new fetching
[fartcode:asked=human] Do the arrow glyphs need an accessible label? — no, bare text matches the hint line's existing shorthand
[fartcode:asked=human] What does 'done' require? — new GitFooter component tests for the render/absent cases plus green typecheck/build

## Plan — 2026-08-15

Implementation plan for the Grill decisions. Files verified against this worktree: `app-frontend/src/components/GitFooter.tsx` (component), `app-frontend/src/store/commit-state.ts` (zustand store — tests seed via `useCommitState.setState({ byWorkspace })`), `app-frontend/src/lib/tauri.ts` (`GitCommitStateDto` already has `upstream/ahead/behind/remotes`), `fartcode-git/src/commit.rs` (stale consumer comment at the `upstream` field: “pull/push affordances key off this” — those verbs left for the ⌘K palette). Test conventions from sibling tests (`TaskHeader.test.tsx`, `SettingsModal.test.tsx`): vitest + @testing-library/react, `vi.mock("../lib/tauri")`, direct zustand seeding. No Rust logic changes anywhere.

### Steps

1. **Test scaffold + regression pins** — touches `app-frontend/src/components/GitFooter.test.tsx` (new). Mock `../lib/tauri` (only the symbols the import graph needs) and mock `../lib/useCommands` so `hint("open-command-palette")` returns a fixed `⌘K` — the exact-text assertions depend on it. Helper `seed(state)` that does `useCommitState.setState({ byWorkspace: { w1: { state, error: null } } })`. Write tests T3, T4, T5 (below). These pass against current code — they are the regression baseline pinning “null upstream / no entry / add-remote form” behavior before anything changes. Satisfies AC3, AC4, AC5 (guard side).
2. **Failing test for the segment** — touches `GitFooter.test.tsx`. Add T1 (`↑3 ↓2 origin/main · ` prefix). Run `vitest run` — must fail (segment not rendered yet). Prepares AC1.
3. **Render the segment** — touches `app-frontend/src/components/GitFooter.tsx`. In the `fc-footer-hint` `<p>`, when `st?.upstream != null`, prepend `↑{st.ahead} ↓{st.behind} {st.upstream} · ` before the existing text; otherwise render the line exactly as today. No new element, no aria-label, no other changes. T1 goes green; T3/T4/T5 stay green. Satisfies AC1.
4. **Zero-count pin** — touches `GitFooter.test.tsx`. Add T2 (`↑0 ↓0 origin/main · ` when synced). Expected to pass immediately given step 3's unconditional counts — it exists to pin “no zero-hiding” against future cleverness. If it fails, step 3 was wrong; fix there. Satisfies AC2.
5. **Correct the stale Rust comment** — touches `fartcode-git/src/commit.rs` only in the doc comment on `upstream`/`ahead`/`behind`: the footer's sync segment (`↑n ↓n <upstream>`) is now the consumer, not “pull/push affordances”. Comment-only; `cargo check -p fartcode-git` to prove nothing broke. Satisfies the comment half of AC7.
6. **Full gate** — no file edits. `vitest run` (all green incl. T1–T5), `tsc && vite build` in `app-frontend`, then `git diff --stat` against the branch base to confirm the only touched files are the two frontend files plus the commit.rs comment. Satisfies AC6 and the scope half of AC7. Commit.

### Test list (file: `app-frontend/src/components/GitFooter.test.tsx`)

- **T1 / AC1** — `it("prefixes the hint line with ↑n ↓n upstream when the branch tracks one")`: seed `{ upstream: "origin/main", ahead: 3, behind: 2, remotes: ["origin"] }`, assert the hint `<p>` text starts `↑3 ↓2 origin/main · d discards after a confirm · fetch / pull / push in ⌘K`. **Written failing (step 2).**
- **T2 / AC2** — `it("shows ↑0 ↓0 rather than hiding zero counts when synced")`: same seed with `ahead: 0, behind: 0`, assert prefix `↑0 ↓0 origin/main · `.
- **T3 / AC3** — `it("renders the bare hint line when upstream is null")`: seed `{ upstream: null, ahead: 0, behind: 0, remotes: ["origin"] }`, assert hint text is exactly `d discards after a confirm · fetch / pull / push in ⌘K` — no arrows, no leading separator.
- **T4 / AC4** — `it("renders the bare hint line without throwing when no state entry exists")`: no seed (`byWorkspace: {}`), assert render succeeds and hint text equals T3's.
- **T5 / AC5** — `it("still shows the add-remote form when remotes is empty")`: seed `{ upstream: null, ahead: 0, behind: 0, remotes: [] }`, assert the remote-name/remote-URL inputs render.
- **AC6** — no single named test, and none is invented: AC6 is the aggregate gate (T1–T5 green + `tsc && vite build` green), executed as step 6.
- **AC7** — not unit-testable, said loudly: “no changes outside app-frontend except the commit.rs comment; no new fetch/poll logic” is verified mechanically in step 6 by `git diff --stat` + reviewing the diff for absence of fetch/poll calls, not by a test.

### Risks, riskiest first

1. **Import-graph side effects sink the new test file (steps 1–2).** `GitFooter` imports `useChanges`, `useCommitState`, `useUi`, `useAsyncSubmit`, `hint` — module-level code may reach tauri APIs beyond what a minimal `vi.mock("../lib/tauri")` provides. Mitigation: copy the mock breadth from `TaskHeader.test.tsx` and grow it until import succeeds; this is why the scaffold is its own step before any behavioral test.
2. **Exact-string assertions are brittle (T1–T4).** The `·` is U+00B7 and `paletteKey` comes from `hint()` with a `"⌘K"` fallback; an unmocked bindings store could yield a different chord and fail AC3's byte-exact claim. Mitigation: mock `../lib/useCommands`'s `hint` to a constant in the scaffold.
3. **Conditional-render regression in step 3.** The prepend must not disturb the `remotes.length === 0` branch or the null paths — exactly what T3/T4/T5 exist to catch before the change lands.
4. **Rust comment edit (step 5).** Near-zero risk (doc comment), bounded by `cargo check`.

All seven acceptance criteria are covered; none was unplannable — AC6/AC7 are gates rather than unit tests, as flagged above.

- Tradeoffs: T2 and the T3–T5 pins will likely be green on arrival rather than red-first — accepted, since their value is regression-guarding, and forcing them red would mean writing deliberately wrong code. Exact-text assertions trade robustness for precision; the mocked `hint` keeps them deterministic.
- Rejected: extracting the segment into a helper/component — one template expression in an existing `<p>` doesn't justify a new unit. Rejected: snapshot tests — the criteria name exact strings; explicit assertions read as the spec. Rejected: touching `commit-state.ts` or `tauri.ts` — the DTO and store already carry the fields end to end; the plan verified this, so the change stays confined to the component, its test, and one Rust comment.

## Implement — 2026-08-15

Executed the plan's six steps in order, strictly red-first where the plan called for red. Step 1's pins (T3 bare hint on null upstream, T4 missing state entry, T5 add-remote form) went green against unchanged code, as intended. Step 2's T1 failed with exactly the expected diff (`d discards…` vs `↑3 ↓2 origin/main · d discards…`); step 3 made it pass with a single conditional template expression prepended inside the existing `fc-footer-hint` `<p>` — no new element, no aria-label, `st?.upstream != null` guards the whole segment. Step 4's T2 (`↑0 ↓0` never hidden) passed on arrival, as the plan predicted for a pin. Step 5 rewrote the stale `commit.rs` doc comment to name the footer sync segment as the consumer (the old text still claimed pull/push affordances that had moved to the ⌘K palette); `cargo check -p fartcode-git` green. Step 6: full suite 303/303 across 27 files, `tsc && vite build` green, `git diff` scope confined to `GitFooter.tsx`, `GitFooter.test.tsx`, and the `commit.rs` comment.

Deviations from the plan, both small:

- The worktree had no `node_modules`; `npm install` in `app-frontend` before any test could run. Environment, not design.
- The test fixture needed `hasRemote: true` — a `GitCommitStateDto` field the plan's fixture sketch omitted. Caught by `tsc` in the step-6 gate, not by vitest (which doesn't typecheck); fixed in the fixture only.

- Tradeoffs: T2/T3/T4/T5 are green-on-arrival pins rather than red-first drivers — accepted per the plan; forcing them red would mean writing deliberately broken code first.
- Rejected: JSX whitespace "cleanup" of the conditional expression — textContent already matches the criteria byte-for-byte, and touching whitespace would only risk the exact-string pins.

## Adversarial — 2026-08-15

Hostile pass over `540c43a..HEAD`. Every finding below was verified against the code (not the dossier's claims) before being reported. Ranked by severity; nothing was fixed.

### Findings

1. **[Medium] The Grill's no-upstream premise is false for local-tracking branches, and the resulting combined state is unreached by any test.** `fartcode-git/src/remote.rs:130–136` — `upstream_of` is `git rev-parse --abbrev-ref @{upstream}`, which succeeds for a branch tracking a *local* branch (`branch.X.remote = "."`, e.g. after `git branch --track feat main`) and returns a bare `main` with no remote prefix. `fartcode-git/src/lib.rs:405–411` — `remotes()` is `git remote`, empty in that repo. So `{ upstream: "main", ahead: n, behind: n, remotes: [] }` is a reachable DTO state, disproving the Grill's recorded “no remotes ⇒ certainly no upstream”. In that state `GitFooter.tsx:41` renders the add-remote form AND `GitFooter.tsx:83` renders `↑n ↓n main` simultaneously — a “no remote configured” prompt above a sync segment claiming an upstream. Not an AC violation (AC1 says render whenever `upstream` is non-null) and arguably even truthful, but it is an unconsidered state with zero test coverage: T5 (`GitFooter.test.tsx:82–87`) only pins `remotes: []` with `upstream: null`.
2. **[Low] No overflow handling for long upstream names.** `app-frontend/src/styles/changes.css:408–413` — `.fc-footer-hint` sets font and color only: no `white-space`, `overflow`, or `text-overflow`. Git branch names are effectively unbounded; `↑0 ↓0 origin/feature/very-long-conventional-name…` wraps the footer onto multiple lines and pushes the panel layout. The Grill never asked about truncation (a grill gap, not an implementation deviation), so the spec is silent — but the edge exists and nothing covers it.
3. **[Low] The `⌘K` fallback branch is untestable dead weight under the current mock.** `GitFooter.tsx:37` (`hint("open-command-palette") || "⌘K"`) — the test mock at `GitFooter.test.tsx:30` always returns the truthy `"⌘K"`, so the `||` fallback is never exercised by any test; AC3's “exact text” is only proven under the mocked chord. Pre-existing pattern (the fallback predates #133), but the new byte-exact assertions institutionalize the blind spot.
4. **[Info] File-header comment drift.** `GitFooter.tsx:3` still enumerates the footer's contents as “the mono hint line, the add-remote mini-form … and inline errors” — the sync segment, this issue's whole point, is absent from the inventory. The `commit.rs` comment was updated; the component's own header was not.

### Checked and cleared

- **AC1–AC5 each have a genuinely discriminating test:** T1 was watched red with the exact expected diff before the implementation existed (session log); T2–T5 assert full-string equality via `toBe`, not `toContain`/`startsWith`, so a wrong separator, missing space, or leaked `false` from the JSX conditional would fail them. Mutating the condition at `GitFooter.tsx:83` (e.g. `!= null` → truthy check) survives — but only because no AC distinguishes `""` upstream, which serde cannot produce from `Option<String>` without an empty shorthand, and `upstream_of` filters empty strings (`remote.rs:134`).
- **JSX whitespace:** the segment template ends `· ` (trailing space) and JSX drops the newline-indent before `d discards`, so textContent is byte-exact — confirmed by the passing `toBe` assertions, not assumed.
- **AC7 scope + no fetch/poll:** diff `540c43a..HEAD` touches exactly `GitFooter.tsx` (+1 line), `GitFooter.test.tsx` (new), `commit.rs` (doc comment only — `cargo check` green); no new invoke/fetch/poll/timer anywhere in it.
- **Security:** `upstream` is interpolated into a JSX template string rendered as a text node — React-escaped; no `dangerouslySetInnerHTML`, no injection surface even for hostile branch names.
- **Races/silent failures:** the change is a pure render expression over already-fetched store state; no new async, no new subscription, no failure path added. `st === null` and missing entries collapse into the same guarded branch (T3/T4).
- **Comment truth in `commit.rs:56–59`:** the new text claims the verbs moved to the ⌘K palette — corroborated by `GitFooter.tsx:2` and the absence of verb buttons in the component.
- **DTO fixture honesty:** `GitFooter.test.tsx` fixture now matches `GitCommitStateDto` exactly (the missing `hasRemote` was caught by `tsc` during implementation, not left to drift).

Decision: findings recorded, none fixed — the brief is find-only. Finding 1 is the only one arguing for a follow-up (either a pin test blessing the combined state or a `remotes.length > 0` guard decision, which would need a human call since it contradicts AC1's letter).

- Tradeoffs: finding 1 is reported at Medium on reachability grounds even though local-tracking branches are rare in this app's worktree-per-issue flow — severity leans on “unconsidered and untested”, not likelihood.
- Rejected: reproducing finding 1 with a live `git branch --track` fixture repo — the Rust source (`rev-parse --abbrev-ref @{upstream}` vs `git remote`) proves reachability without a repro harness, and the review brief forbids building new artifacts beyond the dossier.

## Implement — 2026-08-15 (post-review pass)

The plan's six steps were already executed and gated (first Implement section); this re-entry resolves the four Adversarial findings. Findings 1 and 2 needed a human call — the review itself said a `remotes.length` guard would contradict AC1's letter — so both went to the human before any code moved.

- Finding 1 (local-tracking upstream, Medium): human chose to BLESS the combined state. New pin T6 seeds `{ upstream: "main", ahead: 1, behind: 4, remotes: [] }` and asserts the sync segment AND the add-remote form render together. No implementation change; AC1's letter stands, and the Grill's false “no remotes ⇒ no upstream” premise is now corrected by a test rather than silently relied on.
- Finding 3 (dead `⌘K` fallback branch): new pin T7 uses `mockReturnValueOnce("")` so `hint()` goes falsy and the `|| "⌘K"` branch at `GitFooter.tsx:37` actually executes — the fallback is no longer untested.
- Finding 2 (overflow): human chose CSS ellipsis — `white-space: nowrap; overflow: hidden; text-overflow: ellipsis` on `.fc-footer-hint` (`changes.css`), with a comment naming the finding. Not unit-testable in jsdom (no layout); recorded here as the one deviation from strict test-first in this pass.
- Finding 4 (header comment drift): `GitFooter.tsx` header now names the sync segment in its inventory of footer contents.

TDD note, honestly: T6 and T7 are green-on-arrival pins, not red-first drivers — the behavior they cover already existed and the human blessed it; forcing red would mean breaking working code first. Gate: 305/305 tests (was 303), `tsc && vite build` green, scope confined to the test file, the component's comment, and `changes.css`.

- Tradeoffs: the ellipsis clips the whole hint line, so an extremely long upstream can visually swallow the trailing `fetch / pull / push in ⌘K` hint — accepted over wrapping (which distorts the panel) and over a per-segment `max-width` span (needs a new element for a cosmetic edge).
- Rejected: guarding the segment on `remotes.length > 0` — the human ruled the combined state truthful, and the guard would have amended AC1 and hidden real ahead/behind data for local-tracking branches.

[fartcode:asked=human] Local-tracking upstream renders both the add-remote form and the sync segment — guard or bless? — bless it with a pin test
[fartcode:asked=human] Long upstream names wrap the unstyled hint line — out of scope or fix? — add CSS ellipsis

## Adversarial — 2026-08-15 (second pass, over `2622cf0..HEAD`)

Hostile pass over the post-review fix commit (`11dfc96`): T6/T7 pins, the `.fc-footer-hint` ellipsis, and the header-comment fix. Every claim below was checked against the code; nothing fixed.

### Findings

1. **[Medium] The finding-2 “fix” regresses the common case: `nowrap` clips the palette hint at the panel's DEFAULT width, not just for pathological branch names.** `app-frontend/src/styles/changes.css:414–418` applies `white-space: nowrap; overflow: hidden; text-overflow: ellipsis` to the whole hint line. The changes panel is `useGutterResize(400, 280, 640, -1)` (`ChangesSidebar.tsx:74`) — initial 400px, floor 280px. The hint with an ordinary segment (`↑0 ↓0 origin/main · d discards after a confirm · fetch / pull / push in ⌘K`) is 74 mono chars ≈ 490px at 11px JetBrains Mono (≈0.6em advance) — clipped at the 400px default, hiding `fetch / pull / push in ⌘K`, the only in-app discoverability of the palette verbs. At the 280px floor even the BARE 54-char line (≈356px) is clipped, where before this commit it wrapped and stayed readable. The previous Implement section's tradeoff (“an *extremely long* upstream can visually swallow the trailing hint”) understates the trigger: the default width plus a perfectly normal upstream already does it. The human chose “CSS ellipsis” for finding 2, but the granted premise was long-name overflow — the always-on clipping of the shortcut hint at default width was not surfaced in that question.
2. **[Low] T7's discrimination hangs on exactly one `hint()` call per render.** `GitFooter.test.tsx:103` uses `mockReturnValueOnce("")`; any second render (StrictMode, a future subscription tick) would consume the Once and re-call the restored `"⌘K"` default — the final DOM would then read `BARE_HINT` even with the `|| "⌘K"` fallback deleted, and T7 would pass while testing nothing. Today the component renders once and the pin genuinely discriminates (verified: without the fallback, first render yields `…push in ` ≠ `BARE_HINT`); the fragility is one render away.
3. **[Info] Header comment overstates.** `GitFooter.tsx:3` now says the mono hint line is “led by the ↑n ↓n <upstream> sync segment” — the segment is conditional (`st?.upstream != null`); most fresh branches have no upstream and the line is not led by anything. Pedantic, but this is the same comment that just got fixed for drift.

### Checked and cleared

- **T6 genuinely pins the blessed combined state:** full-string `toBe` on `↑1 ↓4 main · …` plus `getByLabelText("Remote name")` in one render — either half regressing fails it (`GitFooter.test.tsx:94–99`).
- **Mock leakage between tests:** the `mockReturnValueOnce` is consumed inside T7 itself, and `restoreMocks: true` (vite.config.ts) re-arms the factory default; test order (T7 second-to-last, followed by T5 which never reads hint text) cannot smuggle a pass.
- **Ellipsis mechanics:** the `<p>` is a stretch child of the column-flex `.fc-git-footer` (`changes.css:401–406`), so its width is constrained and `text-overflow: ellipsis` actually engages — the declaration is not dead CSS.
- **Sole consumer:** `.fc-footer-hint` appears only in `GitFooter.tsx`; the new rules bleed into no other surface.
- **Scope:** `git diff 2622cf0..HEAD --stat` = the test file, the component (comment + nothing behavioral), `changes.css`, and the dossier. No new async, no race surface, no security-relevant input handling in the pass.
- **Dossier honesty otherwise:** the section correctly labels T6/T7 as green-on-arrival pins and the CSS as a test-first deviation; both human questions carry `[fartcode:asked=human]` tags.

Decision: finding 1 argues the ellipsis choice should go back to the human with honest numbers (clip at default width vs wrap vs segment-only truncation via a `max-width` span); findings 2–3 are hardening/wording nits. Find-only brief — nothing changed.

- Tradeoffs: finding 1's pixel arithmetic assumes JetBrains Mono's ≈0.6em advance rather than a rendered measurement — stated as an estimate; the 280px-floor case needs no estimate at all to show the bare line clips where it previously wrapped.
- Rejected: downgrading finding 1 because the human approved “CSS ellipsis” — the approval was for the long-name edge; the always-on default-width clipping is a consequence the question never disclosed, so the sign-off does not cover it.

## Plan — 2026-08-15 (second-pass remediation)

Subject: the three findings of the second Adversarial pass (`9a9427a`). Finding 1 required a fresh human decision — the first “CSS ellipsis” sign-off was obtained without the default-width numbers — so the question was re-asked honestly before planning; the answer is REVERT TO WRAPPING. Original acceptance criteria AC1–AC7 are already covered and stay untouched; this plan adds no new ACs, it remediates review findings, and it is loud below about which of them admit a failing test (spoiler: none, and here is why each time).

### Steps

1. **Harden T7 against the second-render loophole (finding 2)** — touches `app-frontend/src/components/GitFooter.test.tsx:103` only: change `vi.mocked(hint).mockReturnValueOnce("")` to `vi.mocked(hint).mockReturnValue("")`. Every render in the test then sees the falsy chord, so a future StrictMode/second render can no longer consume the Once and mask a deleted fallback; `restoreMocks: true` re-arms the `"⌘K"` factory default after the test, and test order cannot leak (T5, the only later test, never reads hint text). **Honesty check, mandatory in-step:** temporarily delete `|| "⌘K"` at `GitFooter.tsx:37`, watch T7 go red, restore it, watch green — the mutation probe is the red state this step has, since the fallback already works and no committable failing test exists. Say it plainly: this is test hardening, not red-first TDD.
2. **Revert the full-line ellipsis (finding 1, human-decided)** — touches `app-frontend/src/styles/changes.css:408–419`: delete the `white-space: nowrap; overflow: hidden; text-overflow: ellipsis` declarations and their `#133` comment, restoring the block to `margin/font-family/font-size/color` exactly as before `11dfc96`. The line wraps again; nothing is ever hidden at any panel width (default 400px, floor 280px). **Loudly: this cannot be unit-tested** — jsdom performs no layout and never loads `changes.css`, so no failing test can exist for a stylesheet declaration; verification is (a) the diff shows the three declarations gone and (b) the seven textContent pins stay green, proving the revert touched no behavior.
3. **De-overstate the header comment (finding 3)** — touches `app-frontend/src/components/GitFooter.tsx:3–4`: “led by the ↑n ↓n <upstream> sync segment” → wording that marks the segment conditional (e.g. “prefixed, when an upstream exists, by the ↑n ↓n <upstream> sync segment”). Comment-only; comments are not behavior, no test, stated loudly rather than invented.
4. **Gate and commit** — no file edits: full `vitest run` (305 expected, all green), `tsc && vite build`, `git diff --stat` confined to the two frontend files, `changes.css`, and the dossier; then one commit and the Implement dossier section recording the mutation-probe result.

### Test list

No finding in this pass yields a committable failing test, and none is invented:

- **Finding 2** → existing `it("falls back to ⌘K when no palette binding is configured")` (`GitFooter.test.tsx:102`) is the covering test; the step strengthens its discrimination and proves it via the in-step mutation probe (fallback deleted → red; restored → green). A new test would duplicate it.
- **Finding 1** → NOT unit-plannable, said loudly: stylesheet layout is invisible to jsdom. The guard is the unchanged seven-pin suite plus diff inspection. If the project ever grows browser-level tests, a wrap assertion belongs there — out of scope for size:S.
- **Finding 3** → no test target exists for comment prose; covered by review, not by CI.

### Risks, riskiest first

1. **The mutation probe gets skipped (step 1).** The hardening only proves itself through the probe; an implementer who edits `Once` → `mockReturnValue` and moves on has verified nothing. Mitigation: the step text makes the probe mandatory and the Implement section must record its red/green result.
2. **Something grew a dependency on `nowrap` since `11dfc96` (step 2).** Checked during planning: `.fc-footer-hint` has exactly one consumer (`GitFooter.tsx:82`) and the rules were added six commits ago on this same branch — nothing else references them. Residual risk ≈ zero.
3. **Comment rewording drifts into inaccuracy again (step 3).** Bounded by the second-pass reviewer's exact complaint: the fix is to state conditionality, nothing more.

- Tradeoffs: reverting to wrap costs a second footer line for long upstreams (the very thing the first ellipsis tried to avoid) — accepted by the human once the clipping numbers were on the table: an occasional wrap beats permanently hiding the palette hint at default width. The T7 hardening keeps `restoreMocks` semantics as documented behavior rather than adding a call-count assertion — simpler, at the cost of not pinning “exactly one hint() call”.
- Rejected: segment-only truncation (`max-width` span) — human passed it over; adds an element and a magic width for an edge the wrap handles acceptably. Rejected: keeping the full-line ellipsis — that would have required knowingly signing off the default-width clipping of the palette hint, and the human declined. Rejected: asserting `hint` call counts in T7 — couples the pin to render mechanics instead of visible output.

[fartcode:asked=human] With honest numbers (≈490px line vs 400px default / 280px floor panel), keep full-line ellipsis, truncate only the segment, or revert to wrapping? — revert to wrapping

## Implement — 2026-08-15 (second-pass remediation)

Executed the remediation plan's four steps in order, no deviations.

- Step 1 (finding 2): `mockReturnValueOnce("")` → `mockReturnValue("")` in T7 (`GitFooter.test.tsx:103`), closing the second-render loophole. **Mutation probe result, as the plan demanded:** with `|| "⌘K"` deleted from `GitFooter.tsx:37`, exactly T7 went red (`× falls back to ⌘K when no palette binding is configured`, 1 failed | 6 passed); with the fallback restored, 7/7 green. The pin's discrimination is proven, not assumed — that probe was this step's red state, since no committable failing test exists for an already-working fallback.
- Step 2 (finding 1, human-decided): deleted the three `white-space/overflow/text-overflow` declarations and their comment from `.fc-footer-hint` (`changes.css`), restoring the block byte-for-byte to its pre-`11dfc96` form — the hint line wraps again at every panel width, and the palette-verbs hint is never hidden. Not unit-testable (jsdom loads no stylesheets, performs no layout) — verified instead by the diff (exactly 5 lines removed, nothing else) and by all seven textContent pins staying green, per the plan.
- Step 3 (finding 3): header comment now reads “prefixed, when an upstream exists, by the ↑n ↓n <upstream> sync segment” — conditionality stated, overstatement gone. Comment-only.
- Step 4: full suite 305/305, `tsc && vite build` green, `git diff --stat` confined to `GitFooter.test.tsx` (1 line), `GitFooter.tsx` (comment lines only), `changes.css` (−5), and the dossier's app-owned timeline.

- Tradeoffs: long upstream names can wrap the footer onto a second line — the human accepted this over hiding the ⌘K discoverability hint at the panel's default width once the real numbers were on the table. T7 pins visible output only, not “exactly one hint() call” — simpler, per the plan's explicit rejection of call-count coupling.
- Rejected: keeping any truncation (full-line or segment-only span) — the human's informed answer was to wrap; both truncation variants either hide the palette hint or add an element plus a magic width for an edge that wrapping absorbs.
