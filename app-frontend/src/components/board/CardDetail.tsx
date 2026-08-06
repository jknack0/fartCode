// Card detail (E17-02, #56): the board card's detail surface, swapped into
// the project view's right region (takes precedence over the PM chat).
// Edits go through the issue commands, which return the updated issue —
// no separate refetch. Blocked-by edges are managed here; backend cycle /
// cross-project rejections render inline.

import { useEffect, useState } from "react";
import {
  issueDelete,
  issueLink,
  issueList,
  issueUnlink,
  issueUpdate,
  onAdeEvent,
  type IssueDto,
} from "../../lib/tauri";
import { useUi } from "../../store/ui";

export default function CardDetail({
  projectId,
  issueId,
}: {
  projectId: string;
  issueId: string;
}) {
  const [issue, setIssue] = useState<IssueDto | null>(null);
  /** All project issues — feeds the "blocked by" picker. */
  const [siblings, setSiblings] = useState<IssueDto[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [newAc, setNewAc] = useState("");
  const [edgeTarget, setEdgeTarget] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const close = () => useUi.getState().setBoardDetailIssueId(null);

  useEffect(() => {
    let cancelled = false;
    const reload = () =>
      issueList(projectId)
        .then((list) => {
          if (cancelled) return;
          setSiblings(list);
          const found = list.find((i) => i.id === issueId) ?? null;
          if (!found) {
            setError("issue not found");
            return;
          }
          setIssue(found);
          setTitle(found.title);
          setBody(found.body ?? "");
        })
        .catch((e) => !cancelled && setError(String(e)));
    void reload();
    // Stay in sync with board operations (moves, proposal approvals).
    const unlisten = onAdeEvent((ev) => {
      if (
        (ev.type === "issue:created" ||
          ev.type === "issue:updated" ||
          ev.type === "issue:deleted") &&
        ev.projectId === projectId
      ) {
        if (ev.type === "issue:deleted" && ev.id === issueId) {
          close();
          return;
        }
        void reload();
      }
    });
    return () => {
      cancelled = true;
      void unlisten.then((off) => off());
    };
  }, [projectId, issueId]);

  /** Applies a mutation result; backend errors surface inline. */
  const apply = (p: Promise<IssueDto>) =>
    p.then(setIssue).catch((e) => setError(String(e)));

  const commitTitle = () => {
    const next = title.trim();
    if (issue && next && next !== issue.title) {
      void apply(issueUpdate(issueId, { title: next }));
    }
  };
  const commitBody = () => {
    if (issue && body !== (issue.body ?? "")) {
      void apply(issueUpdate(issueId, { body: body || null }));
    }
  };
  const setAcceptance = (items: string[]) =>
    void apply(issueUpdate(issueId, { acceptance: items }));

  if (!issue && !error) {
    return (
      <aside className="card-detail">
        <p className="muted">Loading…</p>
      </aside>
    );
  }

  const blockable = siblings.filter(
    (s) => s.id !== issueId && !(issue?.blockers ?? []).some((b) => b.id === s.id),
  );

  return (
    <aside className="card-detail" data-issue-id={issueId}>
      <header className="card-detail-header">
        <span className={`card-detail-lane lane-${issue?.lane}`}>{issue?.lane}</span>
        <button onClick={close} aria-label="Close detail">
          ×
        </button>
      </header>
      {error && <p className="error">{error}</p>}
      {issue && (
        <div className="card-detail-body">
          <input
            className="card-detail-title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onBlur={commitTitle}
            onKeyDown={(e) => e.key === "Enter" && commitTitle()}
            aria-label="Issue title"
          />
          <textarea
            className="card-detail-body-edit"
            value={body}
            placeholder="Body"
            rows={5}
            onChange={(e) => setBody(e.target.value)}
            onBlur={commitBody}
          />

          <h3>Acceptance</h3>
          <ul className="card-detail-ac">
            {issue.acceptance.map((ac, i) => (
              <li key={i}>
                {ac}
                <button
                  aria-label="Remove criterion"
                  onClick={() =>
                    setAcceptance(issue.acceptance.filter((_, j) => j !== i))
                  }
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
          <div className="card-detail-ac-add">
            <input
              value={newAc}
              placeholder="Add criterion"
              onChange={(e) => setNewAc(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && newAc.trim()) {
                  setAcceptance([...issue.acceptance, newAc.trim()]);
                  setNewAc("");
                }
              }}
            />
          </div>

          <h3>Blocked by</h3>
          <ul className="card-detail-edges">
            {issue.blockers.map((b) => (
              <li key={b.id}>
                {b.title} <em>({b.lane})</em>
                <button
                  aria-label={`Remove blocker ${b.title}`}
                  onClick={() => void apply(issueUnlink(issueId, b.id))}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
          {blockable.length > 0 && (
            <div className="card-detail-edge-add">
              <select
                value={edgeTarget}
                onChange={(e) => setEdgeTarget(e.target.value)}
                aria-label="Blocker issue"
              >
                <option value="">Add blocker…</option>
                {blockable.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.title}
                  </option>
                ))}
              </select>
              <button
                disabled={!edgeTarget}
                onClick={() => {
                  void apply(issueLink(issueId, edgeTarget));
                  setEdgeTarget("");
                }}
              >
                Add
              </button>
            </div>
          )}

          {issue.linkedTaskId && (
            <p className="card-detail-linked">
              Linked task <code>{issue.linkedTaskId}</code>
            </p>
          )}
          {issue.prdPath && (
            <p className="card-detail-prd muted">
              PRD <code>{issue.prdPath}</code>
              {issue.prdSection ? ` · ${issue.prdSection}` : ""}
            </p>
          )}

          <div className="card-detail-danger">
            {confirmDelete ? (
              <>
                <button className="danger" onClick={() => void issueDelete(issueId).then(close).catch((e) => setError(String(e)))}>
                  Confirm delete
                </button>
                <button onClick={() => setConfirmDelete(false)}>Keep</button>
              </>
            ) : (
              <button onClick={() => setConfirmDelete(true)}>Delete issue</button>
            )}
          </div>
        </div>
      )}
    </aside>
  );
}
