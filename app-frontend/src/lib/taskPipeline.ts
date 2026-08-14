// Pipeline context for the TASK view (ADR-0037, DESIGN.md "Pipeline board").
//
// The pipeline's decision points used to live only on the board: a step
// that settles in a `hold` column renders its step-done dot on a CARD,
// and a parked (queue-mode) step is confirmed by the board's overlay —
// while the person who needs to act is almost always in the task view
// watching the agent that just finished. This module is the derivation
// half of closing that gap; TaskHeader is the surface.
//
// Everything here is PURE except the four runners at the bottom, and
// every rule is borrowed rather than restated:
//   - the card's column comes from `columnIdForIssue` (the board's
//     mirror-else-seedLane resolution — a second one would drift);
//   - the advance target comes from `advanceTarget`, which is
//     `advance_to ?? next-by-position`, the same rule the settle engine
//     applies in step_engine.rs `settle_issues_for_task`;
//   - the spend a parked step will make is named by `columnConfigSummary`,
//     the one formatter the board headers and the settings editor share;
//   - the issue's short ref comes from `issueRefParts`, the board's
//     display-derived `#12`.

import {
  gitStatus,
  issueEnterColumn,
  stepConfirm,
  taskShip,
  type BoardColumnDto,
  type IssueDto,
} from "./tauri";
import { advanceTarget, columnIdForIssue } from "./columnConfig";
import { issueRefParts } from "../components/board/runState";
import { ensureDossierConsent } from "../store/dossierConsent";
import { useColumns } from "../store/columns";
import { useSidebar } from "../store/sidebar";
import { useSteps, type StepFlags } from "../store/steps";
import { useTaskCard } from "../store/taskCard";
import { useUi } from "../store/ui";

/** Everything the header knows about the open task's place in the board.
 * `issue === null` is the ⌘N ad-hoc task: no card, no pipeline, and every
 * action below is invisible and inert. */
export interface TaskCardContext {
  issue: IssueDto | null;
  /** The card's column, resolved the board's way. */
  column: BoardColumnDto | null;
  columns: BoardColumnDto[];
  flags: StepFlags | undefined;
}

/** The board card linked to a task, if any. `linkedTaskId` is the live
 * link — `tasks.linked_issue` is still unset in production. */
export function cardForTask(
  taskId: string | null,
  issues: IssueDto[],
): IssueDto | null {
  if (!taskId) return null;
  return issues.find((i) => i.linkedTaskId === taskId) ?? null;
}

export function taskCardContext(input: {
  taskId: string | null;
  issues: IssueDto[];
  columns: BoardColumnDto[];
  steps: Record<string, StepFlags>;
}): TaskCardContext {
  const issue = cardForTask(input.taskId, input.issues);
  if (!issue) {
    return { issue: null, column: null, columns: input.columns, flags: undefined };
  }
  const columnId = columnIdForIssue(issue, input.columns);
  return {
    issue,
    column: input.columns.find((c) => c.id === columnId) ?? null,
    columns: input.columns,
    flags: input.steps[issue.id],
  };
}

/** The open task's context, read straight off the stores — the snapshot a
 * registry command sees. The header derives the same thing through hooks
 * so it re-renders; both call `taskCardContext`, so there is one rule. */
export function currentTaskCardContext(): TaskCardContext {
  const sb = useSidebar.getState();
  const projectId = sb.selectedProjectId;
  return taskCardContext({
    taskId: sb.selectedTaskId,
    issues: projectId
      ? (useTaskCard.getState().issuesByProject[projectId] ?? [])
      : [],
    columns: projectId ? (useColumns.getState().byProject[projectId] ?? []) : [],
    steps: useSteps.getState().byIssue,
  });
}

/** Breadcrumb segments, left to right.
 *
 * Ad-hoc task → `[project]`, byte-identical to what the header rendered
 * before the pipeline existed. Carded task → `[project, column, ref]`,
 * and a card with no derivable ref (a local issue, never imported) simply
 * has no third segment — an empty separator would claim a pipeline
 * position the card does not have. */
export function pipelineCrumb(
  projectName: string,
  ctx: TaskCardContext,
): string[] {
  const segments = [projectName];
  if (!ctx.issue) return segments;
  if (ctx.column) segments.push(ctx.column.name);
  const { ref } = issueRefParts(ctx.issue.title, ctx.issue.externalRef);
  if (ref) segments.push(ref);
  return segments;
}

/** Which header actions are meaningful right now. Each field is the
 * action's whole visibility condition — a null/false field renders
 * nothing at all rather than a disabled control. */
export interface PipelineActions {
  /** The step settled in a column that HOLDS and there is somewhere to go:
   * show `advance`, targeting this column. */
  advanceTo: BoardColumnDto | null;
  /** A step is parked here awaiting its confirm: show `dispatch`. */
  confirmColumn: BoardColumnDto | null;
  /** Any carded task can be moved. */
  canMove: boolean;
  /** Any carded task has a detail sheet. */
  canOpenCard: boolean;
}

const NO_ACTIONS: PipelineActions = {
  advanceTo: null,
  confirmColumn: null,
  canMove: false,
  canOpenCard: false,
};

export function pipelineActions(ctx: TaskCardContext): PipelineActions {
  if (!ctx.issue) return NO_ACTIONS;
  // `settledColumnId` is only ever written for a column that holds — the
  // engine advances the others itself (step_engine.rs), so the flag IS
  // the "awaiting a human" state. It must still match the card's CURRENT
  // column: a settle the user has already walked away from is stale.
  const settledHere =
    ctx.column !== null && ctx.flags?.settledColumnId === ctx.column.id;
  const advanceTo =
    settledHere && ctx.column ? advanceTarget(ctx.column, ctx.columns) : null;
  const parkedId = ctx.flags?.queuedColumnId ?? null;
  return {
    advanceTo,
    confirmColumn: parkedId
      ? (ctx.columns.find((c) => c.id === parkedId) ?? null)
      : null,
    canMove: ctx.columns.length > 0,
    canOpenCard: true,
  };
}

// -- runners -----------------------------------------------------------------
//
// All three moves go through `issue_enter_column`, the engine's ONE
// primitive: entering a run-mode step dispatches, entering a queue-mode
// step parks, entering a shelf or gate is inert — the same semantics as a
// board drag, because it is the same call. None of them stops an agent
// (ADR-0037 item 11: the board never kills).

/** Enter a column. Returns the outcome so the caller can chain a parked
 * step into its confirm, exactly as the board's drop does.
 *
 * Gated by first-dispatch consent (#74, §8e) for the same reason the
 * board's drop is: this is the same `issue_enter_column` call, so entering
 * an agent step from the task view provisions a worktree and writes to the
 * repo exactly as a drag does. Without the gate, a user who drives the
 * pipeline from here is never asked and the dossier feature stays inert
 * forever — fail-closed, silent, and undiscoverable. A withdrawn ask
 * (project switched) means nothing happened, which is `inert`. */
export async function enterColumn(
  issue: IssueDto,
  column: BoardColumnDto,
): Promise<"launched" | "queued" | "reattached" | "inert"> {
  if (column.kind === "agent_step" && !(await ensureDossierConsent(issue.projectId, issue))) {
    return "inert";
  }
  if (!(await ensureShipped(issue, column))) return "inert";
  if (column.kind === "agent_step") useSteps.getState().beginDispatch(issue.id);
  try {
    const outcome = await issueEnterColumn(issue.id, column.id);
    if (outcome.step === "queued") useSteps.getState().endLaunch(issue.id);
    maybeOfferWorktreeCleanup(issue, column);
    return outcome.step;
  } catch (e) {
    useSteps.getState().endLaunch(issue.id);
    throw e;
  }
}

/** Fire the parked step. The park is consumed backend-side; the launch it
 * produces arrives as `step:launch` like every other launch, so nothing
 * here opens a terminal. The local flag is dropped first for the same
 * reason the board drops it — the queued vocabulary is over the moment
 * the confirm fires. */
export async function confirmParkedStep(issue: IssueDto): Promise<void> {
  useSteps.getState().clearPark(issue.id);
  useSteps.getState().beginDispatch(issue.id);
  try {
    await stepConfirm(issue.id);
  } catch (e) {
    useSteps.getState().endLaunch(issue.id);
    throw e;
  }
}

/** THE ship gate: entering a ship column merges FIRST — squash-merge the
 * linked task's branch into the project root + push (`task_ship`), the
 * same way the consent gate runs before the enter. A clean worktree ships
 * inline (throws on conflict — the card never enters, which IS the
 * bounce-back to Review); a dirty worktree opens the commit-or-cancel
 * dialog and the dialog finishes the interrupted move. Cards without a
 * linked worktree task (or project-root tasks) move like a plain shelf. */
export async function ensureShipped(issue: IssueDto, column: BoardColumnDto): Promise<boolean> {
  if (column.kind !== "ship" || !issue.linkedTaskId) return true;
  const sb = useSidebar.getState();
  const task = (sb.tasksByProject[issue.projectId] ?? []).find(
    (t) => t.id === issue.linkedTaskId,
  );
  const project = sb.projects.find((p) => p.id === issue.projectId);
  if (!task?.workspaceId || task.workspaceId === project?.repositoryWorkspaceId) {
    return true;
  }
  let dirty = false;
  try {
    const s = await gitStatus(task.workspaceId);
    dirty = s.staged.length > 0 || s.unstaged.length > 0 || s.truncated;
  } catch {
    // Status unreadable — let the ship command be the judge.
  }
  if (dirty) {
    useUi.getState().setShipPrompt({ issue, column, taskId: task.id });
    return false;
  }
  await taskShip(task.id);
  return true;
}

/** Landing in Done (seedLane "done") with a linked task opens the
 * delete-task/worktree confirm — cleanup is offered at the moment work
 * finishes instead of waiting for a manual delete. Esc/keep leaves the
 * card in Done with its worktree intact; delete tears the task and
 * worktree down (the card survives — linked_task_id nulls via FK) so the
 * card can rest in Closed. Shared by every entry surface: board drag,
 * card-detail advance, and the task header's advance/move (both funnel
 * through enterColumn above). */
export function maybeOfferWorktreeCleanup(issue: IssueDto, column: BoardColumnDto): void {
  if (column.seedLane !== "done" || !issue.linkedTaskId) return;
  useUi
    .getState()
    .setDeleteTaskTarget({ projectId: issue.projectId, taskId: issue.linkedTaskId });
}

// -- the four header actions -------------------------------------------------
//
// One implementation each, shared by the keyboard (registry commands) and
// the mouse (the header's key-labelled buttons) — a button that did its
// own thing would be a second contract. Every one of them reads the
// current context first and returns silently when the task has no card,
// which is how ⌘N ad-hoc tasks stay inert.

/** Advance a settled step to `advance_to ?? next-by-position`.
 *
 * Deliberately NOT gated by a confirm: the destination column's own
 * `on_enter` is the spend decision (run dispatches, queue parks behind
 * its own confirm), which is precisely what dragging a held card one
 * column right does on the board. The button carries the board's
 * confirm-free-spend brightness so the difference is visible before the
 * press. */
export function runAdvanceStep(): void {
  const ctx = currentTaskCardContext();
  const target = pipelineActions(ctx).advanceTo;
  if (!ctx.issue || !target) return;
  const card = useTaskCard.getState();
  card.setError(null);
  void enterColumn(ctx.issue, target)
    .then((step) => {
      // Advancing into a queue-mode step parks it — carry straight on to
      // the confirm rather than leaving the user to notice a new button.
      if (step === "queued") card.setOverlay("confirm");
    })
    .catch((e) => card.setError(String(e)));
}

/** Open the confirm for a parked step. Opening, not firing: the overlay
 * NAMES the provider · model · effort the step will spend, and ↵ there is
 * what dispatches (`runFireParkedStep`). */
export function runConfirmStep(): void {
  if (!pipelineActions(currentTaskCardContext()).confirmColumn) return;
  const card = useTaskCard.getState();
  card.setError(null);
  card.setOverlay("confirm");
}

/** The confirm overlay's ↵. */
export function runFireParkedStep(): void {
  const ctx = currentTaskCardContext();
  if (!ctx.issue || !pipelineActions(ctx).confirmColumn) return;
  const issue = ctx.issue;
  const card = useTaskCard.getState();
  card.setOverlay(null);
  void confirmParkedStep(issue).catch((e) => card.setError(String(e)));
}

/** Open the key-first column picker. */
export function runMoveToColumn(): void {
  if (!pipelineActions(currentTaskCardContext()).canMove) return;
  const card = useTaskCard.getState();
  card.setError(null);
  card.setOverlay("move");
}

/** The picker's ↵ — the same call a board drag makes, so a run-mode step
 * dispatches and a queue-mode step parks. */
export function runMoveTo(column: BoardColumnDto): void {
  const ctx = currentTaskCardContext();
  if (!ctx.issue) return;
  const card = useTaskCard.getState();
  card.setError(null);
  void enterColumn(ctx.issue, column)
    .then((step) => card.setOverlay(step === "queued" ? "confirm" : null))
    .catch((e) => {
      card.setOverlay(null);
      card.setError(String(e));
    });
}

/** Route to the card's detail sheet.
 *
 * The detail lives in the ONE right-panel slot, and ChangesSidebar gives
 * that slot to Changes/chat whenever a task is selected — so reading the
 * ticket means leaving the task view, exactly as clicking the card on the
 * board does. Nothing is torn down: ⌘⌥↑/↓, the flyout, or the card itself
 * walks straight back. */
export function runOpenCardDetail(): void {
  const ctx = currentTaskCardContext();
  if (!ctx.issue) return;
  const issue = ctx.issue;
  useTaskCard.getState().setOverlay(null);
  const ui = useUi.getState();
  ui.setBoardDetailIssueId(issue.id);
  ui.setChangesOpen(true);
  useSidebar.getState().selectProject(issue.projectId);
}
