// Comment thread panel (E4-10, §14): lists one file's line comments for the
// active diff side — rendered inside diff-body when a gutter marker is
// clicked (or after adding a note). Resolve ✓ is manual (§14 decision: a
// finished linked task shows "→ done" but does NOT auto-resolve). The
// linked-task badge tracks live status off the sidebar's task list
// (task:status_changed already keeps it fresh) and focuses the task on
// click.
import { useLineComments } from "../store/line-comments";
import { useSidebar } from "../store/sidebar";
import type { LineCommentDto } from "../lib/tauri";

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
  const threads = comments
    .filter((c) => c.filePath === filePath && c.sourceSide === side)
    .sort((a, b) => a.lineNumber - b.lineNumber);

  // Live linked-task status: sidebar store is the single source of truth
  // (refreshed by task:status_changed events).
  const taskById: Record<string, { name: string; status: string }> = {};
  for (const list of Object.values(tasksByProject)) {
    for (const t of list) taskById[t.id] = { name: t.name, status: t.status };
  }

  return (
    <div className="comment-thread" onKeyDown={(e) => e.key === "Escape" && onClose()}>
      <div className="comment-thread-header">
        <span>
          Comments · {filePath} ({side})
        </span>
        <button onClick={onClose} aria-label="Close comments">
          ✕
        </button>
      </div>
      {threads.length === 0 && <p className="muted">No comments on this side.</p>}
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
  taskById: Record<string, { name: string; status: string }>;
}) {
  const linked = comment.linkedTaskId ? taskById[comment.linkedTaskId] : null;
  const switchTo = useSidebar((s) => s.switchToTask);
  const tasksByProject = useSidebar((s) => s.tasksByProject);

  return (
    <div className={`comment-row${comment.resolved ? " resolved" : ""}`}>
      <div className="comment-row-head">
        <span className="comment-lines">
          {comment.lineNumber}
          {comment.lineEnd && comment.lineEnd !== comment.lineNumber
            ? `–${comment.lineEnd}`
            : ""}
        </span>
        {linked && (
          <button
            className={`comment-task-badge status-${linked.status}`}
            title={`Open task ${linked.name}`}
            onClick={() => {
              const task = Object.values(tasksByProject)
                .flat()
                .find((t) => t.id === comment.linkedTaskId);
              if (task) switchTo(task);
            }}
          >
            → Task: {linked.name} ●{" "}
            {linked.status === "done" ? "done" : linked.status}
          </button>
        )}
        {!linked && comment.linkedTaskId && (
          <span className="comment-task-badge status-deleted">→ task deleted</span>
        )}
        <span className="comment-row-actions">
          {!comment.resolved && (
            <button
              className="comment-resolve"
              title="Mark resolved"
              onClick={() => void useLineComments.getState().resolve(comment.id)}
            >
              ✓
            </button>
          )}
          {comment.resolved && <span className="comment-resolved-mark">✓ resolved</span>}
          <button
            className="comment-delete"
            title="Delete comment"
            onClick={() => void useLineComments.getState().remove(comment.id)}
          >
            🗑
          </button>
        </span>
      </div>
      <p className="comment-content">{comment.content}</p>
    </div>
  );
}
