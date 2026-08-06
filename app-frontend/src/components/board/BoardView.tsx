// Issue board (E17-02, #56): the project view's primary surface. This is
// the read path — lanes render from issue_list in board order, blocked
// badges included. Drag/drop, card detail, and the blocked-dispatch
// confirm dialog build on top of this component.

import { useEffect, useState } from "react";
import { issueList, onAdeEvent, type IssueDto, type Lane } from "../../lib/tauri";

const LANES: { id: Lane; label: string }[] = [
  { id: "backlog", label: "Backlog" },
  { id: "ready", label: "Ready" },
  { id: "in_progress", label: "In Progress" },
  { id: "in_review", label: "In Review" },
  { id: "done", label: "Done" },
];

export default function BoardView({ projectId }: { projectId: string }) {
  const [issues, setIssues] = useState<IssueDto[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const reload = () =>
      issueList(projectId)
        .then((list) => !cancelled && setIssues(list))
        .catch((e) => !cancelled && setError(String(e)));
    void reload();
    // Any issue event can flip derived blocked badges on OTHER cards —
    // always refetch the project list (ADR-0032).
    const unlisten = onAdeEvent((ev) => {
      if (
        (ev.type === "issue:created" ||
          ev.type === "issue:updated" ||
          ev.type === "issue:deleted") &&
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
            <div className="board-lane-cards">
              {cards.map((issue) => (
                <article key={issue.id} className="board-card" data-issue-id={issue.id}>
                  <span className="board-card-title">{issue.title}</span>
                  {issue.blocked && (
                    <span
                      className="board-card-blocked"
                      title={`Blocked by ${issue.blockers.map((b) => b.title).join(", ")}`}
                    >
                      blocked
                    </span>
                  )}
                </article>
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
