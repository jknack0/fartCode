// Issue board (E17-02, #56): the project view's primary surface. One
// plate with five hairline-ruled lanes render from issue_list in board
// order; native HTML5 drag/drop persists moves via issue_move. Dispatch
// semantics (design_handoff_v2 §5g): dropping a blocked card into In
// Progress gates on an in-board confirm overlay (ADR-0032: confirm,
// never a hard stop), In progress → Done with a live agent warns the
// same way, and re-drag REATTACHES to the linked task — the dispatch
// path dedupes server-side (dispatch.rs), the board never kills an
// agent (teardown is ⌘⌫ only). Card click swaps the right region to the
// card detail. GitHub issues arrive as NATIVE cards via the autorun
// sync on view entry (see below). All state reconciles by refetching on
// issue events (blocked badges are derived — one move can flip OTHER
// cards). Keyboard (frame 4b): j/k move card focus, h/l move lanes,
// ⇧+those move the card through the same dispatch gate, ↵ opens (a
// failed card's ↵ reads the linked task instead), a adds an issue.

import { Fragment, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import {
  gitCommitState,
  issueCreate,
  issueDispatch,
  issueImportGithub,
  issueList,
  issueMove,
  onFartcodeEvent,
  projectGithubIssues,
  terminalOpenAgent,
  terminalWrite,
  type IssueDto,
  type Lane,
  type TaskDto,
} from "../../lib/tauri";
import { useDependencies } from "../../store/dependencies";
import { useSidebar } from "../../store/sidebar";
import { useUi } from "../../store/ui";
import { blockerLabel, elapsedShort, issueRefParts, runStateFor } from "./runState";

// GitHub issue sync autoruns on view entry (formerly the header's manual
// "Sync from GitHub" button): import every open issue not already on the
// board (deduped by URL); the IssueCreated events then drive the board's
// own reload. Failures stay quiet — the board works fine offline/local.
// ponytail: 60s cooldown per project, in-memory only — remount re-syncs.
const lastIssueSyncAt: Record<string, number> = {};
const ISSUE_SYNC_COOLDOWN_MS = 60_000;

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

const LANES: { id: Lane; label: string }[] = [
  { id: "backlog", label: "Backlog" },
  { id: "ready", label: "Ready" },
  { id: "in_progress", label: "In progress" },
  { id: "in_review", label: "In review" },
  { id: "done", label: "Done" },
];

export const LANE_LABEL: Record<Lane, string> = {
  backlog: "Backlog",
  ready: "Ready",
  in_progress: "In progress",
  in_review: "In review",
  done: "Done",
};

/** Provider chip label: the display name minus any "CLI"/"-cli" suffix
 * ("Claude Code CLI" → "Claude Code"). */
const providerLabel = (provider: string) => provider.replace(/\s*[-\s]?CLI$/i, "");

/** A gated move awaiting the user's confirm (§5g): dispatching a blocked
 * card, or moving a live-agent card to Done. The card has NOT moved yet —
 * esc simply keeps it where it is. */
type PendingConfirm =
  | { kind: "blocked"; issue: IssueDto; lane: Lane; position: number }
  | { kind: "done-live"; issue: IssueDto; lane: Lane; position: number };

/** Insertion point during a drag — rendered as a 1px accent line between
 * cards, never a ghost box (frame 4b). */
interface DropTarget {
  lane: Lane;
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
  /** Keyboard focus (frame 4b "focused"): roving, by issue id. */
  const [focusId, setFocusId] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [creating, setCreating] = useState(false);
  const detailIssueId = useUi((s) => s.boardDetailIssueId);
  const projectTasks = useSidebar((s) => s.tasksByProject[projectId]);

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
    });
    return () => {
      cancelled = true;
      void unlisten.then((off) => off());
    };
  }, [projectId]);

  // Elapsed meta ("· 4m") is derived from statusChangedAt, never stored —
  // refresh on a slow tick (the display is minute-coarse).
  const [, setTick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, []);

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

  const move = (issueId: string, lane: Lane, position: number) =>
    issueMove(issueId, lane, position).catch((e) => setError(String(e)));

  /** Focuses the card's linked task (reattach never spawns a second
   * worktree — ADR-0032; dispatch.rs dedupes on linked_task_id). */
  const focusLinkedTask = (taskId: string) => {
    const task = (useSidebar.getState().tasksByProject[projectId] ?? []).find(
      (t) => t.id === taskId,
    );
    if (task) useSidebar.getState().switchToTask(task);
  };

  /** E17-03: dispatch an unlinked card — backend creates the task, then
   * the agent terminal opens with the prompt packet bracket-pasted in. */
  const dispatch = async (issue: IssueDto) => {
    try {
      const outcome = await issueDispatch(issue.id);
      if (outcome.reattached) {
        useSidebar.getState().switchToTask(outcome.task);
        return;
      }
      const terminalId = await terminalOpenAgent(outcome.task.id, outcome.provider, 24, 80);
      await terminalWrite(terminalId, `\u001b[200~${outcome.prompt}\u001b[201~\r`);
      useSidebar.getState().switchToTask(outcome.task);
    } catch (e) {
      setError(String(e));
    }
  };

  const linkedTask = (issue: IssueDto): TaskDto | undefined =>
    issue.linkedTaskId
      ? (useSidebar.getState().tasksByProject[projectId] ?? []).find(
          (t) => t.id === issue.linkedTaskId,
        )
      : undefined;

  /** Cross-lane move with the §5g gates. The card does not move until
   * the confirm resolves — esc keeps it in its current lane. */
  const requestMove = (issue: IssueDto, lane: Lane, position: number) => {
    if (lane === "in_progress" && issue.lane !== "in_progress") {
      if (issue.blocked) {
        setPending({ kind: "blocked", issue, lane, position });
        return;
      }
      if (issue.linkedTaskId) {
        // Reattach: a live linked task gets a status move + focus, never
        // a second spawn (ADR-0032).
        void move(issue.id, lane, position);
        focusLinkedTask(issue.linkedTaskId);
        return;
      }
      void dispatch(issue);
      return;
    }
    if (
      lane === "done" &&
      issue.lane === "in_progress" &&
      linkedTask(issue)?.status === "in_progress"
    ) {
      setPending({ kind: "done-live", issue, lane, position });
      return;
    }
    void move(issue.id, lane, position);
  };

  const confirmPending = () => {
    if (!pending) return;
    const { kind, issue, lane, position } = pending;
    setPending(null);
    if (kind === "blocked") {
      if (issue.linkedTaskId) {
        void move(issue.id, lane, position);
        focusLinkedTask(issue.linkedTaskId);
      } else {
        void dispatch(issue);
      }
      return;
    }
    void move(issue.id, lane, position);
  };

  // Confirm footer branch: only a linked task has one ("… on <branch>";
  // a fresh dispatch generates its branch server-side, so omit).
  useEffect(() => {
    setPendingBranch(null);
    if (!pending || pending.kind !== "blocked") return;
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
    const ui = useUi.getState();
    ui.setBoardDetailIssueId(issue.id);
    // The detail swaps into the right sheet — make sure it's visible
    // regardless of changes/chat.
    ui.setChangesOpen(true);
  };

  // Board keyboard (frame 4b): j/k cards, h/l lanes, ⇧ moves the card,
  // ↵ opens (reads on a failed card), a adds an issue. The confirm
  // overlay swallows ↵/esc while open.
  useEffect(() => {
    const byLane = (lane: Lane) => issues.filter((i) => i.lane === lane);
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      if (pending) {
        if (e.key === "Enter") {
          e.preventDefault();
          confirmPending();
        } else if (e.key === "Escape") {
          e.preventDefault();
          setPending(null);
        }
        return;
      }
      if (useUi.getState().modalOpen()) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;

      const cur = focusId ? issues.find((i) => i.id === focusId) ?? null : null;
      if (e.key === "Enter") {
        // A DOM-focused card handles its own Enter — don't double-open.
        if (e.target instanceof HTMLElement && e.target.closest(".board-card")) return;
        if (cur) {
          e.preventDefault();
          // A failed card advertises "↵ read" — Enter goes to the linked
          // task, not the detail sheet (frame 4a).
          const t = linkedTask(cur);
          if (t && runStateFor(t.status).actionable) focusLinkedTask(t.id);
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
      e.preventDefault();

      if (!cur) {
        const first = LANES.map((l) => byLane(l.id)).find((list) => list.length > 0)?.[0];
        if (first) setFocusId(first.id);
        return;
      }
      const laneIdx = LANES.findIndex((l) => l.id === cur.lane);
      const list = byLane(cur.lane);
      const idx = list.findIndex((i) => i.id === cur.id);

      if (!e.shiftKey) {
        if (key === "j" && idx < list.length - 1) setFocusId(list[idx + 1].id);
        else if (key === "k" && idx > 0) setFocusId(list[idx - 1].id);
        else if (key === "h" || key === "l") {
          const dir = key === "h" ? -1 : 1;
          for (let li = laneIdx + dir; li >= 0 && li < LANES.length; li += dir) {
            const cand = byLane(LANES[li].id);
            if (cand.length > 0) {
              setFocusId(cand[Math.min(idx, cand.length - 1)].id);
              break;
            }
          }
        }
        return;
      }
      // ⇧: move the card itself. Within-lane positions use the
      // after-removal convention (see handleDrop).
      if (key === "j" && idx < list.length - 1) void move(cur.id, cur.lane, idx + 1);
      else if (key === "k" && idx > 0) void move(cur.id, cur.lane, idx - 1);
      else if (key === "h" || key === "l") {
        const li = laneIdx + (key === "h" ? -1 : 1);
        if (li >= 0 && li < LANES.length) {
          const target = LANES[li].id;
          requestMove(cur, target, Math.min(idx, byLane(target).length));
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [issues, focusId, pending, projectId]);

  /** Index among the lane's cards where the cursor is (midpoint rule). */
  const dropIndex = (clientY: number, listEl: HTMLElement): number => {
    const cards = Array.from(listEl.querySelectorAll<HTMLElement>(".board-card"));
    for (let i = 0; i < cards.length; i++) {
      const r = cards[i].getBoundingClientRect();
      if (clientY < r.top + r.height / 2) return i;
    }
    return cards.length;
  };

  const handleDrop = (e: React.DragEvent, lane: Lane) => {
    e.preventDefault();
    setDragId(null);
    setOver(null);
    const issueId = e.dataTransfer.getData("text/fartCode-issue");
    const issue = issues.find((i) => i.id === issueId);
    if (!issue) return;
    const position = dropIndex(e.clientY, e.currentTarget as HTMLElement);

    if (issue.lane === lane) {
      // Within-lane reorder: removing the card shifts later indices down.
      const siblings = issues.filter((i) => i.lane === lane);
      const from = siblings.findIndex((i) => i.id === issueId);
      const to = position > from ? position - 1 : position;
      if (to === from) return; // dropped back on itself
      void move(issueId, lane, to);
      return;
    }
    requestMove(issue, lane, position);
  };

  const submitNew = async () => {
    const title = newTitle.trim();
    if (!title || creating) return;
    setCreating(true);
    try {
      const created = await issueCreate({ projectId, title, lane: "backlog" });
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

  return (
    <div className="board">
      {error && <p className="error board-error">{error}</p>}

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

      {!loaded && !error ? (
        <div className="board-empty muted">Reading the board…</div>
      ) : total === 0 && !adding ? (
        <div className="board-empty">
          <p className="muted">The board is empty.</p>
          <p className="muted">
            Pull work onto it — the GitHub key above imports every open issue,
            or add a card by hand. Dragging one into In&nbsp;progress dispatches
            an agent in its own worktree.
          </p>
          <button className="board-empty-add" onClick={() => setAdding(true)}>
            <span className="board-key">a</span> add issue
          </button>
        </div>
      ) : (
        <div className="board-frame">
          <div className="board-lane-heads">
            {LANES.map(({ id, label }) => {
              const count = issues.filter((i) => i.lane === id).length;
              return (
                <div key={id} className="board-lane-head" data-lane={id}>
                  <span className="board-lane-name">{label}</span>
                  <span className="board-lane-side">
                    {id === "backlog" && (
                      <button
                        className="project-action"
                        onClick={() => setAdding(true)}
                        title="Add issue to Backlog"
                        aria-label="Add issue to Backlog"
                      >
                        +
                      </button>
                    )}
                    <span className="board-lane-count">{count}</span>
                  </span>
                </div>
              );
            })}
          </div>
          <div className="board-lanes">
            {LANES.map(({ id }) => {
              const cards = issues.filter((i) => i.lane === id);
              return (
                <section key={id} className="board-lane" data-lane={id}>
                  <div
                    className="board-lane-cards"
                    onDragOver={(e) => {
                      if (!dragId) return;
                      e.preventDefault();
                      const index = dropIndex(e.clientY, e.currentTarget as HTMLElement);
                      setOver((o) =>
                        o && o.lane === id && o.index === index ? o : { lane: id, index },
                      );
                    }}
                    onDragLeave={(e) => {
                      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
                        setOver((o) => (o?.lane === id ? null : o));
                      }
                    }}
                    onDrop={(e) => handleDrop(e, id)}
                  >
                    {cards.map((issue, i) => (
                      <Fragment key={issue.id}>
                        {over?.lane === id && over.index === i && dragId !== issue.id && (
                          <div className="board-drop-line" />
                        )}
                        <BoardCard
                          issue={issue}
                          tasks={projectTasks ?? []}
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
                    {over?.lane === id && over.index === cards.length && (
                      <div className="board-drop-line" />
                    )}
                    {cards.length === 0 && <div className="board-lane-placeholder" />}
                  </div>
                </section>
              );
            })}
          </div>
        </div>
      )}

      {pending && (
        <ConfirmOverlay
          pending={pending}
          branch={pendingBranch}
          onKeep={() => setPending(null)}
          onGo={confirmPending}
        />
      )}
    </div>
  );
}

/** §5g confirm overlay — renders inside the board, key-first: esc keeps
 * the card where it is, ↵ proceeds. Never a hard stop. */
function ConfirmOverlay({
  pending,
  branch,
  onKeep,
  onGo,
}: {
  pending: PendingConfirm;
  branch: string | null;
  onKeep: () => void;
  onGo: () => void;
}) {
  const { issue } = pending;
  const { ref, title } = issueRefParts(issue.title, issue.externalRef);
  const self = ref ? (
    <span className="board-confirm-ref">{ref}</span>
  ) : (
    <>“{title.length > 48 ? `${title.slice(0, 48)}…` : title}”</>
  );

  let body: React.ReactNode;
  let goLabel: string;
  if (pending.kind === "blocked") {
    const active = issue.blockers.filter((b) => b.lane !== "done");
    body = (
      <>
        {self} is blocked by{" "}
        {active.map((b, i) => (
          <Fragment key={b.id}>
            {i > 0 && ", "}
            <span className="board-confirm-ref">{blockerLabel(b.title)}</span>
          </Fragment>
        ))}
        , still in progress. Dispatch anyway?
      </>
    );
    // Name the agent: issue-level provider first, else the app's default
    // agent (dependencies store — loaded by settings/onboarding; a cold
    // cache degrades to plain "dispatch").
    const defaultAgent = useDependencies
      .getState()
      .deps.find((d) => d.isDefault)?.providerId;
    const agentName = issue.provider ? providerLabel(issue.provider) : defaultAgent;
    goLabel = `dispatch${agentName ? ` ${agentName}` : ""}${branch ? ` on ${branch}` : ""}`;
  } else {
    body = <>{self} has a live agent. Move to Done anyway?</>;
    goLabel = "move to Done";
  }

  return (
    <div className="board-confirm-backdrop" onClick={onKeep}>
      <div
        className="board-confirm"
        role="alertdialog"
        aria-label={pending.kind === "blocked" ? "Dispatch blocked issue" : "Move live task to Done"}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="board-confirm-body">{body}</div>
        <div className="board-confirm-foot">
          <button type="button" onClick={onKeep}>
            esc keep in {LANE_LABEL[issue.lane]}
          </button>
          <button type="button" onClick={onGo}>
            <span className="board-confirm-key">↵</span> {goLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

/** One card (frames 4a/4b): optional run-state dot, mono meta line
 * (ref · run state · elapsed · blocked by · gh · ac), then the title.
 * Hover reveals "↵ open" — except on a failed card, whose ↵ goes to
 * "↵ read" (the linked task). Blockedness is derived, never stored. */
function BoardCard({
  issue,
  tasks,
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
  tasks: TaskDto[];
  selected: boolean;
  focused: boolean;
  dragging: boolean;
  onDragStart: (e: React.DragEvent) => void;
  onDragEnd: () => void;
  onOpen: () => void;
  onOpenIssue: (issueId: string) => void;
  onReadTask: (taskId: string) => void;
}) {
  const task = issue.linkedTaskId
    ? tasks.find((t) => t.id === issue.linkedTaskId)
    : undefined;
  const rs = runStateFor(task?.status);
  const { ref, title } = issueRefParts(issue.title, issue.externalRef);
  const activeBlockers = issue.blockers.filter((b) => b.lane !== "done");

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
          // "↵ read" wins on an actionable (failed) card — frame 4a.
          if (task && rs.actionable) onReadTask(task.id);
          else onOpen();
        } else if (e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
    >
      {task && rs.kind !== "neutral" && (
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
      </span>
    </article>
  );
}
