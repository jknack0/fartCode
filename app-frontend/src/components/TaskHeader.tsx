// Task header (design_handoff_v2 5a, extended for the pipeline by
// ADR-0037): the 46px terminal-first header row — breadcrumb + title +
// agent dot on the left; lifecycle script launchers, the pipeline actions
// and the Changes toggle on the right in mono 11px, then the utilities
// (move / card / files / new task / delete) as drawn icons — name and
// chord live in each hover title, so the row stays readable beside the
// 400px right panel.
//
// PIPELINE CONTEXT (ADR-0037). A card's step settles in a `hold` column,
// or parks behind a queue-mode confirm — and the person who has to act is
// almost always here, watching the agent, not on the board. So this header
// carries both the card's position (the crumb reads `project / <column> /
// <ref>`) and its decision points (advance / dispatch / move / card), each
// shown only when it means something. A ⌘N ad-hoc task has no card and no
// pipeline: its crumb and its actions row are byte-identical to what they
// were before this existed.
//
// Every rule is borrowed, never restated — the card's column comes from
// the board's `columnIdForIssue`, the advance target from `advanceTarget`
// (`advance_to ?? next-by-position`, the settle engine's own rule), and
// the spend copy from `columnConfigSummary`. See lib/taskPipeline.ts.

import { useEffect, useState } from "react";
import { getProjectSettings } from "../lib/tauri";
import { openFileTree, openLifecycleScript } from "../lib/commands";
import { columnSublineTone } from "../lib/columnConfig";
import {
  pipelineActions,
  pipelineCrumb,
  runAdvanceStep,
  runConfirmStep,
  runMoveToColumn,
  runOpenCardDetail,
  taskCardContext,
} from "../lib/taskPipeline";
import { hint, runCommand } from "../lib/useCommands";
import { useColumns } from "../store/columns";
import { defaultAgentName, useDependencies } from "../store/dependencies";
import { SCRIPT_TYPES, useScripts, type ScriptType } from "../store/scripts";
import { useSidebar } from "../store/sidebar";
import { useSteps } from "../store/steps";
import { useTaskCard } from "../store/taskCard";
import { useUi } from "../store/ui";
import TaskPipelineOverlay from "./TaskPipelineOverlay";
import { IconBranch, IconCard, IconColumns, IconFolder, IconPlus, IconTrash } from "./icons";
import type { BoardColumnDto, IssueDto } from "../lib/tauri";


/** Stable empty arrays — a fresh literal in a zustand selector re-renders
 * forever (the board learned this the hard way). */
const NO_COLUMNS: BoardColumnDto[] = [];
const NO_ISSUES: IssueDto[] = [];

export default function TaskHeader({ taskId }: { taskId: string }) {
  const projectId = useSidebar((s) => s.selectedProjectId);
  const projectName = useSidebar(
    (s) => s.projects.find((p) => p.id === s.selectedProjectId)?.name ?? null,
  );
  const task = useSidebar((s) =>
    s.selectedProjectId
      ? ((s.tasksByProject[s.selectedProjectId] ?? []).find((t) => t.id === taskId) ?? null)
      : null,
  );
  const runs = useScripts((s) => s.byTask[taskId]);
  const agentRunning = useScripts((s) => s.agentByTask[taskId]?.running ?? false);
  // A dispatch in flight for THIS task (drag → prompt pasted) — shows the
  // "starting…" label until the directive finishes.
  const launching = useSteps((s) =>
    Object.values(s.launchingByIssue).some((l) => l.taskId === taskId),
  );
  const columns = useColumns((s) =>
    projectId ? (s.byProject[projectId] ?? NO_COLUMNS) : NO_COLUMNS,
  );
  const issues = useTaskCard((s) =>
    projectId ? (s.issuesByProject[projectId] ?? NO_ISSUES) : NO_ISSUES,
  );
  const stepFlags = useSteps((s) => s.byIssue);
  const overlay = useTaskCard((s) => s.overlay);
  const pipelineError = useTaskCard((s) => s.error);
  const defaultAgent = useDependencies((s) => defaultAgentName(s.deps));
  // Re-render hint text when keybindings change.
  useUi((s) => s.bindingsVersion);

  // The board's shape and its cards, both needed before a crumb or an
  // action can be resolved. Cheap, cached per project, and event-refreshed
  // — the same load the board does, minus the board.
  useEffect(() => {
    if (!projectId) return;
    void useColumns.getState().load(projectId);
    void useTaskCard.getState().load(projectId);
    if (useDependencies.getState().deps.length === 0) {
      void useDependencies.getState().load();
    }
  }, [projectId]);

  // An overlay belongs to the task it was opened on; switching tasks (or
  // projects) must not leave a picker pointing at the previous card.
  useEffect(() => {
    return () => useTaskCard.getState().setOverlay(null);
  }, [taskId]);


  // Scripts configured on the project (E1-06) — only configured scripts get
  // a launcher. Cheap DB read, refetched per project.
  const [configured, setConfigured] = useState<ScriptType[]>([]);
  useEffect(() => {
    if (!projectId) return;
    let live = true;
    getProjectSettings(projectId)
      .then((s) => {
        if (!live) return;
        setConfigured(SCRIPT_TYPES.filter((k) => Boolean(s.scripts?.[k]?.trim())));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [projectId]);

  const ctx = taskCardContext({ taskId, issues, columns, steps: stepFlags });
  const actions = pipelineActions(ctx);
  const crumb = pipelineCrumb(projectName ?? "project", ctx);

  // AGENT DOT — the LIVE session, never `task.status`.
  //
  // `TaskStatus` has no production writer: `TaskStore::update_status` is
  // called from nothing, so a task's status is frozen at `in_progress`
  // from birth (ADR-0037 consequences; the audit that found it is the
  // reason board cards derive run-state the same way). Keying the dot on
  // it therefore renders a fixed answer, not a state. The truth is a live
  // agent terminal, which the scripts store tracks — hydrate() snapshots
  // it, noteAgentSpawn() covers agents this frontend just opened, and the
  // terminal-exit event flips it off.
  //
  // This is RUNNING vs IDLE only. It does NOT fix "needs you": that state
  // means the agent stopped mid-turn to ask something, which nothing in
  // the app can observe until the agent hook server exists (unbuilt). The
  // `review` branch below is the shape that state will take when a writer
  // finally appears; today it cannot fire, and no reading of this dot
  // should be taken as "the agent is not waiting on you".
  //
  // Note this deliberately does NOT use runState.ts's `agentLive`, whose
  // pre-hydration fallback is `status === "in_progress"`. That fallback
  // exists so a COLD BOARD full of un-hydrated cards reads as it always
  // did; here there is exactly one task and TaskView hydrates it on
  // mount, so the same fallback would only ever paint a false amber for
  // one frame on a task with no agent at all.
  const dotClass = agentRunning
    ? "status-dot status-in_progress"
    : launching
      ? "status-dot status-launching"
      : task?.status === "review"
        ? "status-dot status-needs-you"
        : "status-dot tv-dot-idle";

  return (
    <header className="tv-header">
      <div className="tv-header-id">
        {/* `project /` on an ad-hoc task; `project / In Progress / #47 /`
            on a carded one. A card with no derivable ref contributes no
            segment rather than an empty separator. */}
        <span className="tv-crumb">{crumb.join(" / ")} /</span>
        <span className="tv-title" title={task?.name ?? undefined}>
          {task?.name ?? "Task"}
        </span>
        <span className={dotClass} />
        {launching && <span className="tv-launching">starting…</span>}
      </div>
      <div className="tv-header-actions">
        {pipelineError && (
          <button
            type="button"
            className="tv-action failed tv-pipeline-error"
            title={pipelineError}
            onClick={() => useTaskCard.getState().setError(null)}
          >
            {pipelineError}
          </button>
        )}
        {configured.map((k) => {
          const run = runs?.[k];
          const running = run?.running ?? false;
          const failed =
            !running && run?.exitCode !== null && run?.exitCode !== undefined && run.exitCode !== 0;
          const ok = !running && run?.exitCode === 0;
          return (
            <button
              key={k}
              type="button"
              className={`tv-action${running ? " running" : ""}${failed ? " failed" : ""}`}
              title={`${k} script — logs in the drawer (${hint("toggle-drawer") || "⌘J"})`}
              onClick={() => openLifecycleScript(taskId, k)}
            >
              {k}
              {ok ? " ✓" : failed ? " ✗" : ""}
            </button>
          );
        })}
        {actions.advanceTo && (
          // Confirm-free spend is brighter: an advance landing on an
          // `on_enter: run` step dispatches on the press, and says so by
          // reading brighter — the board headers' own tone rule.
          <button
            type="button"
            className="tv-action"
            data-tone={columnSublineTone(actions.advanceTo)}
            title={`Advance to ${actions.advanceTo.name} (${hint("advance-step") || "⌘⇧→"})`}
            onClick={() => runAdvanceStep()}
          >
            advance
          </button>
        )}
        {actions.confirmColumn && (
          <button
            type="button"
            className="tv-action"
            title={`Dispatch the step parked in ${actions.confirmColumn.name} (${hint("confirm-step") || "⌘⇧D"})`}
            onClick={() => runConfirmStep()}
          >
            dispatch
          </button>
        )}
        <button
          type="button"
          className="tv-action tv-action-icon"
          title={`Toggle changes panel (${hint("toggle-changes") || "⌘⇧1"})`}
          onClick={() => runCommand("toggle-changes")}
        >
          <IconBranch />
        </button>
        {/* The utilities ride inline as drawn icons — the label and the
            chord live in the hover title. move / card appear only when the
            task has a card; new task is the only mouse path once ⌘\ hides
            the flyout; delete stays last and red-on-hover — its confirm,
            which itemizes the agent, worktree and comments, is still the
            only thing that can destroy. */}
        {actions.canMove && (
          <button
            type="button"
            className="tv-action tv-action-icon"
            title={`Move this card to another column (${hint("move-to-column") || "⌘⇧M"})`}
            onClick={() => runMoveToColumn()}
          >
            <IconColumns />
          </button>
        )}
        {actions.canOpenCard && (
          <button
            type="button"
            className="tv-action tv-action-icon"
            title={`Open the card detail (${hint("open-card-detail") || "⌘⇧I"})`}
            onClick={() => runOpenCardDetail()}
          >
            <IconCard />
          </button>
        )}
        <button
          type="button"
          className="tv-action tv-action-icon"
          title="Open the file tree"
          onClick={() => openFileTree(taskId)}
        >
          <IconFolder />
        </button>
        <button
          type="button"
          className="tv-action tv-action-icon"
          title={`Add a task to this project (${hint("add-task") || "⌘N"})`}
          onClick={() => runCommand("add-task")}
        >
          <IconPlus />
        </button>
        <button
          type="button"
          className="tv-action tv-action-icon tv-action-danger"
          title={`Delete this task and its worktree (${hint("delete-task") || "⌘⌫"})`}
          onClick={() => runCommand("delete-task")}
        >
          <IconTrash />
        </button>
      </div>
      {overlay && ctx.issue && (
        <TaskPipelineOverlay mode={overlay} ctx={ctx} defaultAgent={defaultAgent} />
      )}
    </header>
  );
}
