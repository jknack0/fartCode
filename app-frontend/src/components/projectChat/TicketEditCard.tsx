// Ticket-edit approval card: renders a parsed fartCode-ticket-edit block
// from the PM agent. Shows what changes (title / body / acceptance);
// Apply patches the issue via issue_update, dismiss reverts to raw text.
// Mirrors ProposalCard's never-throw contract and its §5c card shell:
// mono uppercase header, hairline-sectioned body, mono key footer
// (esc dismiss · ↵ apply) with the same click affordances.

import { useEffect, useState } from "react";
import { issueList, issueUpdate, type IssueDto } from "../../lib/tauri";
import { renderMarkdown } from "../../lib/markdown";
import { parseTicketEdit, type TicketEdit } from "../../lib/ticketEdit";

type CardState =
  | { kind: "loading" }
  | { kind: "invalid" }
  | { kind: "ready"; edit: TicketEdit; issue: IssueDto | null }
  | { kind: "applied" }
  | { kind: "dismissed" };

export default function TicketEditCard({
  raw,
  projectId,
}: {
  raw: string;
  projectId: string;
}) {
  const [state, setState] = useState<CardState>({ kind: "loading" });
  const [error, setError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);

  useEffect(() => {
    const edit = parseTicketEdit(raw);
    if (!edit) {
      setState({ kind: "invalid" });
      return;
    }
    let cancelled = false;
    issueList(projectId)
      .then((list) => {
        if (cancelled) return;
        setState({
          kind: "ready",
          edit,
          issue: list.find((i) => i.id === edit.issueId) ?? null,
        });
      })
      .catch(() => !cancelled && setState({ kind: "ready", edit, issue: null }));
    return () => {
      cancelled = true;
    };
  }, [raw, projectId]);

  if (state.kind === "invalid" || state.kind === "dismissed") {
    return <pre className="proposal-raw">{raw}</pre>;
  }
  if (state.kind === "loading") {
    return <div className="proposal-card muted">Parsing ticket edit…</div>;
  }
  if (state.kind === "applied") {
    return <div className="proposal-card applied">✓ Ticket updated</div>;
  }

  const { edit, issue } = state;

  const apply = () => {
    if (applying) return;
    setApplying(true);
    setError(null);
    const patch: Parameters<typeof issueUpdate>[1] = {};
    if (edit.title !== null) patch.title = edit.title;
    if (edit.body !== null) patch.body = edit.body;
    if (edit.acceptance !== null) patch.acceptance = edit.acceptance;
    issueUpdate(edit.issueId, patch)
      .then(() => setState({ kind: "applied" }))
      .catch((e) => setError(String(e)))
      .finally(() => setApplying(false));
  };

  const dismiss = () => setState({ kind: "dismissed" });

  return (
    <div
      className="proposal-card"
      tabIndex={0}
      role="group"
      aria-label="Ticket edit"
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          apply();
        } else if (e.key === "Escape") {
          e.preventDefault();
          dismiss();
        }
      }}
    >
      <div className="proposal-card-header">
        <span className="proposal-card-label">Ticket edit</span>
        <span className="proposal-prd" title={edit.issueId}>
          {issue?.title ?? edit.issueId}
        </span>
      </div>
      {edit.title !== null && issue && edit.title !== issue.title && (
        <div className="ticket-edit-field">
          <span className="ticket-edit-label">Title</span>
          <span className="ticket-edit-old">{issue.title}</span>
          <span className="ticket-edit-new">{edit.title}</span>
        </div>
      )}
      {edit.body !== null && (
        <div className="ticket-edit-field">
          <span className="ticket-edit-label">Body</span>
          <div className="ticket-edit-md card-detail-md">{renderMarkdown(edit.body)}</div>
        </div>
      )}
      {edit.acceptance !== null && (
        <div className="ticket-edit-field">
          <span className="ticket-edit-label">
            Acceptance ({edit.acceptance.length})
          </span>
          <ul className="ticket-edit-ac">
            {edit.acceptance.map((ac, i) => (
              <li key={i}>{ac}</li>
            ))}
          </ul>
        </div>
      )}
      {issue === null && (
        <p className="error">Issue not found on this board — apply will fail.</p>
      )}
      {error && <p className="error">{error}</p>}
      <div className="proposal-card-footer">
        <button className="proposal-dismiss" onClick={dismiss}>
          esc dismiss
        </button>
        <button className="proposal-approve" disabled={applying} onClick={apply}>
          <span className="key">↵</span> {applying ? "applying…" : "apply"}
        </button>
      </div>
    </div>
  );
}
