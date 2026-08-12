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
  issueList,
  issueMove,
  onFartcodeEvent,
  stepConfirm,
  type BoardColumnDto,
  type IssueDto,
  type TaskDto,
} from "../../lib/tauri";
import {
  blockerColumnName,
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
import { hydrateParkedSteps, useSteps } from "../../store/steps";
import { useUi } from "../../store/ui";
import {
  agentLive,
  blockerLabel,
  elapsedShort,
  issueRefParts,
  runStateFor,
} from "./runState";
import { ensureDossierConsent, useDossierConsent } from "../../store/dossierConsent";

/** §8b / DESIGN.md Layout: below this WINDOW width the board is one column
 * plus the mono strip. Deliberately not the board pane's own width — see
 * the resize effect. */
const NARROW_PX = 900;

export function isNarrowViewport(): boolean {
  return typeof window !== "undefined" && window.innerWidth < NARROW_PX;
}

/** Stable empty arrays — a fresh literal in a zustand selector re-renders
 * forever. */
const NO_COLUMNS: BoardColumnDto[] = [];
const NO_TASKS: TaskDto[] = [];

/** Provider chip label: the display name minus any "CLI"/"-cli" suffix
 * ("Claude Code CLI" → "Claude Code"). */
const providerLabel = (provider: string) => provider.replace(/\s*[-\s]?CLI$/i, "");

/** A gated move awaiting the user's confirm (§5g, templated per §8c from
 * column config). `blocked` and `live-agent` have NOT moved the card yet —
 * esc keeps it where it is, and the footer says so. `queued` is different
 * in kind: the engine writes the move BEFORE it parks the step
 * (step_engine.rs — an errored move must leave the park untouched), so by
 * the time the confirm exists the card has already arrived. Its footer
 * must therefore promise what esc actually does, which is leave the step
 * parked, not put the card back. */
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
  /** §8e consent, owned app-level (store/dossierConsent.ts). The board
   * only needs two facts: whether a card is on screen (it must not fight
   * it for ↵/esc) and whether this project has settled (the park-reconcile
   * path must not raise its confirm in front of the question). */
  const consentAsk = useDossierConsent((s) => s.ask);
  const consentError = useDossierConsent((s) => s.error);
  const consentByProject = useDossierConsent((s) => s.byProject);
  const consentSettled = consentByProject[projectId] !== undefined;
  /** Keyboard focus (frame 4b "focused"): roving, by issue id. */
  const [focusId, setFocusId] = useState<string | null>(null);
  /** Column focus is its OWN cursor: h/l walks every column, including
   * empty ones, and narrow mode renders whichever one holds it. */
  const [focusColumnId, setFocusColumnId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [creating, setCreating] = useState(false);
  const [narrow, setNarrow] = useState(() => isNarrowViewport());
  // Step state lives in an app-lifetime store, NOT here: carrying out a
  // launch navigates to the task view and unmounts this component, so
  // component state would be wiped by the very act it is tracking.
  const steps = useSteps((s) => s.byIssue);
  const stepError = useSteps((s) => s.error);
  /** Park whose confirm the user dismissed — the park itself lives on. */
  const [dismissedPark, setDismissedPark] = useState<string | null>(null);
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
    // Parks live backend-side in memory; a webview reload lost them until
    // this re-seed (E18-09). The park-reconcile effect below then raises
    // the queued dot / confirm overlay exactly as if the event arrived.
    void hydrateParkedSteps(projectId);
    if (useDependencies.getState().deps.length === 0) {
      void useDependencies.getState().load();
    }
  }, [projectId]);

  // A project switch does NOT remount this component (App.tsx renders
  // ProjectView without a key), so every cross-project pointer has to be
  // dropped by hand. `pending` is the one that bites: a queue confirm
  // raised for project A stays on screen over project B's board, and its
  // ↵ would fire A's parked step from B. The consent card has the same
  // shape and is cancelled app-level (App.tsx).
  useEffect(() => {
    setPending(null);
    setDismissedPark(null);
  }, [projectId]);

  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    const reload = () =>
      issueList(projectId)
        .then((list) => {
          if (cancelled) return;
          setIssues(list);
          setLoaded(true);
        })
        .catch((e) => !cancelled && setError(String(e)));
    void reload();
    // No GitHub autorun here (lib/github-sync.ts): the import runs once,
    // when the project is added. Re-importing on every board mount turned
    // board↔task navigation into a launch generator.
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
      // Step engine (E18-04/05): the DIRECTIVE and the flags belong to the
      // app-lifetime store (store/steps.ts) — see the note by `steps`
      // above. The board only needs the card list refreshed, since a
      // launch or an advance moves the card.
      if (
        (ev.type === "step:launch" || ev.type === "step:settled") &&
        ev.projectId === projectId
      ) {
        void reload();
      }
    });
    return () => {
      cancelled = true;
      void unlisten.then((off) => off());
    };
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

  // §8b narrow mode follows the WINDOW, not the board pane. Measuring the
  // pane looked more precise and was wrong: the rail (56) + flyout (244) +
  // the 400px card-detail sheet come off the same width, so clicking a
  // card on a 14" laptop dropped a perfectly wide board into single-column
  // mode and took cross-column drag away mid-gesture. DESIGN.md's Layout
  // rule is window-scoped ("Under ~900px the board collapses to one column
  // and the rail narrows to 48px"), and a board too narrow for its columns
  // already has the right answer: .board-frame scrolls sideways.
  useEffect(() => {
    const onResize = () => setNarrow(isNarrowViewport());
    onResize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
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

  /** THE move: enter a column and let the engine decide (run / queue /
   * nothing) from that column's config.
   *
   * The outcome's `launch` is deliberately IGNORED. Every launch also
   * emits `step:launch`, which the app-lifetime store carries out, so
   * acting on both would need a dedupe — and the wall-clock dedupe that
   * used to do it swallowed genuine second dispatches inside its window.
   * One channel, no window.
   *
   * The `queued` discriminator IS used: the command already knows the step
   * parked and returns the moved issue, so the confirm never has to race
   * the card into the refetched list.
   */
  const enter = async (issue: IssueDto, column: BoardColumnDto, position: number | null) => {
    try {
      const outcome = await issueEnterColumn(issue.id, column.id, position ?? undefined);
      if (outcome.step === "queued") {
        setPending({
          kind: "queued",
          issue: outcome.issue,
          column,
          from: columnOf(issue),
          position: null,
        });
      }
    } catch (e) {
      setError(String(e));
    }
  };

  /** THE gate (§8e). Every entry into an `agent_step` funnels through
   * `enter`, so asking here covers the whole board — the task view's two
   * paths await the same gate from their own surfaces. Shelves and human
   * gates never ask: they spend nothing and write nothing.
   *
   * It sits BEFORE `issueEnterColumn`, not after, for two reasons that
   * both matter: the backend reads consent at launch time (so an answer
   * arriving later would miss its own dispatch), and the queue confirm is
   * MADE by that call — gating afterwards would put consent second.
   *
   * A false result means the ask was withdrawn (project switched), not
   * that consent was refused — a refusal still dispatches. */
  const enterGated = async (
    issue: IssueDto,
    column: BoardColumnDto,
    position: number | null,
  ) => {
    if (column.kind === "agent_step" && !(await ensureDossierConsent(projectId, issue))) {
      return;
    }
    await enter(issue, column, position);
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
    void enterGated(issue, column, position);
  };

  const confirmPending = () => {
    if (!pending) return;
    const { kind, issue, column, position } = pending;
    setPending(null);
    if (kind === "queued") {
      // The park is consumed by the backend; the launch it produces
      // arrives as `step:launch` like every other launch.
      useSteps.getState().clearPark(issue.id);
      stepConfirm(issue.id).catch((e) => setError(String(e)));
      return;
    }
    void enterGated(issue, column, position);
  };

  const dismissPending = () => {
    if (pending?.kind === "queued") {
      // The card already moved and the backend park SURVIVES — esc means
      // "leave it parked", which is what the footer now says. Keeping the
      // flag is what preserves the dashed queued ring and lets the confirm
      // be reopened, so it is deliberately not cleared here.
      setDismissedPark(pending.issue.id);
    }
    setPending(null);
  };

  // Reconcile parks the UI did not raise itself: a settle that advanced
  // into a queue-mode step, or the engine re-parking after a restart.
  // `issues` IS a dep — a park announced before the card reached the list
  // used to be dropped on the floor — and a dismissed park is remembered
  // by id so the overlay does not immediately reopen.
  useEffect(() => {
    if (pending || consentAsk) return;
    for (const [issueId, flags] of Object.entries(steps)) {
      if (!flags.queuedColumnId || dismissedPark === issueId) continue;
      const issue = issues.find((i) => i.id === issueId);
      const column = columns.find((c) => c.id === flags.queuedColumnId);
      if (!issue || !column) continue;
      // §8e holds here too: a park the board did NOT raise (a settle that
      // chained into a queue-mode step, or a rehydrated one after a
      // reload) is still an agent_step entry, and consent comes before the
      // dispatch confirm wherever the confirm comes from.
      //
      // It resolves through the SAME gate the drag path awaits, so this
      // branch cannot trust a cache the drag path would have re-read —
      // and, because the gate only settles once the consent write has
      // committed, the confirm below can never become interactive ahead of
      // the write. Nothing is deferred: the gate settling updates the
      // store, which re-runs this effect, which then raises the confirm it
      // was standing in front of.
      if (!consentSettled) {
        void ensureDossierConsent(projectId, issue);
        return;
      }
      setPending({ kind: "queued", issue, column, from: columnOf(issue), position: null });
      return;
    }
    // columnOf is derived from columns/issues, both already deps.
  }, [steps, columns, issues, pending, consentAsk, consentSettled, projectId, dismissedPark]);

  // A park that goes away (confirmed, superseded, card dragged out) frees
  // its dismissal, so the next park on that card asks again.
  useEffect(() => {
    if (dismissedPark && !steps[dismissedPark]?.queuedColumnId) setDismissedPark(null);
  }, [steps, dismissedPark]);

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
      // The consent card is in front of everything and owns ↵/esc itself
      // (DossierConsentCard). Stand down completely rather than racing it.
      if (consentAsk) return;
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

      // THE COLUMN CURSOR is what h/l walks, and it is always well defined
      // — the focused card's column when there is one, otherwise the
      // column cursor itself. Deriving direction from the CARD was the
      // bug: landing on an empty column nulls the card, and the next press
      // then had nothing to step from, so it restarted the scan and
      // teleported to the leftmost non-empty column. §8b says h/l walks
      // EVERY column, so the walk must never depend on a card existing.
      const curColumn = cur ? columnOf(cur) : columnById(focusColumnId);
      const colIdx = Math.max(
        0,
        columns.findIndex((c) => c.id === curColumn?.id),
      );
      const list = cardsIn(columns[colIdx]?.id ?? null);
      const idx = cur ? list.findIndex((i) => i.id === cur.id) : -1;

      /** Steps the cursor one column and takes the nearest card with it
       * (none when the column is empty — an empty column is a real stop). */
      const walk = (dir: -1 | 1): void => {
        const next = colIdx + dir;
        if (next < 0 || next >= columns.length) return;
        const target = columns[next];
        setFocusColumnId(target.id);
        const cand = cardsIn(target.id);
        setFocusId(
          cand.length > 0 ? cand[Math.min(Math.max(idx, 0), cand.length - 1)].id : null,
        );
      };

      if (!e.shiftKey) {
        if (key === "h" || key === "l") {
          walk(key === "h" ? -1 : 1);
          return;
        }
        // j/k with no focused card: take the first card of the column the
        // cursor is on. On an empty column there is nothing to take, and
        // j/k no-op rather than teleporting.
        if (!cur) {
          if (list.length > 0) setFocusId(list[key === "j" ? 0 : list.length - 1].id);
          return;
        }
        if (key === "j" && idx < list.length - 1) setFocusId(list[idx + 1].id);
        else if (key === "k" && idx > 0) setFocusId(list[idx - 1].id);
        return;
      }
      // ⇧: move the card itself. With no card focused there is nothing to
      // move, but the cursor still walks so the board stays navigable.
      if (!cur) {
        if (key === "h" || key === "l") walk(key === "h" ? -1 : 1);
        return;
      }
      // Within-column positions use the after-removal convention (see
      // handleDrop).
      if (key === "j" && idx < list.length - 1) void reorder(cur, idx + 1);
      else if (key === "k" && idx > 0) void reorder(cur, idx - 1);
      else if (key === "h" || key === "l") {
        const next = colIdx + (key === "h" ? -1 : 1);
        if (next < 0 || next >= columns.length) return;
        const target = columns[next];
        setFocusColumnId(target.id);
        requestMove(cur, target, Math.min(Math.max(idx, 0), cardsIn(target.id).length));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    issues,
    columns,
    byColumn,
    focusId,
    focusColumnId,
    pending,
    consentAsk,
    agentByTask,
    projectId,
  ]);

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
      if (landing) await enterGated(created, landing, null);
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
              holdReason={
                steps[issue.id]?.heldColumnId === column.id
                  ? (steps[issue.id]?.holdReason ?? null)
                  : null
              }
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
  // A launch directive that could not be carried out (agent binary gone,
  // PTY refused) fails in the app-lifetime store — surface it here rather
  // than letting the card sit looking dispatched with nothing running.
  // A failed consent write has no dialog to report to — the card is gone
  // by then (see DossierConsentCard) — so it lands here, beside the other
  // "the app tried and could not" messages.
  const shown = error ?? stepError ?? columnsError ?? consentError;

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
            <div className="board-columns">
              {visibleColumns.map((column) => (
                <section
                  key={column.id}
                  className="board-column"
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
          <div className="board-columns">
            {columns.map((column) => (
              <section
                key={column.id}
                className="board-column"
                data-done={column.countsAsDone ? "" : undefined}
              >
                <div className="board-lane-head">
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
                {renderCards(column)}
              </section>
            ))}
          </div>
        </div>
      )}

      {/* §8e: consent → then the dispatch confirm. The card itself renders
          app-level (App.tsx); the board just stays out of its way. */}
      {!consentAsk && pending && (
        <ConfirmOverlay
          pending={pending}
          branch={pendingBranch}
          columns={columns}
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
  columns,
  summary,
  onKeep,
  onGo,
}: {
  pending: PendingConfirm;
  branch: string | null;
  columns: BoardColumnDto[];
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
  // "esc keep in <column>" is only honest for the confirms raised BEFORE
  // the move. A queued step is raised after it (the engine writes the move,
  // then parks), so esc leaves the card exactly where it now is and the
  // step parked — which is what the footer must say.
  let keepLabel = `esc keep in ${from?.name ?? column.name}`;
  if (pending.kind === "blocked") {
    // Finished-ness is the column's counts_as_done flag, never a name —
    // and the copy names each blocker's OWN column (§8c binding copy:
    // "#a is blocked by #b, still in <blocker's column>"). One shared
    // column reads as a single "still in X" tail; blockers spread across
    // columns get per-blocker parentheticals, so the line never lies.
    const active = issue.blockers.filter((b) => !b.countsAsDone);
    const blockerColumns = active.map((b) => blockerColumnName(b, columns));
    const oneColumn = new Set(blockerColumns).size === 1;
    body = (
      <>
        {self} is blocked by{" "}
        {active.map((b, i) => (
          <Fragment key={b.id}>
            {i > 0 && ", "}
            <span className="board-confirm-ref">{blockerLabel(b.title)}</span>
            {!oneColumn && <> ({blockerColumns[i]})</>}
          </Fragment>
        ))}
        {oneColumn && <>, still in {blockerColumns[0]}</>}. Send to {column.name}{" "}
        anyway?
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
    keepLabel = "esc leave parked";
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
            {keepLabel}
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
/** Human copy for a chain-guard hold reason (#82). */
function holdReasonCopy(reason: string): string {
  switch (reason) {
    case "depth":
      return "auto-run limit";
    case "cycle":
      return "loop detected";
    case "budget":
      return "budget spent";
    default:
      return reason;
  }
}

function BoardCard({
  issue,
  task,
  agent,
  stepDone,
  queued,
  holdReason,
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
  /** #82 chain-guard hold reason for THIS column, null when not held. */
  holdReason: string | null;
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
  if (holdReason) {
    // #82: the chain guard refused the next automatic launch — say why,
    // in the meta line's failure voice (nearest pattern: the step-done
    // hint line; frames pending per the ticket's design gate).
    segs.push(
      <span key="held" className="board-meta-bad">
        held · {holdReasonCopy(holdReason)}
      </span>,
    );
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
