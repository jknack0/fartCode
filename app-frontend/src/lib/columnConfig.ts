// Column config → display strings and card placement (ADR-0037, handoff
// v3 §8a/§8d). Pure functions, no state: the board header sublines and the
// settings Columns editor (#67, §8d "the SAME strings the board headers
// use — one formatter") both call `columnConfigSummary`. Adding a second
// formatter anywhere is the bug this file exists to prevent.

import type { BoardColumnDto, IssueDto, Lane } from "./tauri";

/** Board order is `position`; the backend already sorts, but every
 * consumer that merges/patches columns locally must re-sort. */
export function sortColumns(columns: BoardColumnDto[]): BoardColumnDto[] {
  return [...columns].sort((a, b) => a.position - b.position);
}

/** Where an `on_settle: advance` card goes: the explicit `advanceTo`
 * target, else the next column by position (v3/ADR item 4). Null when the
 * column is last (advancing off the end degrades to hold, backend-side). */
export function advanceTarget(
  column: BoardColumnDto,
  columns: BoardColumnDto[],
): BoardColumnDto | null {
  if (column.advanceTo) {
    return columns.find((c) => c.id === column.advanceTo) ?? null;
  }
  const ordered = sortColumns(columns);
  const i = ordered.findIndex((c) => c.id === column.id);
  return i >= 0 ? (ordered[i + 1] ?? null) : null;
}

/** Subline brightness (v3 "Confirm-free spend is brighter"): `run` agent
 * steps read #9a9aa1, queue-mode steps `--meta`, everything else
 * `--meta-dim` (#4e4e55). Presence + brightness is the whole signal — no
 * new colour, no badge. */
export type ColumnSublineTone = "kind" | "queue" | "run";

export function columnSublineTone(column: BoardColumnDto): ColumnSublineTone {
  if (column.kind !== "agent_step") return "kind";
  return column.onEnter === "run" ? "run" : "queue";
}

/** THE column config summary — one formatter for §8a's header subline and
 * §8d's collapsed editor row.
 *
 * - `shelf` / `human gate` / `counts as done` for non-step columns;
 *   `counts_as_done` outranks the kind word, since the seeded terminal
 *   column is a shelf and its flag is the thing worth saying.
 * - agent steps: `<provider> · <model> · <effort> — <trigger>[ → <target>]`.
 *   A step with no pinned provider runs the app's default agent, so pass
 *   `defaultAgent` (the same name the dispatch confirm names); model and
 *   effort are omitted when unset rather than rendered as blanks. The
 *   arrow appears only for `on_settle: advance`, naming the resolved
 *   target — the flag alone never tells you where a card lands. */
export function columnConfigSummary(
  column: BoardColumnDto,
  opts: { columns?: BoardColumnDto[]; defaultAgent?: string | null } = {},
): string {
  if (column.kind !== "agent_step") {
    if (column.countsAsDone) return "counts as done";
    return column.kind === "human_gate" ? "human gate" : "shelf";
  }
  const agent = [
    column.stepProvider ?? opts.defaultAgent ?? null,
    column.stepModel,
    column.stepEffort,
  ]
    .filter((p): p is string => Boolean(p && p.trim()))
    .join(" · ");
  const trigger = column.onEnter === "run" ? "run" : "queue";
  let tail = trigger;
  if (column.onSettle === "advance") {
    const target = advanceTarget(column, opts.columns ?? []);
    if (target) tail += ` → ${target.name}`;
  }
  return agent ? `${agent} — ${tail}` : tail;
}

/** The artifact a held step leaves behind, for the step-done card hint
 * (`↵ read <artifact> · drag on`). No column field carries one yet — the
 * dossier work (#75) is where artifacts get declared — so this reads a
 * forward-compatible field and returns null meanwhile, which is exactly
 * v3's "omit the hint if none". */
export function stepArtifact(column: BoardColumnDto): string | null {
  const declared = (column as { stepArtifact?: string | null }).stepArtifact;
  return declared && declared.trim() ? declared.trim() : null;
}

/** Which column a card renders in.
 *
 * The mirror pointer wins when it resolves: it is the only thing that can
 * express a NON-seeded column (Quick, user-created), because
 * `enter_column` leaves `lane` stale for those. A never-moved card has no
 * mirror, so it falls back to the seeded column carrying its lane, then to
 * the landing column. This is DISPLAY resolution only — `lane` is still
 * the backend's authority until the E18-07 authority-flip round. */
export function columnIdForIssue(
  issue: IssueDto,
  columns: BoardColumnDto[],
): string | null {
  if (issue.columnId && columns.some((c) => c.id === issue.columnId)) {
    return issue.columnId;
  }
  const seeded = columns.find((c) => c.seedLane === issue.lane);
  if (seeded) return seeded.id;
  return columns.find((c) => c.isLanding)?.id ?? columns[0]?.id ?? null;
}

/** Cards per column id, each list in `position` order. Every column gets
 * an entry (an empty column is still a drop target and an h/l stop). */
export function groupByColumn(
  issues: IssueDto[],
  columns: BoardColumnDto[],
): Map<string, IssueDto[]> {
  const byColumn = new Map<string, IssueDto[]>();
  for (const c of columns) byColumn.set(c.id, []);
  for (const issue of issues) {
    const id = columnIdForIssue(issue, columns);
    if (id) byColumn.get(id)?.push(issue);
  }
  for (const list of byColumn.values()) {
    list.sort((a, b) => a.position - b.position);
  }
  return byColumn;
}

/** Display name for a blocker row. Prefers the ref's resolved column id —
 * a lane can only ever name a SEEDED column, so a blocker parked in Quick
 * used to be labelled with whatever column its stale lane pointed at, and
 * the same panel would name two different columns for one card. The lane
 * is the fallback for a ref whose column resolved to nothing. */
export function blockerColumnName(
  blocker: { lane: Lane; columnId: string | null },
  columns: BoardColumnDto[],
): string {
  if (blocker.columnId) {
    const named = columns.find((c) => c.id === blocker.columnId);
    if (named) return named.name;
  }
  return columns.find((c) => c.seedLane === blocker.lane)?.name ?? blocker.lane;
}

/** The column new work lands in (GitHub import, PM apply, manual add) —
 * ADR-0037 item 7: one per board, never a name test. */
export function landingColumn(columns: BoardColumnDto[]): BoardColumnDto | null {
  return columns.find((c) => c.isLanding) ?? columns[0] ?? null;
}
