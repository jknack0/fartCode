# ADR-0038: Feature dossiers — per-feature project memory, seeded skill, FTS-searchable

**Status:** accepted (design review 2026-08-09, handoff v3 — companion to
ADR-0037). Frames settled the open surfaces: dashboard lives at settings →
project → Memory; delete-with-issues renders as a disabled label with the
reason, not a dialog; consent card fires once per project before any queue
confirm, and declining still dispatches.

## Context

Once columns are pipeline steps (ADR-0037), each feature accumulates real
decisions — grill answers, plan tradeoffs, review verdicts — that today
evaporate into per-task transcripts and get torn down with the worktree.
The want: every step and decision along a feature's lifecycle documented as
project memory that future agents and chats inherit, searchable, with a
skill so agents know the convention exists.

The substrate is already there. ADR-0032 settled repo-first docs ("PRD =
markdown in the repo… agents read them with normal file access. No DB
blob") and issues already carry `prd_path`/`prd_section`. The ⌘K search
index is a generic trigram FTS5 table `search_index(item_type, item_id,
project_id, task_id, title, keywords)` (fartcode-core/src/search.rs) — new
item types are rows, not schema. The event channel already emits every
lifecycle fact a timeline needs. And ADR-0037 gives every step its own
system prompt — a place to put an append instruction. This repo's own
`decisions/` + `MEMORY.md` practice is the hand-rolled version this ADR
productizes for managed projects.

## Decision

1. **One dossier per feature: `docs/features/<slug>.md`.** Created by the
   app at the card's first `agent_step` entry — the same moment the
   worktree provisions — so the file is born on the feature branch and
   travels with it. It is born with a **backfilled header** from what the
   app already holds: issue title/body/acceptance, PRD link + section,
   proposal provenance, and the pre-worktree timeline — the grill answers
   that shaped the feature are in the file from day one. The issue gains
   `dossier_path` beside `prd_path`; the card detail links it. Slug derives
   from the issue ref/title the same way task names do.
2. **App writes the skeleton; agents write the substance.** The app appends
   machine breadcrumbs from events it already emits — created (source:
   proposal / import / manual, backfilled at dossier creation), step
   launched (column, provider, model), step settled, lane moves, PR opened/
   merged — under a `## Timeline` section. It writes only while a worktree
   exists; pre-provision history is backfilled at creation, post-teardown
   events go unrecorded (the feature is done). Each seeded step prompt ends
   with the append instruction: before settling, add `## <Column> — <date>`
   with decisions made, tradeoffs, and rejected alternatives, in the
   agent's own words. A skipped append leaves the facts intact — only the
   reasoning section is missing.
3. **The convention ships as a seeded repo skill.** fartCode scaffolds
   `.claude/skills/feature-log/` plus an `AGENTS.md` pointer into managed
   projects: where dossiers live, the section format, append discipline,
   and how to search them (grep). Any agent CLI in any tool learns the
   history exists — not just fartCode-launched steps. Seeding is
   provenance-tagged and **opt-in per project, asked at first dispatch** —
   the consent lands the moment value is imminent ("this feature will keep
   a dossier — write the convention files to your repo?"); declining runs
   the step without memory, and project settings carries the same switch in
   both directions, so the decision is reversible either way (writing into
   a user's repo is never silent; same consent posture as project-settings
   share).
   *Rejected:* prompt-only convention (invisible to agents outside the
   pipeline) and an MCP tool server (ADR-0032 already defers MCP surfaces
   to the E10 era).
4. **Dossier sections feed ⌘K.** New `item_type: "feature"` rows in
   `search_index` — one per dossier section, title = heading, keywords
   extracted from the section body. Reindex on step settle (worktree copy)
   and on project pull (main-branch copy). **Enter opens the card detail**,
   which gains a dossier section — one destination that works for live and
   landed features alike, since issue rows persist after the task is gone.
   Agents don't need the UI — they grep.
5. **Merge is publication.** Dossier commits ride the feature branch, so
   the project knowledge base grows exactly when features land on main. A
   deleted unmerged branch takes its dossier with it — the same risk
   profile as the code, which is the point. Archived tasks keep worktree +
   branch (existing semantics), so archived features keep their dossiers.
6. **The moat is the intelligence, not the data.** Considered and
   **rejected**: app-owned memory as a retention lever (DB-only store,
   with or without export). It fights the feature's own premise — steps
   read prior steps' output from the worktree, and outside-app agents
   learn the convention from the seeded skill; app-only storage blinds
   both and fattens every prompt packet with injected context. Retention
   comes from what only the app can do with repo-resident memory: the FTS
   index, card↔dossier↔session↔PR linking, timeline rendering, and the
   value dashboard (item 7). Leaving fartCode keeps your notes; it loses
   the living system.
7. **Memory value is measured, locally.** fartcode-telemetry computes four
   signals, in-app only: **memory citations** (step sessions referencing
   dossier content), **re-ask rate** (clarifications answered from memory
   vs. re-asked to the human), **context tokens saved** (referencing vs.
   re-deriving, from provider usage metadata), and **time-to-land trend**
   (cycle time as the knowledge base grows — the headline chart, and the
   noisiest; never presented without the caveat). Surfaced as a value
   dashboard ("your project memory saved N re-explanations this month") —
   frame TBD in design review.
8. **fartCode dogfoods the convention** on managed projects first; whether
   this repo migrates its own `decisions/` + `MEMORY.md` practice onto
   dossiers is explicitly out of scope.

## Consequences

- **A new consent surface.** Dossier + skill seeding writes into user
  repos; project creation (or first step dispatch) needs an explicit
  opt-in, and the files must be provenance-tagged like shared settings.
  This is the main design-review item beyond ⌘K hit rendering and the
  card-detail dossier link.
- Seeded step-prompt templates grow an append instruction and are
  versioned together with the skill scaffold — a prompt/skill mismatch
  (skill describes a format prompts no longer request) is the new
  staleness risk.
- Merge conflicts on a dossier are rare by construction (one feature = one
  branch writes it) but possible after cross-feature edits; the format is
  append-only sections to keep conflicts trivial.
- The FTS rowid scheme (FNV-1a over `item_type:item_id`) accommodates
  per-section ids unchanged; deleting a feature's rows on dossier removal
  needs the same delete path tasks use today.
- Metric attribution needs mechanisms: citations require detecting dossier
  references in transcripts (path-mention heuristic first); re-ask rate
  requires the grill steps to tag questions as memory-answered vs.
  human-asked (a step-prompt convention, versioned with the skill); token
  math comes from provider usage metadata the transcript reducer already
  sees; time-to-land derives from Timeline events. All local — no metric
  leaves the machine.
- The value dashboard is a new design-review surface alongside the
  first-dispatch consent card, the ⌘K feature-hit row, and the card
  detail's dossier section.
- **Transcript indexing is deferred, with a decision hook:** raw
  transcripts stay out of FTS until the citation metrics (item 7) show
  whether agents exhaust the dossier layer first — the metrics are
  literally the input to reopening this.
