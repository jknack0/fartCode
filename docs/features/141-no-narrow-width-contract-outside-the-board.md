# #141 No narrow-width contract outside the board

<!-- fartCode feature dossier (ADR-0038). Append-only: add sections, never rewrite existing ones. The app owns `## Timeline`; agents add `## <Column> — <date>` sections below it. -->

## Context

Labels: enhancement, size:M

**Evidence:** `app-frontend/src/styles.css` has exactly one `@media` rule.

**Impact:** rail, flyout, task view, changes panel and PR tab have no defined behaviour below the board's breakpoint; `.rail` also has no overflow handling, so past ~15 projects the `+` and ⌘ tiles push off-screen.

**Fix:** define breakpoints per surface; make the rail's project tiles a scrolling region with the mark, `+` and ⌘ pinned.

_Filed from the 2026-08-12 code audit (successor to the deleted `docs/e2e-scenarios.md` gap register); each claim re-verified against `main` at the time of filing._

## References

- card: `iss_c9bb1adb-98be-4a27-9edf-079e5383e99a`
- source: import · https://github.com/jknack0/fartCode/issues/141
- tracker: https://github.com/jknack0/fartCode/issues/141

## Timeline
<!-- fartcode:timeline -->

- 2026-08-14 18:14:47 · created · import · https://github.com/jknack0/fartCode/issues/141
- 2026-08-14 20:20 · dossier created with the worktree · Plan
- 2026-08-14 20:20 · Plan · launched · pi

## Plan — 2026-08-14

**Loud finding 1 — there is no grill record to plan from.** This dossier has `## Context`,
`## References` and `## Timeline` and nothing else: no Grill section, no ratified decisions, no
acceptance criteria. Everything under "Criteria" below is *reconstructed* by me from the issue
text plus the two places the product already states a narrow rule (`DESIGN.md:264-265`,
`design_handoff_left_nav/README.md` §Narrow (4g)). Treat them as proposed, not agreed. If the
grill decided something else — particularly about the flyout and the task view — this plan aims
at the wrong target and should be rejected rather than executed.

**Loud finding 2 — the fix this issue asks for is already on the branch.** `7e1db3a`
("fix(rail): scroll project tiles, narrow-width contract (#141)", 2026-08-13) is an ancestor of
HEAD, i.e. it landed *after* the 2026-08-12 audit re-verified the claim. In the working tree today:

- `app-frontend/src/components/Nav.tsx:85` wraps the project tiles in `.rail-scroll`; the mark,
  `+`, `.rail-spacer` and the `⌘` tile are siblings outside it.
- `app-frontend/src/styles.css:398-413` gives `.rail-scroll` `flex: 0 1 auto; min-height: 0;
  overflow-y: auto` with the scrollbar hidden.
- `app-frontend/src/styles.css:415-436` is the `@media (max-width: 899px)` §4g block: rail 48px,
  tiles 30×30, `.changes-panel { max-width: 100% }`, with a comment recording that the flyout and
  the task view deliberately get *no* narrow variant (fixed-width + internal scroll, and
  fluid + truncating, respectively).
- `Nav.test.tsx:120` pins the mark/`+`/`⌘` as outside the scroller.

So the implementer's job is **not** to build this. It is to prove the claims are true and covered,
and to close the three gaps the landed commit left. Any step that re-implements a scrolling rail
or a second `<900px` block is a defect, not progress.

**Bound on the problem.** `fartcode-app/tauri.conf.json` sets `minWidth: 800`. The undefined band
is therefore only 800–899px — the app cannot be dragged narrower. At 800px the rail (48) plus the
flyout (244) leave 508px of main pane. Nothing below 800px needs a rule, and any step proposing
phone-width layouts is out of scope.

### Criteria (reconstructed — unratified)

- **A. Rail overflow.** With ~15+ projects the `+` and `⌘` tiles stay on-screen and reachable; the
  project tiles scroll instead of pushing them off. *(Issue: "Fix" clause 2.)*
- **B. Rail narrow.** Below 900px the rail is 48px and its tiles 30×30. *(DESIGN.md:264-265, §4g.)*
- **C. Changes sheet narrow.** Below 900px the changes sheet cannot exceed the main pane, so a
  drag-resized sheet (`useGutterResize(400, 280, 640)`, `ChangesSidebar.tsx:74`) cannot cover the
  rail or flyout at an 800px window.
- **D. PR tab narrow.** The PR tab inherits C because it renders *inside* `.changes-panel`
  (`ChangesSidebar.tsx:298`); that containment is the whole of its narrow contract and must be
  pinned so moving the panel out doesn't silently drop it.
- **E. Flyout and task view.** Both are width-stable by design and get no narrow variant — and that
  decision is written somewhere an auditor reads, not only in a CSS comment.

### Steps (ordered, one sitting each)

0. **Restore the toolchain.** `npm ci` in `app-frontend/` (no `node_modules` in this worktree —
   `npx vitest run` currently dies at `Cannot find package 'vitest'`), then run the full suite and
   record the baseline. Touches: nothing. Satisfies: none — gate for every step below.
1. **Mutation-check the existing rail test.** Temporarily unwrap `.rail-scroll` in `Nav.tsx`,
   confirm `Nav.test.tsx:120` turns red, restore. A criterion whose test cannot fail is not
   covered. Touches: `app-frontend/src/components/Nav.tsx` (reverted). Satisfies: A (verification).
2. **Strengthen the rail test to the issue's actual claim.** The current test asserts DOM
   *structure*; the issue's claim is about ~15+ projects. Add a test that renders 30 projects and
   asserts all 30 tiles are inside `.rail-scroll` while `+` and `⌘` remain direct children of
   `.rail` after it. Touches: `app-frontend/src/components/Nav.test.tsx`. Satisfies: A.
3. **Pin the §4g rail rule.** New `app-frontend/src/test/narrowContract.test.ts`: read
   `src/styles.css`, extract the `@media (max-width: 899px)` block, assert `.rail { width: 48px }`
   and `.rail-tile { width/height: 30px }`. Red-first by mutating the declaration, not by writing
   the feature. Touches: new test file. Satisfies: B.
4. **Pin the changes clamp.** Same file: assert the §4g block declares `max-width: 100%` for
   `.changes-panel`. Touches: `narrowContract.test.ts`. Satisfies: C.
5. **Pin PR-tab containment.** In `narrowContract.test.ts` (or `ChangesSidebar`'s suite): assert the
   rendered PR tab is a descendant of `.changes-panel`. Touches: test file only. Satisfies: D.
6. **Write the contract down.** Extend `DESIGN.md`'s Layout section (currently only "board collapses
   to one column and the rail narrows to 48px") with a five-row per-surface table — rail, flyout,
   task view, changes sheet, PR tab — each with its rule or an explicit "no narrow variant, and
   why", plus the 800px floor from `tauri.conf.json`. Reduce the `styles.css:415` comment to a
   pointer so there is one source of truth. Touches: `DESIGN.md`,
   `app-frontend/src/styles.css`. Satisfies: E, and it is the step that stops the next audit
   refiling this issue.
7. **Close out.** Full `npm test`, then append an Implement section here. Touches: this file.

### Tests (one named test per criterion)

| Criterion | Test | File |
|---|---|---|
| A | `keeps the + and ⌘ tiles reachable with 30 projects` | `components/Nav.test.tsx` |
| B | `narrows the rail to 48px with 30px tiles below 900px` | `test/narrowContract.test.ts` |
| C | `clamps the changes sheet to the main pane below 900px` | `test/narrowContract.test.ts` |
| D | `renders the PR tab inside the clamped changes sheet` | `test/narrowContract.test.ts` |
| E | **none — say so out loud** | — |

How these go red matters. B, C and D describe behaviour that already exists, so there is no missing
feature to make them fail; the TDD step is a *mutation* (delete the declaration, confirm red,
restore), exactly as #142 handled its already-landed fix. An implementer who writes these tests
green and moves on has verified nothing.

**Criterion E has no test and I will not invent one.** Asserting that a sentence exists in
`DESIGN.md` pins a string, not a behaviour, and would rot on the first rewording. E is satisfied by
review of the step-6 diff.

### Risks, riskiest first

1. **The criteria above are mine, not the grill's.** No grill record exists (finding 1). If the
   grill ratified a narrow variant for the flyout or the task view, steps 3–6 encode the opposite
   decision and the whole plan is wrong. Stop and get the criteria before step 1.
2. **Re-implementation.** The obvious failure mode is an implementer reading the issue, not the
   branch, and adding a second scrolling rail or a second `@media` block — there is already a
   sibling worktree on this issue (`…-us2gx`). Step 0 must include reading `git show 7e1db3a`.
3. **The new tests are string assertions over CSS.** They can pass while the rendered UI is broken,
   because jsdom applies no media queries and computes no layout. They guard against *deletion*,
   which is the realistic regression, and nothing more. Do not let them stand in for looking at the
   window.
4. **No unit test can confirm the layout actually holds at 800px.** jsdom has no layout engine, and
   the repo has no browser-driven test. Criteria B and C are ultimately verified by resizing the
   real window; the plan should say so rather than imply green tests mean a correct layout.
5. **`npm ci` needs the network** and has never been run in this worktree; if the lockfile install
   fails, every step is blocked and that is the only thing to report.
6. **Touching `styles.css:415`'s comment (step 6) risks a merge conflict** with the sibling
   worktree. Cheapest mitigation: make step 6 last, as ordered.

- Tradeoffs: I planned verification and documentation rather than features, which will read as a
  thin diff for a `size:M` issue; I accepted that over manufacturing narrow variants the design
  explicitly declines. I also accepted brittle CSS-source tests as the only automatable guard
  available in this stack, instead of introducing a browser test runner for two declarations.
- Rejected: planning fresh implementation of the rail scroller and the §4g block — both are on the
  branch and tested; duplicating them would produce conflicting CSS.
- Rejected: adding narrow variants for the flyout and task view — the landed decision is that they
  are width-stable, and at the 800px floor they cost 244px of a 800px window, which the board
  absorbs. Overturning that is a design change, not this issue.
- Rejected: a media-query test via `window.matchMedia` — jsdom's `matchMedia` is a stub that never
  evaluates the stylesheet, so such a test asserts the mock, not the CSS.
- Rejected: a test asserting `DESIGN.md` wording (criterion E) — it pins prose, not behaviour.

[fartcode:asked=memory] What is the lower bound of the undefined width band? — `fartcode-app/tauri.conf.json` sets `minWidth: 800`, so the whole gap is 800–899px.
[fartcode:asked=human] There is no Grill section in this dossier — were acceptance criteria ever ratified, and do they agree that the flyout and task view get no narrow variant? The plan above assumes they do.
