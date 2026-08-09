// Transcript item renderers (E2-11-6): per-kind rows for the reduced
// transcript — messages, thinking rows, tool-call cards, tool groups — plus
// the memoized settled-turn block. Concepts ported from the reference
// chat-ui (Solid), sized for the Phase-2 React renderer: no virtualization,
// but the two-tier structure (memoized settled turns + live active turn)
// keeps streaming appends from re-rendering settled history (ADR-0031).
//
// Status voice (DESIGN.md "dots are data"): tool status renders as neutral
// glyphs — running pulses, done is a passive check, error is destructive —
// and awaiting-permission overrides with an info-blue key (the prompt
// itself docks at the composer, reference PermissionBand placement).
import { Fragment, memo, useState, type ReactNode } from "react";
import { extractProposalBlocks, stripProposalBlocks } from "../lib/proposal";
import { extractTicketEditBlocks, stripTicketEditBlocks } from "../lib/ticketEdit";
import ProposalCard from "./projectChat/ProposalCard";
import TicketEditCard from "./projectChat/TicketEditCard";
import type {
  MessageItem,
  ThinkingItem,
  ToolCallItem,
  ToolGroupItem,
  ToolNode,
  ToolStatus,
  TranscriptItem,
  TranscriptTurn,
} from "../lib/tauri";

/** Tool-call ids gated by a pending permission (shield override). Only the
 * active turn can await a permission; settled turns pass the frozen empty
 * set so the memo comparator never has to look at it. */
export const NO_AWAITING: ReadonlySet<string> = new Set();

function StatusGlyph({
  status,
  awaiting,
}: {
  status: ToolStatus;
  awaiting: boolean;
}) {
  if (awaiting) {
    return (
      <span className="chat-status awaiting" title="Awaiting permission">
        ◆
      </span>
    );
  }
  if (status === "running") {
    return (
      <span className="chat-status running" title="Running" aria-label="running" />
    );
  }
  if (status === "error") {
    return (
      <span className="chat-status error" title="Failed">
        ✕
      </span>
    );
  }
  return (
    <span className="chat-status done" title="Completed">
      ✓
    </span>
  );
}

function basename(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? path : path.slice(idx + 1);
}

function dirname(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx <= 0 ? "" : path.slice(0, idx);
}

// -- messages -----------------------------------------------------------------

/** File mentions in PM prose (§5c): wraps path-looking tokens (a slash and
 * an extension, e.g. docs/prds/invite-vetting.md) in a styled span — mono
 * 11.5px --info under .pm-chat, inert everywhere else. Display only, like
 * the frame: there is no file surface to open yet (E5). */
const FILE_MENTION = /(?:^|[\s(`])((?:[\w.-]+\/)+[\w.-]+\.[A-Za-z]{1,8})/g;

function withFileMentions(text: string): ReactNode {
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(FILE_MENTION)) {
    const start = m.index! + m[0].length - m[1].length;
    if (start > last) out.push(text.slice(last, start));
    out.push(
      <span
        key={start}
        className="chat-file-mention"
        title="file mention — file surfaces land with E5"
      >
        {m[1]}
      </span>,
    );
    last = start + m[1].length;
  }
  if (out.length === 0) return text;
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Minimal inline markdown for PM prose (§5c): *em*, **strong**, `code` —
 * the token grammar mirrors lib/markdown.tsx's inline() (links excluded;
 * PM prose has no link affordance yet). Runs BEFORE the file-mention wrap:
 * plain and emphasized segments still get mention spans, code stays raw.
 * PM branch only — the task ACP transcript keeps literal text. */
const PM_INLINE = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*)|(\*[^*\n]+\*)/g;

function renderPmProse(text: string): ReactNode {
  const out: ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(PM_INLINE)) {
    const idx = m.index!;
    if (idx > last) {
      out.push(
        <Fragment key={`t${last}`}>{withFileMentions(text.slice(last, idx))}</Fragment>,
      );
    }
    const tok = m[0];
    if (tok.startsWith("`")) {
      out.push(<code key={idx}>{tok.slice(1, -1)}</code>);
    } else if (tok.startsWith("**")) {
      out.push(<strong key={idx}>{withFileMentions(tok.slice(2, -2))}</strong>);
    } else {
      out.push(<em key={idx}>{withFileMentions(tok.slice(1, -1))}</em>);
    }
    last = idx + tok.length;
  }
  if (out.length === 0) return withFileMentions(text);
  if (last < text.length) {
    out.push(<Fragment key={`t${last}`}>{withFileMentions(text.slice(last))}</Fragment>);
  }
  return out;
}

function MessageRow({
  item,
  proposalProjectId,
}: {
  item: MessageItem;
  /** E17-04: set only for the PM chat (project: owner) — assistant text is
   * scanned for fartCode-proposal blocks, each rendered as an approval card. */
  proposalProjectId?: string | null;
}) {
  if (item.role === "user") {
    return <div className="chat-user-card">{item.text}</div>;
  }
  if (proposalProjectId) {
    const blocks = extractProposalBlocks(item.text);
    const edits = extractTicketEditBlocks(item.text);
    if (blocks.length > 0 || edits.length > 0) {
      const prose = stripTicketEditBlocks(stripProposalBlocks(item.text));
      // `has-cards` / `chat-prose` are styling hooks only (PM panel scope —
      // src/styles/project-chat.css); behavior is identical either way.
      return (
        <div className="chat-assistant has-cards">
          {prose && <div className="chat-prose">{renderPmProse(prose)}</div>}
          {blocks.map((raw, i) => (
            <ProposalCard key={`p${i}`} raw={raw} projectId={proposalProjectId} />
          ))}
          {edits.map((raw, i) => (
            <TicketEditCard key={`e${i}`} raw={raw} projectId={proposalProjectId} />
          ))}
        </div>
      );
    }
  }
  return (
    <div className="chat-assistant">
      {proposalProjectId ? renderPmProse(item.text) : item.text}
    </div>
  );
}

// -- thinking -----------------------------------------------------------------

function ThinkingRow({ item }: { item: ThinkingItem }) {
  // null = automatic: expanded while streaming, header-only once done
  // (reference thinking.def collapse behavior).
  const [expanded, setExpanded] = useState<boolean | null>(null);
  const streaming = item.status === "thinking";
  const open = expanded ?? streaming;
  const label = streaming
    ? "Thinking…"
    : item.durationMs != null
      ? `Thought for ${Math.max(1, Math.round(item.durationMs / 1000))}s`
      : "Thought";
  return (
    <div className="chat-thinking">
      <button
        className="chat-thinking-header"
        onClick={() => setExpanded(!open)}
        aria-expanded={open}
      >
        <span className={`chat-chevron${open ? " open" : ""}`}>›</span>
        {label}
      </button>
      {open && item.text && <div className="chat-thinking-body">{item.text}</div>}
    </div>
  );
}

// -- tool calls ---------------------------------------------------------------

/** One-line tool row: glyph + verb + subject + trailing detail. */
function ToolLine({
  status,
  awaiting,
  verb,
  subject,
  detail,
  error,
}: {
  status: ToolStatus;
  awaiting: boolean;
  verb: string;
  subject: string;
  detail?: string;
  error?: boolean;
}) {
  return (
    <div className={`chat-tool-line${error ? " failed" : ""}`}>
      <StatusGlyph status={status} awaiting={awaiting} />
      <span className="chat-tool-verb">{verb}</span>
      <span className="chat-tool-subject">{subject}</span>
      {detail && <span className="chat-tool-detail">{detail}</span>}
    </div>
  );
}

function ExecuteCard({ item, awaiting }: { item: ToolCallItem; awaiting: boolean }) {
  const [open, setOpen] = useState(true);
  const title = item.inputSummary || item.title || "Execute";
  return (
    <div className="chat-tool-card">
      <button
        className="chat-tool-card-header"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
      >
        <StatusGlyph status={item.status} awaiting={awaiting} />
        <span className="chat-tool-card-title">{title}</span>
        <span className={`chat-chevron${open ? " open" : ""}`}>›</span>
      </button>
      {open && (item.command || item.outputText) && (
        <div className="chat-tool-card-body">
          {item.command && <div className="chat-cmd">$ {item.command}</div>}
          {item.outputText && <div className="chat-output">{item.outputText}</div>}
        </div>
      )}
    </div>
  );
}

const FILE_PREVIEW_LINES = 12;

function FileChangeCard({ item, awaiting }: { item: ToolCallItem; awaiting: boolean }) {
  const created = item.kind === "create-file-tool-call";
  const path = item.path ?? item.title;
  const text = item.newText ?? item.content ?? "";
  const lines = text.length > 0 ? text.split("\n") : [];
  const clipped = lines.length > FILE_PREVIEW_LINES;
  const preview = clipped ? lines.slice(0, FILE_PREVIEW_LINES) : lines;
  return (
    <div className="chat-tool-card">
      <div className="chat-tool-card-header static">
        <StatusGlyph status={item.status} awaiting={awaiting} />
        <span className="chat-tool-verb">{created ? "Create" : "Edit"}</span>
        <span className="chat-tool-card-title">{basename(path)}</span>
        <span className="chat-tool-detail">{dirname(path)}</span>
      </div>
      {preview.length > 0 && (
        <div className="chat-tool-card-body">
          <div className="chat-file-preview">
            {preview.join("\n")}
            {clipped ? `\n… ${lines.length - FILE_PREVIEW_LINES} more lines` : ""}
          </div>
        </div>
      )}
    </div>
  );
}

function ToolRow({
  item,
  awaitingIds,
}: {
  item: ToolCallItem;
  awaitingIds: ReadonlySet<string>;
}) {
  const awaiting = awaitingIds.has(item.toolCallId);
  const failed = item.status === "error";
  switch (item.kind) {
    case "execute-tool-call":
      return <ExecuteCard item={item} awaiting={awaiting} />;
    case "create-file-tool-call":
    case "modify-file-tool-call":
      return <FileChangeCard item={item} awaiting={awaiting} />;
    case "read-tool-call":
    case "delete-file-tool-call": {
      const path = item.path ?? item.title;
      return (
        <ToolLine
          status={item.status}
          awaiting={awaiting}
          verb={item.kind === "read-tool-call" ? "Read" : "Delete"}
          subject={basename(path)}
          detail={dirname(path)}
          error={failed}
        />
      );
    }
    case "search-tool-call":
      return (
        <ToolLine
          status={item.status}
          awaiting={awaiting}
          verb="Search"
          subject={item.query ?? item.title}
          detail={item.matchCount != null ? `${item.matchCount} matches` : undefined}
          error={failed}
        />
      );
    case "mcp-tool-call":
      return (
        <ToolLine
          status={item.status}
          awaiting={awaiting}
          verb="MCP"
          subject={item.server ? `${item.server} · ${item.tool ?? item.title}` : item.title}
          detail={item.inputSummary}
          error={failed}
        />
      );
    case "web-fetch-tool-call":
      return (
        <ToolLine
          status={item.status}
          awaiting={awaiting}
          verb="Fetch"
          subject={item.pageTitle || item.url || item.title}
          detail={item.pageTitle ? item.url : undefined}
          error={failed}
        />
      );
    case "spawn-subagent-tool-call":
      return (
        <ToolLine
          status={item.status}
          awaiting={awaiting}
          verb="Agent"
          subject={item.name ?? item.title}
          detail={item.background ? "background" : undefined}
          error={failed}
        />
      );
    case "create-plan-tool-call":
      return (
        <ToolLine
          status={item.status}
          awaiting={awaiting}
          verb="Plan"
          subject="Updated the plan"
          error={failed}
        />
      );
    default:
      return (
        <ToolLine
          status={item.status}
          awaiting={awaiting}
          verb={item.toolKind ?? "Tool"}
          subject={item.name ?? item.title}
          detail={item.inputSummary}
          error={failed}
        />
      );
  }
}

function GroupRow({
  group,
  awaitingIds,
}: {
  group: ToolGroupItem;
  awaitingIds: ReadonlySet<string>;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="chat-group">
      <button
        className="chat-tool-card-header"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
      >
        <StatusGlyph status={group.status} awaiting={false} />
        <span className="chat-tool-card-title">{group.label}</span>
        <span className="chat-tool-detail">{group.children.length}</span>
        <span className={`chat-chevron${open ? " open" : ""}`}>›</span>
      </button>
      {open && (
        <div className="chat-group-children">
          {group.children.map((child) => (
            <NodeRow key={nodeId(child)} node={child} awaitingIds={awaitingIds} />
          ))}
        </div>
      )}
    </div>
  );
}

function nodeId(node: ToolNode): string {
  return node.id;
}

function NodeRow({
  node,
  awaitingIds,
}: {
  node: ToolNode;
  awaitingIds: ReadonlySet<string>;
}) {
  if (node.kind === "tool-group") {
    return <GroupRow group={node} awaitingIds={awaitingIds} />;
  }
  return <ToolRow item={node} awaitingIds={awaitingIds} />;
}

// -- turns ----------------------------------------------------------------------

function ItemRow({
  item,
  awaitingIds,
  proposalProjectId,
}: {
  item: TranscriptItem;
  awaitingIds: ReadonlySet<string>;
  proposalProjectId?: string | null;
}) {
  switch (item.kind) {
    case "message":
      return <MessageRow item={item} proposalProjectId={proposalProjectId} />;
    case "thinking":
      return <ThinkingRow item={item} />;
    case "tool-group":
      return <GroupRow group={item} awaitingIds={awaitingIds} />;
    default:
      return <ToolRow item={item} awaitingIds={awaitingIds} />;
  }
}

const OUTCOME_LABEL: Record<string, string> = {
  cancelled: "Turn cancelled",
  error: "Turn failed",
  interrupted: "Turn interrupted",
};

export function TurnBlock({
  turn,
  awaitingIds,
  proposalProjectId,
}: {
  turn: TranscriptTurn;
  awaitingIds: ReadonlySet<string>;
  proposalProjectId?: string | null;
}) {
  const outcome = turn.outcome;
  const failed = outcome?.kind === "error";
  return (
    <div className="chat-turn" data-turn-id={turn.id}>
      {turn.items.map((item) => (
        <ItemRow
          key={item.id}
          item={item}
          awaitingIds={awaitingIds}
          proposalProjectId={proposalProjectId}
        />
      ))}
      {outcome && outcome.kind !== "done" && (
        <div className={`chat-outcome${failed ? " error" : ""}`}>
          {OUTCOME_LABEL[outcome.kind]}
          {outcome.reason ? ` (${outcome.reason.replace(/_/g, " ")})` : ""}
        </div>
      )}
    </div>
  );
}

/** Settled turns are immutable once committed (the reducer appends, never
 * edits) — item count + outcome kind catch the only transitions a committed
 * snapshot can carry, so streaming appends skip re-rendering history. */
export const SettledTurn = memo(
  TurnBlock,
  (prev, next) =>
    prev.turn.id === next.turn.id &&
    prev.turn.items.length === next.turn.items.length &&
    prev.turn.outcome?.kind === next.turn.outcome?.kind,
);
