// Proposal approval card (E17-04, #58; ADR-0032) — design_handoff_v2 §5c.
// Renders a parsed fartCode-proposal block from the PM agent as numbered
// issue rows with a focus cursor and single-key actions: ↑/↓ (or j/k) move,
// e edits the focused title inline, x toggles dropped (rows stay visible,
// struck through), ↵ approves the surviving rows into Backlog. Blocked-by
// edges show as right-aligned row-number notes; dropping a row drops edges
// to it at approve time (the backend rejects unknown titles). Parse failure
// renders the raw block as plain text — the card NEVER throws on bad input.
// Approve writes via issue_apply_proposal (all-or-nothing backend).

import { useEffect, useRef, useState } from "react";
import {
  issueApplyProposal,
  issueParseProposal,
  type ProposalDto,
} from "../../lib/tauri";

type CardState =
  | { kind: "parsing" }
  | { kind: "invalid" }
  | { kind: "ready"; proposal: ProposalDto }
  | { kind: "applied"; count: number };

function prdFilename(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? path : path.slice(idx + 1);
}

export default function ProposalCard({
  raw,
  projectId,
}: {
  raw: string;
  projectId: string;
}) {
  const [state, setState] = useState<CardState>({ kind: "parsing" });
  const [dropped, setDropped] = useState<ReadonlySet<number>>(new Set());
  const [focused, setFocused] = useState(0);
  const [editing, setEditing] = useState<number | null>(null);
  const [editText, setEditText] = useState("");
  const [applyError, setApplyError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);
  const cardRef = useRef<HTMLDivElement>(null);
  // Ref mirror of `editing`: blur-commit races Escape-cancel through stale
  // closures otherwise (cancel clears the ref, so the late blur no-ops).
  const editingRef = useRef<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    issueParseProposal(raw)
      .then((proposal) => !cancelled && setState({ kind: "ready", proposal }))
      .catch(() => !cancelled && setState({ kind: "invalid" }));
    return () => {
      cancelled = true;
    };
  }, [raw]);

  if (state.kind === "invalid") {
    return <pre className="proposal-raw">{raw}</pre>;
  }
  if (state.kind === "parsing") {
    return <div className="proposal-card muted">Parsing proposal…</div>;
  }
  if (state.kind === "applied") {
    return (
      <div className="proposal-card applied">
        ✓ {state.count} {state.count === 1 ? "issue" : "issues"} added to Backlog
      </div>
    );
  }

  const { proposal } = state;
  const approveCount = proposal.issues.length - dropped.size;

  const beginEdit = (i: number) => {
    if (dropped.has(i) || applying) return;
    setFocused(i);
    editingRef.current = i;
    setEditing(i);
    setEditText(proposal.issues[i].title);
  };

  const cancelEdit = () => {
    editingRef.current = null;
    setEditing(null);
  };

  const commitEdit = () => {
    const i = editingRef.current;
    editingRef.current = null;
    setEditing(null);
    if (i === null) return;
    const title = editText.trim();
    const oldTitle = proposal.issues[i].title;
    if (!title || title === oldTitle) return;
    // Title renames must not strand blockedBy references to the old title.
    const issues = proposal.issues.map((iss, j) => ({
      ...iss,
      title: j === i ? title : iss.title,
      blockedBy: iss.blockedBy.map((t) => (t === oldTitle ? title : t)),
    }));
    setState({ kind: "ready", proposal: { ...proposal, issues } });
  };

  const toggleDrop = (i: number) => {
    setDropped((prev) => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });
  };

  const approve = () => {
    if (applying || approveCount === 0) return;
    const droppedTitles = new Set(
      proposal.issues.filter((_, i) => dropped.has(i)).map((iss) => iss.title),
    );
    // Dropped rows leave the payload, and edges to them go with it.
    const issues = proposal.issues
      .filter((_, i) => !dropped.has(i))
      .map((iss) => ({
        ...iss,
        blockedBy: iss.blockedBy.filter((t) => !droppedTitles.has(t)),
      }));
    setApplying(true);
    setApplyError(null);
    issueApplyProposal(projectId, { ...proposal, issues })
      .then((created) => setState({ kind: "applied", count: created.length }))
      .catch((e) => setApplyError(String(e)))
      .finally(() => setApplying(false));
  };

  /** Right-aligned mono note: blockers as 1-based row numbers (titles from
   * outside the proposal pass through verbatim); dropped blockers vanish. */
  const blockedNote = (blockedBy: string[]): string | null => {
    if (blockedBy.length === 0) return null;
    const refs: string[] = [];
    for (const title of blockedBy) {
      const j = proposal.issues.findIndex((iss) => iss.title === title);
      if (j === -1) refs.push(title);
      else if (!dropped.has(j)) refs.push(String(j + 1));
    }
    return refs.length > 0 ? `blocked by ${refs.join(", ")}` : null;
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (editing !== null) return; // the row input owns the keyboard
    const last = proposal.issues.length - 1;
    if (e.key === "ArrowDown" || e.key === "j") {
      setFocused((f) => Math.min(f + 1, last));
    } else if (e.key === "ArrowUp" || e.key === "k") {
      setFocused((f) => Math.max(f - 1, 0));
    } else if (e.key === "e") {
      beginEdit(focused);
    } else if (e.key === "x") {
      toggleDrop(focused);
    } else if (e.key === "Enter") {
      approve();
    } else {
      return;
    }
    e.preventDefault();
  };

  return (
    <div
      ref={cardRef}
      className="proposal-card"
      tabIndex={0}
      role="group"
      aria-label="Proposal"
      onKeyDown={onKeyDown}
    >
      <div className="proposal-card-header">
        <span className="proposal-card-label">Proposal</span>
        {proposal.prd && (
          <span
            className="proposal-prd"
            title={proposal.prd.title ?? proposal.prd.path}
          >
            {prdFilename(proposal.prd.path)}
          </span>
        )}
      </div>
      <ul className="proposal-rows">
        {proposal.issues.map((issue, i) => {
          const isDropped = dropped.has(i);
          const note = isDropped ? "dropped" : blockedNote(issue.blockedBy);
          return (
            <li
              key={i}
              className={`proposal-row${i === focused ? " focused" : ""}${
                isDropped ? " dropped" : ""
              }`}
              onClick={() => {
                setFocused(i);
                cardRef.current?.focus();
              }}
              onDoubleClick={() => beginEdit(i)}
            >
              <span className="proposal-row-index">{i + 1}</span>
              {editing === i ? (
                <input
                  className="proposal-row-edit"
                  value={editText}
                  autoFocus
                  onFocus={(e) => e.currentTarget.select()}
                  onChange={(e) => setEditText(e.target.value)}
                  onBlur={commitEdit}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") {
                      e.preventDefault();
                      commitEdit();
                      cardRef.current?.focus();
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      cancelEdit();
                      cardRef.current?.focus();
                    }
                  }}
                  aria-label={`Issue ${i + 1} title`}
                />
              ) : (
                <span className="proposal-row-title">{issue.title}</span>
              )}
              {note && <span className="proposal-row-note">{note}</span>}
            </li>
          );
        })}
      </ul>
      {applyError && <p className="error">{applyError}</p>}
      <div className="proposal-card-footer">
        <span className="proposal-keys">e edit · x drop</span>
        <button
          className="proposal-approve"
          disabled={applying || approveCount === 0}
          onClick={approve}
        >
          <span className="key">↵</span>{" "}
          {applying ? "applying…" : `approve ${approveCount} → Backlog`}
        </button>
      </div>
    </div>
  );
}
