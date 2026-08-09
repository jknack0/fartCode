// Comment thread panel (E4-10, §14 — restyled per design_handoff_v2 §5f):
// lists one file's line comments for the active diff side, floating over
// diff-body when a gutter marker is clicked (or after adding a note).
// Placed comment row = status dot mirroring the linked task's state
// (running pulse / needs-you hollow / failed red / neutral when done) +
// mono meta "note → <task> · <status> · <elapsed>" (the task is an --info
// link that opens it; the line range lives in the row tooltip, audit 22)
// + the comment title. The badge flips live when the
// linked task lands — the sidebar's task list (kept fresh by
// task:status_changed) is the single source of truth. Resolve ✓ is manual
// (§14 decision: a finished linked task does NOT auto-resolve). E4-11
// agent-authored comments keep attribution, folded into the meta voice as
// "<provider> · …" (no chip).
import { useEffect, useState } from "react";
import { useLineComments } from "../store/line-comments";
import { useSidebar } from "../store/sidebar";
import { commentAuthor, type LineCommentDto } from "../lib/tauri";

interface LinkedTask {
  name: string;
  status: string;
  statusChangedAt: string | null;
}

export default function CommentThread({
  taskId,
  filePath,
  side,
  onClose,
}: {
  taskId: string;
  filePath: string;
  side: "before" | "after";
  onClose: () => void;
}) {
  const comments = useLineComments((s) => s.byTask[taskId] ?? []);
  const tasksByProject = useSidebar((s) => s.tasksByProject);

  // Elapsed meta ("· 1m") stays fresh without any store churn: a slow
  // re-render tick, not an animation.
  const [, setTick] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => setTick((n) => n + 1), 30_000);
    return () => window.clearInterval(t);
  }, []);

  const threads = comments
    .filter((c) => c.filePath === filePath && c.sourceSide === side)
    .sort((a, b) => a.lineNumber - b.lineNumber);

  // Live linked-task status: sidebar store is the single source of truth
  // (refreshed by task:status_changed events).
  const taskById: Record<string, LinkedTask> = {};
  for (const list of Object.values(tasksByProject)) {
    for (const t of list) {
      taskById[t.id] = {
        name: t.name,
        status: t.status,
        statusChangedAt: t.statusChangedAt,
      };
    }
  }

  return (
    <div className="lc-thread" onKeyDown={(e) => e.key === "Escape" && onClose()}>
      <div className="lc-thread-header">
        <span className="lc-thread-label">comments</span>
        <span className="lc-thread-path" title={filePath}>
          {filePath}
          {side === "before" ? " · baseline" : ""}
        </span>
        <button className="lc-thread-close" onClick={onClose} aria-label="Close comments">
          ✗
        </button>
      </div>
      {threads.length === 0 && <p className="lc-empty">no comments on this side</p>}
      {threads.map((c) => (
        <CommentRow key={c.id} comment={c} taskById={taskById} />
      ))}
    </div>
  );
}

function CommentRow({
  comment,
  taskById,
}: {
  comment: LineCommentDto;
  taskById: Record<string, LinkedTask>;
}) {
  const linked = comment.linkedTaskId ? taskById[comment.linkedTaskId] : null;
  const switchTo = useSidebar((s) => s.switchToTask);
  const tasksByProject = useSidebar((s) => s.tasksByProject);
  const author = commentAuthor(comment.createdBy);

  // §5f: the dot mirrors the linked task's state via the shared .status-dot
  // classes; a plain note (or a resolved one) sits neutral.
  const dotStatus = comment.resolved ? "done" : (linked?.status ?? "todo");

  const lineRef =
    comment.lineEnd && comment.lineEnd !== comment.lineNumber
      ? `${comment.lineNumber}–${comment.lineEnd}`
      : `${comment.lineNumber}`;

  const openLinked = () => {
    const task = Object.values(tasksByProject)
      .flat()
      .find((t) => t.id === comment.linkedTaskId);
    if (task) switchTo(task);
  };

  const age = linked ? elapsed(linked.statusChangedAt ?? comment.createdAt) : "";

  return (
    <div
      className={`lc-row${comment.resolved ? " resolved" : ""}`}
      title={
        comment.lineEnd && comment.lineEnd !== comment.lineNumber
          ? `lines ${lineRef}`
          : `line ${lineRef}`
      }
    >
      <span className={`status-dot status-${dotStatus}`} />
      <div className="lc-row-body">
        <div className="lc-meta-row">
          <span className="lc-meta">
            {author.kind === "agent" && `${author.provider ?? "agent"} · `}
            note
            {linked && (
              <>
                {" → "}
                <button
                  className="lc-task-link"
                  title={`Open task ${linked.name}`}
                  onClick={openLinked}
                >
                  {linked.name}
                </button>
                {` · ${statusWord(linked.status)}`}
                {age && ` · ${age}`}
              </>
            )}
            {!linked && comment.linkedTaskId && " → task deleted"}
            {comment.resolved && " · resolved ✓"}
          </span>
          <span className="lc-row-actions">
            {!comment.resolved && (
              <button
                className="lc-action"
                title="Mark resolved"
                onClick={() => void useLineComments.getState().resolve(comment.id)}
              >
                ✓
              </button>
            )}
            <button
              className="lc-action danger"
              title="Delete comment"
              onClick={() => void useLineComments.getState().remove(comment.id)}
            >
              ✗
            </button>
          </span>
        </div>
        <div className="lc-title">{comment.content}</div>
      </div>
    </div>
  );
}

/** Meta vocabulary: the design says "running", the store says
 * "in_progress" — translate; everything else reads as-is. */
function statusWord(status: string): string {
  if (status === "in_progress") return "running";
  return status.replace(/_/g, " ");
}

/** Compact elapsed time for the meta line ("now" / "4m" / "2h" / "3d");
 * empty when the timestamp is missing or unparsable. */
function elapsed(iso: string | null): string {
  if (!iso) return "";
  const ms = Date.now() - Date.parse(iso);
  if (!Number.isFinite(ms) || ms < 0) return "";
  const m = Math.floor(ms / 60_000);
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}
