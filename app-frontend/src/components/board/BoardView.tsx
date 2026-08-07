// Issue board (E17-02, #56): the project view's primary surface. Five
// lanes render from issue_list in board order; native HTML5 drag/drop
// persists moves via issue_move. Dragging a blocked card into In Progress
// gates on a confirm modal (ADR-0032: confirm, never a hard stop); the
// actual task spawn is E17-03. Card click swaps the right region to the
// card detail. GitHub issues arrive as NATIVE cards via the header's
// "Sync from GitHub" (gh badge for provenance). All state reconciles by
// refetching on issue events (blocked badges are derived — one move can
// flip OTHER cards).

import { useEffect, useState } from "react";
import {
  issueDispatch,
  issueList,
  issueMove,
  onAdeEvent,
  terminalOpenAgent,
  terminalWrite,
  type IssueDto,
  type Lane,
} from "../../lib/tauri";
import { useSidebar } from "../../store/sidebar";
import { useUi } from "../../store/ui";

const LANES: { id: Lane; label: string }[] = [
  { id: "backlog", label: "Backlog" },
  { id: "ready", label: "Ready" },
  { id: "in_progress", label: "In Progress" },
  { id: "in_review", label: "In Review" },
  { id: "done", label: "Done" },
];

const LANE_LABEL: Record<Lane, string> = {
  backlog: "Backlog",
  ready: "Ready",
  in_progress: "In Progress",
  in_review: "In Review",
  done: "Done",
};

/** A blocked drop awaiting the user's confirm (issue + where it lands). */
interface PendingBlockedDrop {
  issue: IssueDto;
  lane: Lane;
  position: number;
}

export default function BoardView({ projectId }: { projectId: string }) {
  const [issues, setIssues] = useState<IssueDto[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingBlockedDrop | null>(null);
  const projectTasks = useSidebar((s) => s.tasksByProject[projectId]);

  useEffect(() => {
    let cancelled = false;
    const reload = () =>
      issueList(projectId)
        .then((list) => !cancelled && setIssues(list))
        .catch((e) => !cancelled && setError(String(e)));
    void reload();
    const unlisten = onAdeEvent((ev) => {
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
    const issueId = e.dataTransfer.getData("text/ade-issue");
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

  return (
    <div className="board">
      {error && <p className="error">{error}</p>}
      {LANES.map(({ id, label }) => {
        const cards = issues.filter((i) => i.lane === id);
        return (
          <section key={id} className="board-lane" data-lane={id}>
            <header className="board-lane-header">
              {label}
              <span className="board-lane-count">{cards.length}</span>
            </header>
            <div
              className="board-lane-cards"
              onDragOver={(e) => {
                if (dragId) e.preventDefault();
              }}
              onDrop={(e) => handleDrop(e, id)}
            >
              {cards.map((issue) => (
                <article
                  key={issue.id}
                  className={`board-card${dragId === issue.id ? " dragging" : ""}`}
                  data-issue-id={issue.id}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData("text/ade-issue", issue.id);
                    e.dataTransfer.effectAllowed = "move";
                    setDragId(issue.id);
                  }}
                  onDragEnd={() => setDragId(null)}
                  onClick={() => {
                    const ui = useUi.getState();
                    ui.setBoardDetailIssueId(issue.id);
                    // The detail swaps into the right sheet — make sure it's
                    // visible regardless of the changes/chat mode.
                    ui.setChangesOpen(true);
                  }}
                >
                  <span className="board-card-title">{issue.title}</span>
                  <span className="board-card-badges">
                    {issue.externalRef && (
                      <a
                        className="board-card-github"
                        href={issue.externalRef}
                        title="Imported from GitHub"
                        onClick={(e) => e.stopPropagation()}
                      >
                        gh
                      </a>
                    )}
                    {issue.provider && (
                      <span className="board-card-provider">{issue.provider}</span>
                    )}
                    {issue.linkedTaskId &&
                      (() => {
                        const task = (projectTasks ?? []).find(
                          (t) => t.id === issue.linkedTaskId,
                        );
                        return (
                          <span
                            className={`board-card-dot${task ? ` status-${task.status}` : ""}`}
                            title={task ? `task ${task.status}` : "task linked"}
                          />
                        );
                      })()}
                    {issue.blocked && (
                      <span className="board-card-blocked" tabIndex={0}>
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
                  </span>
                </article>
              ))}
            </div>
          </section>
        );
      })}
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
