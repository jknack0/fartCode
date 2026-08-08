// Issue board (E17-02, #56): the project view's primary surface. One
// plate with five hairline-ruled lanes render from issue_list in board
// order; native HTML5 drag/drop persists moves via issue_move. Dragging
// a blocked card into In Progress gates on a confirm modal (ADR-0032:
// confirm, never a hard stop); the actual task spawn is E17-03. Card
// click swaps the right region to the card detail. GitHub issues arrive
// as NATIVE cards via the header's "Sync from GitHub". All state
// reconciles by refetching on issue events (blocked badges are derived
// — one move can flip OTHER cards).

import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import {
  issueCreate,
  issueDispatch,
  issueList,
  issueMove,
  onFartcodeEvent,
  terminalOpenAgent,
  terminalWrite,
  type IssueDto,
  type Lane,
  type TaskDto,
} from "../../lib/tauri";
import { useSidebar } from "../../store/sidebar";
import { useUi } from "../../store/ui";
import { IconGitHub, IconPlus } from "../icons";

const LANES: { id: Lane; label: string }[] = [
  { id: "backlog", label: "Backlog" },
  { id: "ready", label: "Ready" },
  { id: "in_progress", label: "In Progress" },
  { id: "in_review", label: "In Review" },
  { id: "done", label: "Done" },
];

export const LANE_LABEL: Record<Lane, string> = {
  backlog: "Backlog",
  ready: "Ready",
  in_progress: "In Progress",
  in_review: "In Review",
  done: "Done",
};

/** Provider chip label: the display name minus any "CLI"/"-cli" suffix
 * ("Claude Code CLI" → "Claude Code"). */
const providerLabel = (provider: string) => provider.replace(/\s*[-\s]?CLI$/i, "");

/** A blocked drop awaiting the user's confirm (issue + where it lands). */
interface PendingBlockedDrop {
  issue: IssueDto;
  lane: Lane;
  position: number;
}

export default function BoardView({ projectId }: { projectId: string }) {
  const [issues, setIssues] = useState<IssueDto[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [overLane, setOverLane] = useState<Lane | null>(null);
  const [pending, setPending] = useState<PendingBlockedDrop | null>(null);
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

  const move = (issueId: string, lane: Lane, position: number) =>
    issueMove(issueId, lane, position).catch((e) => setError(String(e)));

  /** Focuses the card's linked task (reattach never spawns a second
   * worktree — ADR-0032). */
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
    setOverLane(null);
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
    if (lane === "in_progress") {
      // Reattach: a live linked task gets a status move + focus, never a
      // second spawn (ADR-0032).
      if (issue.linkedTaskId) {
        void move(issueId, lane, position);
        focusLinkedTask(issue.linkedTaskId);
        return;
      }
      // Dispatch spawns a real agent — blocked cards confirm first.
      if (issue.blocked) {
        setPending({ issue, lane, position });
        return;
      }
      void dispatch(issue);
      return;
    }
    void move(issueId, lane, position);
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
      <div className="board-toolbar">
        <h2 className="board-title">Board</h2>
        <span className="board-total">{total}</span>
        <button
          className="board-add"
          onClick={() => setAdding(true)}
          title="Add issue to Backlog"
        >
          <IconPlus size={10} />
          Add issue
        </button>
      </div>

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
          <button
            className="primary"
            disabled={!newTitle.trim() || creating}
            onClick={() => void submitNew()}
          >
            {creating ? "Adding…" : "Add"}
          </button>
          <button
            onClick={() => {
              setAdding(false);
              setNewTitle("");
            }}
          >
            Cancel
          </button>
        </div>
      )}

      {!loaded && !error ? (
        <div className="board-empty muted">Reading the board…</div>
      ) : total === 0 && !adding ? (
        <div className="board-empty">
          <p className="muted">The board is empty.</p>
          <p className="muted">
            Pull work onto it — the GitHub key above imports every open issue,
            or add a card by hand. Dragging one into In&nbsp;Progress dispatches
            an agent in its own worktree.
          </p>
          <button className="primary" onClick={() => setAdding(true)}>
            Add issue
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
                  <span className="board-lane-count">{count}</span>
                </div>
              );
            })}
          </div>
          <div className="board-lanes">
            {LANES.map(({ id }) => {
              const cards = issues.filter((i) => i.lane === id);
              return (
                <section
                  key={id}
                  className={`board-lane${overLane === id && dragId ? " over" : ""}`}
                  data-lane={id}
                >
                  <div
                    className="board-lane-cards"
                    onDragOver={(e) => {
                      if (dragId) {
                        e.preventDefault();
                        setOverLane(id);
                      }
                    }}
                    onDragLeave={(e) => {
                      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
                        setOverLane((l) => (l === id ? null : l));
                      }
                    }}
                    onDrop={(e) => handleDrop(e, id)}
                  >
                    {cards.map((issue) => (
                      <BoardCard
                        key={issue.id}
                        issue={issue}
                        tasks={projectTasks ?? []}
                        selected={detailIssueId === issue.id}
                        dragging={dragId === issue.id}
                        onDragStart={(e) => {
                          e.dataTransfer.setData("text/fartCode-issue", issue.id);
                          e.dataTransfer.effectAllowed = "move";
                          setDragId(issue.id);
                        }}
                        onDragEnd={() => {
                          setDragId(null);
                          setOverLane(null);
                        }}
                        onOpen={() => {
                          const ui = useUi.getState();
                          ui.setBoardDetailIssueId(issue.id);
                          // The detail swaps into the right sheet — make
                          // sure it's visible regardless of changes/chat.
                          ui.setChangesOpen(true);
                        }}
                      />
                    ))}
                    {cards.length === 0 && <div className="board-lane-placeholder" />}
                  </div>
                </section>
              );
            })}
          </div>
        </div>
      )}

      {pending && (
        <div className="modal-backdrop" onClick={() => setPending(null)}>
          <div
            className="modal"
            role="dialog"
            aria-label="Dispatch blocked issue"
            onClick={(e) => e.stopPropagation()}
          >
            <h2>Dispatch blocked issue?</h2>
            <p>
              <strong>{pending.issue.title}</strong> is blocked by:
            </p>
            <ul className="blocked-confirm-list">
              {pending.issue.blockers.map((b) => (
                <li key={b.id}>
                  {b.title} <em>({LANE_LABEL[b.lane] ?? b.lane})</em>
                </li>
              ))}
            </ul>
            <div className="modal-actions">
              <button onClick={() => setPending(null)}>Cancel</button>
              <button
                className="primary"
                onClick={() => {
                  const { issue, lane, position } = pending;
                  setPending(null);
                  if (issue.linkedTaskId) {
                    void move(issue.id, lane, position);
                  } else {
                    void dispatch(issue);
                  }
                }}
              >
                Dispatch anyway
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** One card. Dot-first: the linked-task state leads, then the title,
 * then provenance + agent + dependency chips. */
function BoardCard({
  issue,
  tasks,
  selected,
  dragging,
  onDragStart,
  onDragEnd,
  onOpen,
}: {
  issue: IssueDto;
  tasks: TaskDto[];
  selected: boolean;
  dragging: boolean;
  onDragStart: (e: React.DragEvent) => void;
  onDragEnd: () => void;
  onOpen: () => void;
}) {
  const task = issue.linkedTaskId
    ? tasks.find((t) => t.id === issue.linkedTaskId)
    : undefined;
  return (
    <article
      className={[
        "board-card",
        selected ? "selected" : "",
        dragging ? "dragging" : "",
        issue.blocked ? "blocked" : "",
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
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
    >
      <span className="board-card-main">
        {issue.linkedTaskId && (
          <span
            className={`status-dot${task ? ` status-${task.status}` : ""}`}
            title={task ? `${task.name} — ${task.status.replace("_", " ")}` : "task linked"}
          />
        )}
        <span className="board-card-title">{issue.title}</span>
        {issue.acceptance.length > 0 && (
          <span className="board-card-ac">
            {issue.acceptance.length}
            <span className="board-card-ac-unit">ac</span>
          </span>
        )}
      </span>
      <span className="board-card-chips">
        {issue.blocked && (
          <span className="board-chip board-chip-blocked">
            blocked
            <span className="blocked-popover" role="tooltip">
              {issue.blockers.map((b) => (
                <span key={b.id} className="blocked-popover-row">
                  {b.title}
                  <em>{LANE_LABEL[b.lane] ?? b.lane}</em>
                </span>
              ))}
            </span>
          </span>
        )}
        {issue.externalRef && (
          <button
            className="board-chip board-chip-gh"
            title={issue.externalRef}
            draggable={false}
            onClick={(e) => {
              e.stopPropagation();
              void open(issue.externalRef!).catch(() => {});
            }}
          >
            <IconGitHub size={10} />
          </button>
        )}
        {issue.provider && (
          <span className="board-chip board-chip-provider" title={issue.provider}>
            {providerLabel(issue.provider)}
          </span>
        )}
      </span>
    </article>
  );
}
