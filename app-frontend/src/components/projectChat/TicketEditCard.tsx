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
import { useAsyncSubmit } from "../../lib/useAsyncSubmit";

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
  // #125: second-confirm gate for an empty-acceptance edit (clears all criteria).
  const [confirmClear, setConfirmClear] = useState(false);
  const { busy: applying, error, run } = useAsyncSubmit();

  useEffect(() => {
    setConfirmClear(false);
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

  // #125: "acceptance": [] is a FULL replacement that wipes every existing
  // criterion — never present it as a plain "(0)"; make apply a two-step.
  const clearsAcceptance =
    edit.acceptance !== null &&
    edit.acceptance.length === 0 &&
    (issue?.acceptance.length ?? 0) > 0;

  const apply = () => {
    if (clearsAcceptance && !confirmClear) {
      setConfirmClear(true);
      return;
    }
    const patch: Parameters<typeof issueUpdate>[1] = {};
    if (edit.title !== null) patch.title = edit.title;
    if (edit.body !== null) patch.body = edit.body;
    if (edit.acceptance !== null) patch.acceptance = edit.acceptance;
    void run(() => issueUpdate(edit.issueId, patch), {
      onSuccess: () => setState({ kind: "applied" }),
    });
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
            {edit.acceptance.length === 0
              ? issue && issue.acceptance.length > 0
                ? `Acceptance — clears all ${issue.acceptance.length} criteria`
                : "Acceptance — empty"
              : `Acceptance (${edit.acceptance.length})`}
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
      {confirmClear && (
        <p className="error">
          Applying clears all {issue?.acceptance.length} acceptance criteria —
          apply again to confirm.
        </p>
      )}
      {error && <p className="error">{error}</p>}
      <div className="proposal-card-footer">
        <button className="proposal-dismiss" onClick={dismiss}>
          esc dismiss
        </button>
        <button className="proposal-approve" disabled={applying} onClick={apply}>
          <span className="key">↵</span>{" "}
          {applying ? "applying…" : confirmClear ? "confirm clear" : "apply"}
        </button>
      </div>
    </div>
  );
}
