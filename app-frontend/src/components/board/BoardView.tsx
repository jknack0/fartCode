// Issue board (E17-02 #56, generalized by E18-07 #66): the project view's
// primary surface. ONE plate of N hairline-ruled COLUMNS, rendered from
// board_columns in position order (ADR-0037) — there is no lane list here,
// no five, and nothing keyed on a column's name. Every column carries its
// own semantics as data: `kind` decides the header subline, `on_enter`
// decides whether a drop runs an agent or parks it behind a confirm,
// `on_settle` decides whether a settled step holds or advances,
// `counts_as_done` dims the column, `is_landing` takes new work.
//
// Moves go through the step engine's ONE primitive, `issue_enter_column`
// (E18-04/05), which runs/queues/does nothing per the target column and
// hands back a launch payload; `step:launch` carries the same directive for
// launches the engine chained on its own. Within-column reorder stays on
// `issue_move` — the enter primitive is a step trigger, and re-entering an
// agent step reattaches, which is not what dragging a card up the list
// means. The board NEVER kills an agent (ADR-0037 item 11): a move into a
// terminal column confirms and then moves; the agent keeps running.
//
// Card run-state derives from the LIVE SESSION (runState.ts), never from
// the card's column. Blockedness derives from `countsAsDone`, never from a
// lane string. All state reconciles by refetching on issue/step events —
// one move can flip OTHER cards' blocked badges.
//
// Keyboard (frame 4b + v3 §8b): j/k move card focus, h/l walk EVERY column
// (empty ones included), ⇧+those move the card through the same gates, ↵
// opens (a failed card's ↵ reads the linked task), a adds an issue to the
// landing column. Under 900px the board collapses to the §8b narrow mode:
// a scrolling mono strip of every column, the focused column alone below
// it, and the strip following focus.

import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import {
  gitCommitState,
  issueCreate,
  issueEnterColumn,
  issueImportGithub,
  issueList,
  issueMove,
  onFartcodeEvent,
  projectGithubIssues,
  stepConfirm,
  terminalOpenAgent,
  terminalWrite,
  type BoardColumnDto,
  type IssueDto,
  type StepLaunchInfoDto,
  type TaskDto,
} from "../../lib/tauri";
import {
  columnConfigSummary,
  columnIdForIssue,
  columnSublineTone,
  groupByColumn,
  landingColumn,
  stepArtifact,
} from "../../lib/columnConfig";
import { useColumns } from "../../store/columns";
import { defaultAgentName, useDependencies } from "../../store/dependencies";
import { useScripts } from "../../store/scripts";
import { useSidebar } from "../../store/sidebar";
import { useUi } from "../../store/ui";
import {
  agentLive,
  blockerLabel,
  elapsedShort,
  issueRefParts,
  runStateFor,
} from "./runState";

// GitHub issue sync autoruns on view entry (formerly the header's manual
// "Sync from GitHub" button): import every open issue not already on the
// board (deduped by URL); the IssueCreated events then drive the board's
// own reload. Failures stay quiet — the board works fine offline/local.
// ponytail: 60s cooldown per project, in-memory only — remount re-syncs.
const lastIssueSyncAt: Record<string, number> = {};
const ISSUE_SYNC_COOLDOWN_MS = 60_000;

/** §8b: below this the board is one column plus the mono strip. */
const NARROW_PX = 900;

/** Stable empty arrays — a fresh literal in a zustand selector re-renders
 * forever. */
const NO_COLUMNS: BoardColumnDto[] = [];
const NO_TASKS: TaskDto[] = [];

async function syncGithubIssues(projectId: string): Promise<void> {
  const [ghIssues, board] = await Promise.all([
    projectGithubIssues(projectId),
    issueList(projectId),
  ]);
  const imported = new Set(
    board.filter((i) => i.externalRef).map((i) => i.externalRef),
  );
  for (const g of ghIssues.filter((x) => !imported.has(x.url))) {
    await issueImportGithub({
      projectId,
      number: g.number,
      title: g.title,
      url: g.url,
      body: g.body,
      labels: g.labels,
      assignees: g.assignees,
      milestone: g.milestone,
    });
  }
}

// A launch reaches the frontend TWICE — once as the command's
// `EnterOutcomeDto.launch`, once as the `step:launch` event — and the
// backend's delivered flag does not ride the event. Claim it per
// issue+column so the pair collapses to one opened session while a CHAIN
// (settle advancing into another agent step) still opens its own.
const lastLaunchAt = new Map<string, number>();
const LAUNCH_DEDUPE_MS = 4_000;

function claimLaunch(issueId: string, columnId: string): boolean {
  const key = `${issueId}:${columnId}`;
  const now = Date.now();
  const prev = lastLaunchAt.get(key);
  if (prev !== undefined && now - prev < LAUNCH_DEDUPE_MS) return false;
  lastLaunchAt.set(key, now);
  return true;
}

/** Provider chip label: the display name minus any "CLI"/"-cli" suffix
 * ("Claude Code CLI" → "Claude Code"). */
const providerLabel = (provider: string) => provider.replace(/\s*[-\s]?CLI$/i, "");

/** A gated move awaiting the user's confirm (§5g, templated per §8c from
 * column config). `blocked` and `live-agent` have NOT moved the card yet —
 * esc keeps it where it is; `queued` HAS moved it (the engine parked the
 * step on entry) and esc simply dismisses. */
type PendingConfirm = {
  kind: "blocked" | "live-agent" | "queued";
  issue: IssueDto;
  /** The move's target column — the one whose name fills the copy. */
  column: BoardColumnDto;
  /** Where the card sits now, for "esc keep in <column>". */
  from: BoardColumnDto | null;
  position: number | null;
};

/** Insertion point during a drag — rendered as a 1px accent line between
 * cards, never a ghost box (frame 4b). */
interface DropTarget {
  columnId: string;
  index: number;
}

/** Per-issue engine state, fed by the step events. Nothing is stored
 * backend-side (step-done is derived state by doctrine), so this lives for
 * the session only. */
interface StepFlags {
  /** step:settled landed for this column and it holds. */
  settledColumnId?: string;
  /** step:queued parked a step here, awaiting the confirm. */
  queuedColumnId?: string;
}

const isEditableTarget = (t: EventTarget | null): boolean =>
  t instanceof HTMLElement &&
  (t.tagName === "INPUT" ||
    t.tagName === "TEXTAREA" ||
    t.tagName === "SELECT" ||
    t.isContentEditable);

export default function BoardView({ projectId }: { projectId: string }) {
  const [issues, setIssues] = useState<IssueDto[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [over, setOver] = useState<DropTarget | null>(null);
  const [pending, setPending] = useState<PendingConfirm | null>(null);
  /** Linked task's branch for the confirm footer (fetched lazily). */
  const [pendingBranch, setPendingBranch] = useState<string | null>(null);
  /** Keyboard focus (frame 4b "focused"): roving, by issue id. */
  const [focusId, setFocusId] = useState<string | null>(null);
  /** Column focus is its OWN cursor: h/l walks every column, including
   * empty ones, and narrow mode renders whichever one holds it. */
  const [focusColumnId, setFocusColumnId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [creating, setCreating] = useState(false);
  const [steps, setSteps] = useState<Record<string, StepFlags>>({});
  const [narrow, setNarrow] = useState(false);
  const detailIssueId = useUi((s) => s.boardDetailIssueId);
  const projectTasks = useSidebar((s) => s.tasksByProject[projectId]) ?? NO_TASKS;
  const agentByTask = useScripts((s) => s.agentByTask);
  const columns = useColumns((s) => s.byProject[projectId] ?? NO_COLUMNS);
  const columnsLoaded = useColumns((s) => s.loaded[projectId] ?? false);
  const columnsError = useColumns((s) => s.error);
  const defaultAgent = useDependencies((s) => defaultAgentName(s.deps));
  const boardRef = useRef<HTMLDivElement | null>(null);
  const stripRef = useRef<HTMLDivElement | null>(null);
  /** Latest issues for event handlers, which close over their mount. */
  const issuesRef = useRef<IssueDto[]>([]);
  issuesRef.current = issues;

  // Columns are the board's shape — load before anything is drawn. The
  // default agent names step columns whose provider is unpinned (§8a
  // subline), so make sure the dependency cache is warm.
  useEffect(() => {
    void useColumns.getState().load(projectId);
    if (useDependencies.getState().deps.length === 0) {
      void useDependencies.getState().load();
    }
  }, [projectId]);

  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    setSteps({});
    const reload = () =>
      issueList(projectId)
        .then((list) => {
          if (cancelled) return;
          setIssues(list);
          setLoaded(true);
        })
        .catch((e) => !cancelled && setError(String(e)));
    void reload();
    // Autorun the GitHub import (cooldown-gated; imports emit IssueCreated,
    // which the listener below turns into a reload).
    const now = Date.now();
    if (now - (lastIssueSyncAt[projectId] ?? 0) >= ISSUE_SYNC_COOLDOWN_MS) {
      lastIssueSyncAt[projectId] = now;
      syncGithubIssues(projectId).catch((e) =>
        console.warn("github issue sync failed:", e),
      );
    }
    const unlisten = onFartcodeEvent((ev) => {
      if (
        (ev.type === "issue:created" ||
          ev.type === "issue:updated" ||
          ev.type === "issue:deleted") &&
        ev.projectId === projectId
      ) {
        void reload();
      }
      // Task lifecycle affects cards: deletion unlinks (SET NULL), status
      // changes recolor the linked-task dot.
      if (ev.type === "task:deleted" || ev.type === "task:status_changed") {
        void reload();
      }
      // -- step engine (E18-04/05) ---------------------------------------
      if (
        ev.type !== "step:launch" &&
        ev.type !== "step:queued" &&
        ev.type !== "step:queue_cleared" &&
        ev.type !== "step:settled"
      ) {
        return;
      }
      if (ev.projectId !== projectId) return;
      if (ev.type === "step:launch") {
        // A fresh launch clears both derived states, then opens (or
        // focuses) the session — this is a DIRECTIVE, and a settle-chained
        // launch arrives here and nowhere else.
        setSteps((s) => ({ ...s, [ev.issueId]: {} }));
        void openLaunchedSession(ev.issueId, ev.columnId, {
          taskId: ev.taskId,
          prompt: ev.prompt,
          provider: ev.provider,
          reattached: ev.reattached,
        });
        void reload();
      } else if (ev.type === "step:queued") {
        setSteps((s) => ({ ...s, [ev.issueId]: { queuedColumnId: ev.columnId } }));
      } else if (ev.type === "step:queue_cleared") {
        setSteps((s) => {
          const flags = s[ev.issueId];
          if (flags?.queuedColumnId !== ev.columnId) return s;
          return { ...s, [ev.issueId]: { ...flags, queuedColumnId: undefined } };
        });
      } else if (ev.type === "step:settled") {
        // Hold leaves the card put with the step-done dot; advance moved
        // it already — the reload is what shows that.
        setSteps((s) => ({ ...s, [ev.issueId]: { settledColumnId: ev.columnId } }));
        void useScripts.getState().hydrate(ev.taskId);
        void reload();
      }
    });
    return () => {
      cancelled = true;
      void unlisten.then((off) => off());
    };
    // projectId alone on purpose: the handler reaches for the launch
    // opener, which is rebuilt every render — listing it would tear down
    // and re-subscribe the event channel on each one.
  }, [projectId]);

  // Run-state is the live agent terminal, not task.status (ADR-0037 / the
  // TaskHeader-dot finding) — hydrate every linked task once so the board
  // knows which sessions are actually alive.
  useEffect(() => {
    const known = useScripts.getState().agentByTask;
    for (const id of new Set(
      issues.map((i) => i.linkedTaskId).filter((id): id is string => Boolean(id)),
    )) {
      if (!known[id]) void useScripts.getState().hydrate(id);
    }
  }, [issues]);

  // §8b: one column plus the strip under 900px. Measured, not a media
  // query — the board shares the pane with the changes/chat sheet.
  useEffect(() => {
    const el = boardRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width ?? el.clientWidth;
      setNarrow(w > 0 && w < NARROW_PX);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Elapsed meta ("· 4m") is derived from statusChangedAt, never stored —
  // refresh on a slow tick (the display is minute-coarse).
  const [, setTick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);

  const byColumn = useMemo(() => groupByColumn(issues, columns), [issues, columns]);
  const cardsIn = (columnId: string | null): IssueDto[] =>
    (columnId && byColumn.get(columnId)) || [];
  const columnById = (id: string | null): BoardColumnDto | null =>
    columns.find((c) => c.id === id) ?? null;
  const columnOf = (issue: IssueDto): BoardColumnDto | null =>
    columnById(columnIdForIssue(issue, columns));

  // Keep the column cursor on a real column — first load, a deleted
  // column, or a project switch all land here.
  useEffect(() => {
    if (columns.length === 0) {
      if (focusColumnId !== null) setFocusColumnId(null);
      return;
    }
    if (!columns.some((c) => c.id === focusColumnId)) {
      setFocusColumnId(landingColumn(columns)?.id ?? columns[0].id);
    }
  }, [columns, focusColumnId]);

  // Drop stale keyboard focus when its card leaves the board.
  useEffect(() => {
    if (focusId && !issues.some((i) => i.id === focusId)) setFocusId(null);
  }, [issues, focusId]);

  useEffect(() => {
    if (!focusId) return;
    document
      .querySelector(`.board-card[data-issue-id="${focusId}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [focusId, issues]);

  // §8b "the strip auto-scrolls to keep focus visible" — instant, never a
  // smooth scroll (the app has exactly two keyframes).
  useEffect(() => {
    if (!narrow || !focusColumnId) return;
    stripRef.current
      ?.querySelector(`[data-column-id="${focusColumnId}"]`)
      ?.scrollIntoView({ inline: "nearest", block: "nearest" });
  }, [narrow, focusColumnId, columns]);

  const linkedTask = (issue: IssueDto): TaskDto | undefined =>
    issue.linkedTaskId
      ? projectTasks.find((t) => t.id === issue.linkedTaskId)
      : undefined;

  /** Does this card have a live agent right now? The scripts store is the
   * authority once hydrated; before that the legacy status test stands in. */
  const hasLiveAgent = (issue: IssueDto): boolean => {
    const task = linkedTask(issue);
    if (!task) return false;
    return agentLive(agentByTask[task.id], task.status);
  };

  /** Focuses the card's linked task (reattach never spawns a second
   * worktree — ADR-0032; the engine dedupes on linked_task_id). */
  const focusLinkedTask = (taskId: string) => {
    const task = projectTasks.find((t) => t.id === taskId);
    if (task) useSidebar.getState().switchToTask(task);
    else useSidebar.getState().selectTask(taskId);
  };

  /** Acts on a launch directive: reattach focuses the task, a real launch
   * opens the agent terminal and bracket-pastes the step's prompt. Shared
   * by the command outcome and the `step:launch` event, deduped by
   * issue+column. */
  const openLaunchedSession = async (
    issueId: string,
    columnId: string,
    launch: { taskId: string; prompt: string; provider: string; reattached: boolean },
  ) => {
    if (!claimLaunch(issueId, columnId)) return;
    try {
      if (launch.reattached) {
        focusLinkedTask(launch.taskId);
        return;
      }
      const terminalId = await terminalOpenAgent(launch.taskId, launch.provider, 24, 80);
      useScripts.getState().noteAgentSpawn(launch.taskId, terminalId);
      if (launch.prompt) {
        await terminalWrite(terminalId, `\u001b[200~${launch.prompt}\u001b[201~\r`);
      }
      focusLinkedTask(launch.taskId);
    } catch (e) {
      setError(String(e));
    }
  };

  const fromOutcome = (issueId: string, columnId: string, launch: StepLaunchInfoDto) =>
    openLaunchedSession(issueId, columnId, {
      taskId: launch.task.id,
      prompt: launch.prompt,
      provider: launch.provider,
      reattached: launch.reattached,
    });

  /** THE move: enter a column and let the engine decide (run / queue /
   * nothing) from that column's config. */
  const enter = async (issue: IssueDto, column: BoardColumnDto, position: number | null) => {
    try {
      const outcome = await issueEnterColumn(issue.id, column.id, position ?? undefined);
      if (outcome.launch) await fromOutcome(issue.id, column.id, outcome.launch);
    } catch (e) {
      setError(String(e));
    }
  };

  /** Within-column reorder — position only. Deliberately NOT the enter
   * primitive: re-entering an agent step reattaches its session, which is
   * not what dragging a card up its own column means. */
  const reorder = (issue: IssueDto, position: number) =>
    issueMove(issue.id, issue.lane, position).catch((e) => setError(String(e)));

  /** Cross-column move with the §5g/§8c gates. The card does not move
   * until the confirm resolves — esc keeps it where it is. */
  const requestMove = (issue: IssueDto, column: BoardColumnDto, position: number) => {
    const from = columnOf(issue);
    if (from?.id === column.id) {
      void reorder(issue, position);
      return;
    }
    // Blocked work entering a step: confirm, never a hard stop (ADR-0032).
    if (column.kind === "agent_step" && issue.blocked) {
      setPending({ kind: "blocked", issue, column, from, position });
      return;
    }
    // Live agent into a terminal column: the board never kills, so this
    // asks and then moves — the agent keeps running either way.
    if (column.countsAsDone && hasLiveAgent(issue)) {
      setPending({ kind: "live-agent", issue, column, from, position });
      return;
    }
    void enter(issue, column, position);
  };

  const confirmPending = () => {
    if (!pending) return;
    const { kind, issue, column, position } = pending;
    setPending(null);
    if (kind === "queued") {
      setSteps((s) => ({ ...s, [issue.id]: {} }));
      stepConfirm(issue.id)
        .then((outcome) => {
          if (outcome.launch) void fromOutcome(issue.id, column.id, outcome.launch);
        })
        .catch((e) => setError(String(e)));
      return;
    }
    void enter(issue, column, position);
  };

  const dismissPending = () => {
    if (pending?.kind === "queued") {
      // The card already moved; only the overlay goes away. The backend
      // park survives — re-dragging the card re-parks and re-asks.
      const issueId = pending.issue.id;
      setSteps((s) => ({ ...s, [issueId]: {} }));
    }
    setPending(null);
  };

  // A parked step IS a pending confirm — surface it whichever way it was
  // parked (drag, keyboard, or the engine re-parking after a restart).
  useEffect(() => {
    if (pending) return;
    for (const [issueId, flags] of Object.entries(steps)) {
      if (!flags.queuedColumnId) continue;
      const issue = issuesRef.current.find((i) => i.id === issueId);
      const column = columns.find((c) => c.id === flags.queuedColumnId);
      if (!issue || !column) continue;
      setPending({ kind: "queued", issue, column, from: columnOf(issue), position: null });
      return;
    }
    // Issues come from a ref, not a dep: a park is announced by the step
    // event, and re-running this on every board refetch would re-open a
    // confirm the user just dismissed.
  }, [steps, columns, pending]);

  // Confirm footer branch: only a linked task has one ("… on <branch>";
  // a fresh dispatch generates its branch server-side, so omit).
  useEffect(() => {
    setPendingBranch(null);
    if (!pending || pending.kind === "live-agent") return;
    const task = linkedTask(pending.issue);
    if (!task?.workspaceId) return;
    let cancelled = false;
    gitCommitState(task.workspaceId)
      .then((s) => {
        if (!cancelled && s.branch) setPendingBranch(s.branch);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pending]);

  const openCard = (issue: IssueDto) => {
    setFocusId(issue.id);
    const col = columnOf(issue);
    if (col) setFocusColumnId(col.id);
    const ui = useUi.getState();
    ui.setBoardDetailIssueId(issue.id);
    // The detail swaps into the right sheet — make sure it's visible
    // regardless of changes/chat.
    ui.setChangesOpen(true);
  };

  // Board keyboard (frame 4b + §8b): j/k cards, h/l EVERY column, ⇧ moves
  // the card, ↵ opens (reads on a failed card), a adds an issue. The
  // confirm overlay swallows ↵/esc while open.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      if (pending) {
        if (e.key === "Enter") {
          e.preventDefault();
          confirmPending();
        } else if (e.key === "Escape") {
          e.preventDefault();
          dismissPending();
        }
        return;
      }
      if (useUi.getState().modalOpen()) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;

      const cur = focusId ? (issues.find((i) => i.id === focusId) ?? null) : null;
      if (e.key === "Enter") {
        // A DOM-focused card handles its own Enter — don't double-open.
        if (e.target instanceof HTMLElement && e.target.closest(".board-card")) return;
        if (cur) {
          e.preventDefault();
          // A failed card advertises "↵ read" — Enter goes to the linked
          // task, not the detail sheet (frame 4a).
          const t = linkedTask(cur);
          const rs = runStateFor({
            status: t?.status,
            agent: t ? agentByTask[t.id] : undefined,
            stepDone: false,
            queued: false,
          });
          if (t && rs.actionable) focusLinkedTask(t.id);
          else openCard(cur);
        }
        return;
      }
      if (e.key === "a") {
        e.preventDefault();
        setAdding(true);
        return;
      }
      const key = e.key.toLowerCase();
      if (key !== "j" && key !== "k" && key !== "h" && key !== "l") return;
      if (columns.length === 0) return;
      e.preventDefault();

      // No card focused yet: land on the focused column's first card, else
      // the first card anywhere.
      if (!cur) {
        const here = cardsIn(focusColumnId)[0];
        const first = here ?? columns.map((c) => cardsIn(c.id)).find((l) => l.length > 0)?.[0];
        if (first) {
          setFocusId(first.id);
          const col = columnOf(first);
          if (col) setFocusColumnId(col.id);
        }
        return;
      }
      const curColumn = columnOf(cur);
      const colIdx = columns.findIndex((c) => c.id === curColumn?.id);
      const list = cardsIn(curColumn?.id ?? null);
      const idx = list.findIndex((i) => i.id === cur.id);

      if (!e.shiftKey) {
        if (key === "j" && idx < list.length - 1) setFocusId(list[idx + 1].id);
        else if (key === "k" && idx > 0) setFocusId(list[idx - 1].id);
        else if (key === "h" || key === "l") {
          // §8b: h/l walks EVERY column — an empty one is a real stop (it
          // is a drop target, and skipping it hides columns from the
          // keyboard entirely on a sparse board).
          const next = colIdx + (key === "h" ? -1 : 1);
          if (next < 0 || next >= columns.length) return;
          const target = columns[next];
          setFocusColumnId(target.id);
          const cand = cardsIn(target.id);
          setFocusId(cand.length > 0 ? cand[Math.min(idx, cand.length - 1)].id : null);
        }
        return;
      }
      // ⇧: move the card itself. Within-column positions use the
      // after-removal convention (see handleDrop).
      if (key === "j" && idx < list.length - 1) void reorder(cur, idx + 1);
      else if (key === "k" && idx > 0) void reorder(cur, idx - 1);
      else if (key === "h" || key === "l") {
        const next = colIdx + (key === "h" ? -1 : 1);
        if (next < 0 || next >= columns.length) return;
        const target = columns[next];
        setFocusColumnId(target.id);
        requestMove(cur, target, Math.min(idx, cardsIn(target.id).length));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issues, columns, byColumn, focusId, focusColumnId, pending, agentByTask, projectId]);

  /** Index among the column's cards where the cursor is (midpoint rule). */
  const dropIndex = (clientY: number, listEl: HTMLElement): number => {
    const cards = Array.from(listEl.querySelectorAll<HTMLElement>(".board-card"));
    for (let i = 0; i < cards.length; i++) {
      const r = cards[i].getBoundingClientRect();
      if (clientY < r.top + r.height / 2) return i;
    }
    return cards.length;
  };

  const handleDrop = (e: React.DragEvent, column: BoardColumnDto) => {
    e.preventDefault();
    setDragId(null);
    setOver(null);
    const issueId = e.dataTransfer.getData("text/fartCode-issue");
    const issue = issues.find((i) => i.id === issueId);
    if (!issue) return;
    const position = dropIndex(e.clientY, e.currentTarget as HTMLElement);

    if (columnOf(issue)?.id === column.id) {
      // Within-column reorder: removing the card shifts later indices down.
      const siblings = cardsIn(column.id);
      const from = siblings.findIndex((i) => i.id === issueId);
      const to = position > from ? position - 1 : position;
      if (to === from) return; // dropped back on itself
      void reorder(issue, to);
      return;
    }
    requestMove(issue, column, position);
  };

  const submitNew = async () => {
    const title = newTitle.trim();
    if (!title || creating) return;
    setCreating(true);
    try {
      const created = await issueCreate({ projectId, title });
      // ADR-0037 item 7: new work lands in the `is_landing` column,
      // whichever it is — routed through the enter primitive so a landing
      // column that IS a step behaves like every other entry into it.
      const landing = landingColumn(columns);
      if (landing) await enter(created, landing, null);
      const ui = useUi.getState();
      ui.setBoardDetailIssueId(created.id);
      ui.setChangesOpen(true);
      setNewTitle("");
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  };

  const total = issues.length;
  const landing = landingColumn(columns);
  const focusedColumn = columnById(focusColumnId);
  /** Columns rendered right now: all of them, or just the focused one. */
  const visibleColumns = narrow
    ? focusedColumn
      ? [focusedColumn]
      : columns.slice(0, 1)
    : columns;
  /** Empty-state copy names the first step column instead of a lane. */
  const firstStep = columns.find((c) => c.kind === "agent_step");

  const summaryOf = (column: BoardColumnDto) =>
    columnConfigSummary(column, { columns, defaultAgent });

  const columnHasLiveAgent = (columnId: string): boolean =>
    cardsIn(columnId).some((i) => hasLiveAgent(i));

  const renderCards = (column: BoardColumnDto) => {
    const cards = cardsIn(column.id);
    const artifact = stepArtifact(column);
    return (
      <div
        className="board-lane-cards"
        onDragOver={(e) => {
          if (!dragId) return;
          e.preventDefault();
          const index = dropIndex(e.clientY, e.currentTarget as HTMLElement);
          setOver((o) =>
            o && o.columnId === column.id && o.index === index
              ? o
              : { columnId: column.id, index },
          );
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
            setOver((o) => (o?.columnId === column.id ? null : o));
          }
        }}
        onDrop={(e) => handleDrop(e, column)}
      >
        {cards.map((issue, i) => (
          <Fragment key={issue.id}>
            {over?.columnId === column.id && over.index === i && dragId !== issue.id && (
              <div className="board-drop-line" />
            )}
            <BoardCard
              issue={issue}
              task={linkedTask(issue)}
              agent={
                issue.linkedTaskId ? agentByTask[issue.linkedTaskId] : undefined
              }
              stepDone={steps[issue.id]?.settledColumnId === column.id}
              queued={steps[issue.id]?.queuedColumnId === column.id}
              artifact={artifact}
              selected={detailIssueId === issue.id}
              focused={focusId === issue.id}
              dragging={dragId === issue.id}
              onDragStart={(e) => {
                e.dataTransfer.setData("text/fartCode-issue", issue.id);
                e.dataTransfer.effectAllowed = "move";
                setDragId(issue.id);
              }}
              onDragEnd={() => {
                setDragId(null);
                setOver(null);
              }}
              onOpen={() => openCard(issue)}
              onOpenIssue={(otherId) => {
                const other = issues.find((x) => x.id === otherId);
                if (other) openCard(other);
              }}
              onReadTask={(taskId) => focusLinkedTask(taskId)}
            />
          </Fragment>
        ))}
        {over?.columnId === column.id && over.index === cards.length && (
          <div className="board-drop-line" />
        )}
        {cards.length === 0 && <div className="board-lane-placeholder" />}
      </div>
    );
  };

  // A column read that failed must SAY so — the shape of the board comes
  // from it, so a silent failure would leave "Reading the board…" forever.
  const shown = error ?? columnsError;

  return (
    <div className="board" ref={boardRef}>
      {shown && <p className="error board-error">{shown}</p>}

      {adding && (
        <div className="board-new-card">
          <input
            autoFocus
            className="board-new-input"
            value={newTitle}
            placeholder="Issue title"
            disabled={creating}
            onChange={(e) => setNewTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void submitNew();
              if (e.key === "Escape") {
                setAdding(false);
                setNewTitle("");
              }
            }}
          />
          <span className="board-new-keys">
            <span className="board-key">↵</span> add ·{" "}
            <span className="board-key">esc</span> cancel
          </span>
        </div>
      )}

      {(!loaded || !columnsLoaded) && !shown ? (
        <div className="board-empty muted">Reading the board…</div>
      ) : columns.length === 0 ? (
        <div className="board-empty">
          <p className="muted">This project has no columns.</p>
        </div>
      ) : total === 0 && !adding ? (
        <div className="board-empty">
          <p className="muted">The board is empty.</p>
          <p className="muted">
            Pull work onto it — the GitHub key above imports every open issue,
            or add a card by hand.
            {firstStep
              ? ` Dragging one into ${firstStep.name} dispatches an agent in its own worktree.`
              : ""}
          </p>
          <button className="board-empty-add" onClick={() => setAdding(true)}>
            <span className="board-key">a</span> add issue
          </button>
        </div>
      ) : narrow ? (
        // §8b narrow: the mono strip walks every column, the focused one
        // renders below it, and the spend subline survives under the strip.
        <div className="board-narrow">
          <div className="board-strip-wrap">
            <div className="board-strip" ref={stripRef}>
              {columns.map((c) => (
                <button
                  key={c.id}
                  type="button"
                  className="board-strip-entry"
                  data-column-id={c.id}
                  data-active={c.id === focusColumnId ? "" : undefined}
                  data-working={columnHasLiveAgent(c.id) ? "" : undefined}
                  onClick={() => {
                    setFocusColumnId(c.id);
                    setFocusId(cardsIn(c.id)[0]?.id ?? null);
                  }}
                >
                  {c.name.toLowerCase()}{" "}
                  <span className="board-strip-count">{cardsIn(c.id).length}</span>
                </button>
              ))}
            </div>
          </div>
          {focusedColumn && (
            <div className="board-strip-sub" data-tone={columnSublineTone(focusedColumn)}>
              {summaryOf(focusedColumn)}
            </div>
          )}
          <div className="board-frame" style={{ ["--column-count" as string]: 1 }}>
            <div className="board-lanes">
              {visibleColumns.map((column) => (
                <section
                  key={column.id}
                  className="board-lane"
                  data-done={column.countsAsDone ? "" : undefined}
                >
                  {renderCards(column)}
                </section>
              ))}
            </div>
          </div>
          <div className="board-narrow-foot">
            <span className="board-key">h</span> <span className="board-key">l</span> walk
            every column · strip follows focus
          </div>
        </div>
      ) : (
        <div
          className="board-frame"
          style={{ ["--column-count" as string]: columns.length }}
        >
          <div className="board-lane-heads">
            {columns.map((column) => (
              <div
                key={column.id}
                className="board-lane-head"
                data-done={column.countsAsDone ? "" : undefined}
              >
                <div className="board-lane-name-row">
                  <span className="board-lane-name">{column.name}</span>
                  {column.isLanding && <span className="board-lane-landing">landing</span>}
                  <span className="board-lane-side">
                    {landing?.id === column.id && (
                      <button
                        className="project-action"
                        onClick={() => setAdding(true)}
                        title={`Add issue to ${column.name}`}
                        aria-label={`Add issue to ${column.name}`}
                      >
                        +
                      </button>
                    )}
                    <span className="board-lane-count">{cardsIn(column.id).length}</span>
                  </span>
                </div>
                <div className="board-lane-kind" data-tone={columnSublineTone(column)}>
                  {summaryOf(column)}
                </div>
              </div>
            ))}
          </div>
          <div className="board-lanes">
            {columns.map((column) => (
              <section
                key={column.id}
                className="board-lane"
                data-done={column.countsAsDone ? "" : undefined}
              >
                {renderCards(column)}
              </section>
            ))}
          </div>
        </div>
      )}

      {pending && (
        <ConfirmOverlay
          pending={pending}
          branch={pendingBranch}
          onKeep={dismissPending}
          onGo={confirmPending}
          summary={summaryOf(pending.column)}
        />
      )}
    </div>
  );
}

/** §5g confirm overlay — renders inside the board, key-first: esc keeps
 * the card where it is, ↵ proceeds. Never a hard stop. Every name in the
 * copy is a template slot filled from column config (§8c); #68 owns the
 * final wording. */
function ConfirmOverlay({
  pending,
  branch,
  summary,
  onKeep,
  onGo,
}: {
  pending: PendingConfirm;
  branch: string | null;
  summary: string;
  onKeep: () => void;
  onGo: () => void;
}) {
  const { issue, column, from } = pending;
  const { ref, title } = issueRefParts(issue.title, issue.externalRef);
  const self = ref ? (
    <span className="board-confirm-ref">{ref}</span>
  ) : (
    <>“{title.length > 48 ? `${title.slice(0, 48)}…` : title}”</>
  );

  let body: React.ReactNode;
  let goLabel: string;
  let label: string;
  if (pending.kind === "blocked") {
    // Finished-ness is the column's counts_as_done flag, never a name.
    const active = issue.blockers.filter((b) => !b.countsAsDone);
    body = (
      <>
        {self} is blocked by{" "}
        {active.map((b, i) => (
          <Fragment key={b.id}>
            {i > 0 && ", "}
            <span className="board-confirm-ref">{blockerLabel(b.title)}</span>
          </Fragment>
        ))}
        , still in progress. Send to {column.name} anyway?
      </>
    );
    // Name the agent: the column's pinned provider first, else the issue's,
    // else the app's default agent (the summary already carries it).
    const agentName = column.stepProvider
      ? providerLabel(column.stepProvider)
      : issue.provider
        ? providerLabel(issue.provider)
        : null;
    goLabel = `dispatch${agentName ? ` ${agentName}` : ""}${branch ? ` on ${branch}` : ""}`;
    label = "Dispatch blocked issue";
  } else if (pending.kind === "live-agent") {
    // The board never kills — this moves the card and leaves the agent be.
    body = <>{self} has a live agent. Move to {column.name} anyway?</>;
    goLabel = `move to ${column.name}`;
    label = `Move live task to ${column.name}`;
  } else {
    body = (
      <>
        {column.name} runs <span className="board-confirm-ref">{summary}</span> on {self}.
        Dispatch?
      </>
    );
    goLabel = `dispatch${branch ? ` on ${branch}` : ""}`;
    label = `Dispatch queued step in ${column.name}`;
  }

  return (
    <div className="board-confirm-backdrop" onClick={onKeep}>
      <div
        className="board-confirm"
        role="alertdialog"
        aria-label={label}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="board-confirm-body">{body}</div>
        <div className="board-confirm-foot">
          <button type="button" onClick={onKeep}>
            esc keep in {from?.name ?? column.name}
          </button>
          <button type="button" onClick={onGo}>
            <span className="board-confirm-key">↵</span> {goLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

/** One card (frames 4a/4b + v3 step-done): optional run-state dot, mono
 * meta line (ref · run state · elapsed · blocked by · gh · ac), then the
 * title. Hover reveals "↵ open" — except on a failed card, whose ↵ goes to
 * "↵ read" (the linked task). A settled step adds the accent dot and, when
 * the step declares an artifact, the "↵ read <artifact> · drag on" hint.
 * Blockedness is derived, never stored. */
function BoardCard({
  issue,
  task,
  agent,
  stepDone,
  queued,
  artifact,
  selected,
  focused,
  dragging,
  onDragStart,
  onDragEnd,
  onOpen,
  onOpenIssue,
  onReadTask,
}: {
  issue: IssueDto;
  task: TaskDto | undefined;
  agent: { running: boolean } | undefined;
  stepDone: boolean;
  queued: boolean;
  artifact: string | null;
  selected: boolean;
  focused: boolean;
  dragging: boolean;
  onDragStart: (e: React.DragEvent) => void;
  onDragEnd: () => void;
  onOpen: () => void;
  onOpenIssue: (issueId: string) => void;
  onReadTask: (taskId: string) => void;
}) {
  const rs = runStateFor({ status: task?.status, agent, stepDone, queued });
  const { ref, title } = issueRefParts(issue.title, issue.externalRef);
  // "Still blocking?" is the blocker column's counts_as_done flag (E18-03,
  // ADR-0037 item 6) — no lane name is consulted anywhere.
  const activeBlockers = issue.blockers.filter((b) => !b.countsAsDone);
  const showHint = rs.kind === "step-done" && Boolean(artifact);

  const segs: React.ReactNode[] = [];
  if (ref) segs.push(<span key="ref">{ref}</span>);
  if (task && rs.label) {
    segs.push(<span key="run">{rs.label}</span>);
    if (task.statusChangedAt) {
      segs.push(<span key="elapsed">{elapsedShort(task.statusChangedAt)}</span>);
    }
  }
  if (issue.blocked && activeBlockers.length > 0) {
    segs.push(
      <span key="blocked">
        blocked by{" "}
        {activeBlockers.map((b, i) => (
          <Fragment key={b.id}>
            {i > 0 && " "}
            <button
              type="button"
              className="board-blocked-ref"
              title={b.title}
              draggable={false}
              onClick={(e) => {
                e.stopPropagation();
                onOpenIssue(b.id);
              }}
            >
              {blockerLabel(b.title)}
            </button>
          </Fragment>
        ))}
      </span>,
    );
  }
  if (issue.externalRef) {
    segs.push(
      <button
        key="gh"
        type="button"
        className="board-gh-link"
        title={issue.externalRef}
        draggable={false}
        onClick={(e) => {
          e.stopPropagation();
          void open(issue.externalRef!).catch(() => {});
        }}
      >
        gh
      </button>,
    );
  }
  if (issue.acceptance.length > 0) {
    segs.push(<span key="ac">{issue.acceptance.length} ac</span>);
  }

  return (
    <article
      className={[
        "board-card",
        selected ? "selected" : "",
        focused ? "focused" : "",
        dragging ? "dragging" : "",
        issue.blocked || rs.dimTitle ? "dim-title" : "",
        rs.dimRow ? "dim-row" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      data-issue-id={issue.id}
      draggable
      tabIndex={0}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          // "↵ read" wins on an actionable (failed) card — frame 4a — and
          // on a step-done card with an artifact to read.
          if (task && (rs.actionable || showHint)) onReadTask(task.id);
          else onOpen();
        } else if (e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
    >
      {rs.kind !== "neutral" && (
        <span className={`status-dot board-run-dot ${rs.dot}`.trim()} />
      )}
      <span className="board-card-body">
        <span className={`board-card-meta${rs.bad ? " board-meta-bad" : ""}`}>
          <span className="board-card-meta-left">
            {segs.map((s, i) => (
              <Fragment key={i}>
                {i > 0 && <span className="board-meta-sep"> · </span>}
                {s}
              </Fragment>
            ))}
          </span>
          {!(task && rs.actionable) && (
            <span className="board-card-open">↵ open</span>
          )}
        </span>
        <span className="board-card-title">{title}</span>
        {task && rs.actionable && (
          <span className="board-card-action">
            <button
              type="button"
              draggable={false}
              onClick={(e) => {
                e.stopPropagation();
                onReadTask(task.id);
              }}
            >
              ↵ read
            </button>
          </span>
        )}
        {task && !rs.actionable && showHint && (
          <span className="board-card-action">
            <button
              type="button"
              draggable={false}
              onClick={(e) => {
                e.stopPropagation();
                onReadTask(task.id);
              }}
            >
              ↵ read {artifact}
            </button>
            <span className="board-card-hint-tail"> · drag on</span>
          </span>
        )}
      </span>
    </article>
  );
}
