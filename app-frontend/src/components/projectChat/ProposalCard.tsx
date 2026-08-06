// Proposal approval card (E17-04, #58; ADR-0032). Renders a parsed
// ade-proposal block from the PM agent's message: editable issue rows
// (title, provider/model), droppable rows, Approve/Dismiss. Parse failure
// renders the raw block as plain code text — the card NEVER throws on bad
// input. Approve writes via issue_apply_proposal (all-or-nothing backend).

import { useEffect, useState } from "react";
import {
  issueApplyProposal,
  issueParseProposal,
  type ProposalDto,
} from "../../lib/tauri";

type CardState =
  | { kind: "parsing" }
  | { kind: "invalid" }
  | { kind: "ready"; proposal: ProposalDto }
  | { kind: "applied"; count: number }
  | { kind: "dismissed" };

export default function ProposalCard({
  raw,
  projectId,
}: {
  raw: string;
  projectId: string;
}) {
  const [state, setState] = useState<CardState>({ kind: "parsing" });
  const [applyError, setApplyError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);

  useEffect(() => {
    let cancelled = false;
    issueParseProposal(raw)
      .then((proposal) => !cancelled && setState({ kind: "ready", proposal }))
      .catch(() => !cancelled && setState({ kind: "invalid" }));
    return () => {
      cancelled = true;
    };
  }, [raw]);

  if (state.kind === "invalid" || state.kind === "dismissed") {
    return <pre className="proposal-raw">{raw}</pre>;
  }
  if (state.kind === "parsing") {
    return <div className="proposal-card muted">Parsing proposal…</div>;
  }
  if (state.kind === "applied") {
    return (
      <div className="proposal-card applied">
        ✓ {state.count} {state.count === 1 ? "issue" : "issues"} added to the board
      </div>
    );
  }

  const { proposal } = state;

  const update = (next: ProposalDto) => setState({ kind: "ready", proposal: next });

  const setIssue = (i: number, patch: Partial<ProposalDto["issues"][number]>) => {
    const issues = proposal.issues.map((iss, j) => {
      if (j !== i) return iss;
      // Title renames must not strand blockedBy references to the old title.
      const next = { ...iss, ...patch };
      return next;
    });
    // If the patch renamed a title, rewrite blockedBy references.
    if (patch.title !== undefined && patch.title !== proposal.issues[i].title) {
      const oldTitle = proposal.issues[i].title;
      for (const iss of issues) {
        iss.blockedBy = iss.blockedBy.map((t) => (t === oldTitle ? patch.title! : t));
      }
    }
    update({ ...proposal, issues });
  };

  const dropIssue = (i: number) => {
    const dropped = proposal.issues[i].title;
    const issues = proposal.issues
      .filter((_, j) => j !== i)
      .map((iss) => ({
        ...iss,
        // Dropping a row drops edges to it (backend would reject the
        // unknown title otherwise).
        blockedBy: iss.blockedBy.filter((t) => t !== dropped),
      }));
    update({ ...proposal, issues });
  };

  const approve = () => {
    setApplying(true);
    setApplyError(null);
    issueApplyProposal(projectId, proposal)
      .then((created) => setState({ kind: "applied", count: created.length }))
      .catch((e) => setApplyError(String(e)))
      .finally(() => setApplying(false));
  };

  return (
    <div className="proposal-card">
      <div className="proposal-card-header">
        <span>Proposed issues</span>
        {proposal.prd && (
          <span className="proposal-prd" title={proposal.prd.title ?? undefined}>
            {proposal.prd.path}
          </span>
        )}
      </div>
      <ul className="proposal-rows">
        {proposal.issues.map((issue, i) => (
          <li key={i} className="proposal-row">
            <input
              className="proposal-row-title"
              value={issue.title}
              onChange={(e) => setIssue(i, { title: e.target.value })}
              aria-label={`Issue ${i + 1} title`}
            />
            {issue.blockedBy.length > 0 && (
              <span className="proposal-row-edges">⟵ {issue.blockedBy.join(", ")}</span>
            )}
            <input
              className="proposal-row-provider"
              value={issue.provider ?? ""}
              placeholder="provider"
              onChange={(e) => setIssue(i, { provider: e.target.value || null })}
              aria-label={`Issue ${i + 1} provider`}
            />
            <button
              className="proposal-row-drop"
              onClick={() => dropIssue(i)}
              aria-label={`Drop ${issue.title}`}
            >
              ×
            </button>
          </li>
        ))}
      </ul>
      {applyError && <p className="error">{applyError}</p>}
      <div className="proposal-actions">
        <button onClick={() => setState({ kind: "dismissed" })}>Dismiss</button>
        <button
          className="primary"
          disabled={applying || proposal.issues.length === 0}
          onClick={approve}
        >
          {applying ? "Applying…" : `Approve ${proposal.issues.length}`}
        </button>
      </div>
    </div>
  );
}
