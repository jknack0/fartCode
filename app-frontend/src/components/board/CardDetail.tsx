// Card detail (E17-02, #56): the board card's detail surface, swapped into
// the project view's right region (takes precedence over the PM chat).
// Read path: loads the issue and renders title/body/acceptance + blockers.
// Editing, edge management, and delete build on top of this component.

import { useEffect, useState } from "react";
import { issueList, type IssueDto } from "../../lib/tauri";
import { useUi } from "../../store/ui";

export default function CardDetail({
  projectId,
  issueId,
}: {
  projectId: string;
  issueId: string;
}) {
  const [issue, setIssue] = useState<IssueDto | null>(null);
  const [error, setError] = useState<string | null>(null);
  const close = () => useUi.getState().setBoardDetailIssueId(null);

  useEffect(() => {
    let cancelled = false;
    // No single-issue command: derive from the project list (the board
    // refetches on every issue event, so this reads the same source).
    issueList(projectId)
      .then((list) => {
        if (cancelled) return;
        const found = list.find((i) => i.id === issueId) ?? null;
        if (found) setIssue(found);
        else setError("issue not found");
      })
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [projectId, issueId]);

  return (
    <aside className="card-detail" data-issue-id={issueId}>
      <header className="card-detail-header">
        <span>{issue?.title ?? "…"}</span>
        <button onClick={close} aria-label="Close detail">
          ×
        </button>
      </header>
      {error && <p className="error">{error}</p>}
      {issue && (
        <div className="card-detail-body">
          {issue.body && <p>{issue.body}</p>}
          {issue.acceptance.length > 0 && (
            <ul>
              {issue.acceptance.map((ac, i) => (
                <li key={i}>{ac}</li>
              ))}
            </ul>
          )}
          {issue.blockers.length > 0 && (
            <p className="muted">
              Blocked by {issue.blockers.map((b) => b.title).join(", ")}
            </p>
          )}
        </div>
      )}
    </aside>
  );
}
