// Changes sidebar (E4-03): right-side panel for the selected task —
// Staged/Changed sections with status glyphs, per-row stage/unstage/
// discard actions (discard confirms via the ui store modal), and
// click-to-open diff tabs (single = preview, double = persistent).
// Refresh is event-driven through the changes store; nothing here polls.
import { useEffect, useState } from "react";
import { openDiffTab } from "../lib/diff-tabs";
import { useChanges } from "../store/changes";
import { useCommitState } from "../store/commit-state";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";
import { hint } from "../lib/useCommands";
import { provisionTask, type DiffSide, type GitChangeDto } from "../lib/tauri";
import CommitCard from "./CommitCard";
import { IconBranch, IconClose, IconDiscard, IconMinus, IconPlus } from "./icons";

const GLYPH: Record<GitChangeDto["status"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  conflicted: "!",
};

export default function ChangesSidebar() {
  const open = useUi((s) => s.changesOpen);
  const setOpen = useUi((s) => s.setChangesOpen);
  const { tasksByProject, selectedProjectId, selectedTaskId } = useSidebar();
  const task = selectedProjectId
    ? (tasksByProject[selectedProjectId] ?? []).find((t) => t.id === selectedTaskId)
    : undefined;
  const taskId = task?.id ?? null;
  const workspaceId = task?.workspaceId ?? null;
  const entry = useChanges((s) => (workspaceId ? s.byWorkspace[workspaceId] : undefined));
  const [provisioning, setProvisioning] = useState(false);

  useEffect(() => {
    if (open && workspaceId) {
      void useChanges.getState().ensure(workspaceId);
      void useCommitState.getState().ensure(workspaceId);
    }
  }, [open, workspaceId]);

  if (!open || !taskId) return null;

  const snapshot = entry?.snapshot ?? null;
  const changeCount = snapshot ? snapshot.staged.length + snapshot.unstaged.length : 0;

  return (
    <aside className="changes-panel">
      <div className="changes-header">
        <IconBranch size={12} />
        <span className="changes-title">Changes</span>
        {snapshot && !snapshot.truncated && (
          <span className="changes-count">{changeCount}</span>
        )}
        <div className="header-actions">
          {workspaceId && snapshot && snapshot.unstaged.length > 0 && (
            <button
              className="changes-stage-all"
              title="Stage all changes"
              onClick={() => void useChanges.getState().stageAll(workspaceId)}
            >
              Stage all
            </button>
          )}
          <button
            title={`Close changes (${hint("toggle-changes") || "⌘⇧1"})`}
            onClick={() => setOpen(false)}
          >
            <IconClose size={10} />
          </button>
        </div>
      </div>

      {!workspaceId ? (
        <p className="changes-empty muted">
          This task has no workspace yet — changes appear once it's provisioned.
        </p>
      ) : entry?.error && entry.error.includes("workspace has no local path") ? (
        <div className="changes-empty">
          <p className="muted">This task's workspace isn't on disk yet.</p>
          <button
            className="primary"
            disabled={provisioning}
            onClick={async () => {
              setProvisioning(true);
              try {
                await provisionTask(taskId);
                await useChanges.getState().refetch(workspaceId);
              } catch (e) {
                useChanges.setState((s) => ({
                  byWorkspace: {
                    ...s.byWorkspace,
                    [workspaceId]: {
                      ...(s.byWorkspace[workspaceId] ?? {
                        snapshot: null,
                        loading: false,
                        error: null,
                      }),
                      error: String(e),
                    },
                  },
                }));
              } finally {
                setProvisioning(false);
              }
            }}
          >
            {provisioning ? "Provisioning…" : "Provision workspace"}
          </button>
        </div>
      ) : entry?.loading && !snapshot ? (
        <p className="changes-empty muted">Reading worktree…</p>
      ) : entry?.error && !snapshot ? (
        <div className="changes-error">
          <p className="error">{entry.error}</p>
          <button onClick={() => void useChanges.getState().refetch(workspaceId)}>
            Retry
          </button>
        </div>
      ) : snapshot?.truncated ? (
        <p className="changes-empty muted">
          Too many changed files to show — refine on the command line.
        </p>
      ) : snapshot && changeCount === 0 ? (
        <p className="changes-empty muted">No changes</p>
      ) : snapshot ? (
        <div className="changes-body">
          <ChangeSection
            title="Staged"
            side="staged"
            changes={snapshot.staged}
            taskId={taskId}
            workspaceId={workspaceId}
          />
          <ChangeSection
            title="Changed"
            side="unstaged"
            changes={snapshot.unstaged}
            taskId={taskId}
            workspaceId={workspaceId}
          />
        </div>
      ) : null}

      {workspaceId && snapshot && <CommitCard workspaceId={workspaceId} />}
    </aside>
  );
}

function ChangeSection({
  title,
  side,
  changes,
  taskId,
  workspaceId,
}: {
  title: string;
  side: DiffSide;
  changes: GitChangeDto[];
  taskId: string;
  workspaceId: string;
}) {
  return (
    <section className="changes-section">
      <h3>
        {title}
        <span className="changes-count">{changes.length}</span>
      </h3>
      {changes.length > 0 && (
        <ul>
          {changes.map((change) => (
            <ChangeRow
              key={`${side}:${change.path}`}
              change={change}
              side={side}
              taskId={taskId}
              workspaceId={workspaceId}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function ChangeRow({
  change,
  side,
  taskId,
  workspaceId,
}: {
  change: GitChangeDto;
  side: DiffSide;
  taskId: string;
  workspaceId: string;
}) {
  const setDiscardTarget = useUi((s) => s.setDiscardTarget);

  return (
    <li
      className={`change-row status-${change.status}`}
      title={change.path}
      onClick={(e) =>
        openDiffTab({
          taskId,
          workspaceId,
          path: change.path,
          origPath: change.origPath,
          side,
          preview: e.detail < 2,
        })
      }
    >
      <span className="change-glyph" aria-label={change.status}>
        {GLYPH[change.status]}
      </span>
      <span className="change-path">
        {change.origPath ? (
          <>
            <span className="change-orig">{change.origPath}</span>
            <span className="change-arrow">→</span>
            {change.path}
          </>
        ) : (
          change.path
        )}
      </span>
      {(change.additions > 0 || change.deletions > 0) && (
        <span className="change-stats">
          {change.additions > 0 && <em className="add">+{change.additions}</em>}
          {change.deletions > 0 && <em className="del">−{change.deletions}</em>}
        </span>
      )}
      <span className="change-actions" onClick={(e) => e.stopPropagation()}>
        {side === "unstaged" ? (
          <>
            <button
              title="Stage"
              onClick={() => void useChanges.getState().stage(workspaceId, [change.path])}
            >
              <IconPlus size={10} />
            </button>
            <button
              className="danger"
              title="Discard changes"
              onClick={() =>
                setDiscardTarget({
                  workspaceId,
                  paths: [change.path],
                  hasUntracked: change.status === "added",
                })
              }
            >
              <IconDiscard size={10} />
            </button>
          </>
        ) : (
          <button
            title="Unstage"
            onClick={() => void useChanges.getState().unstage(workspaceId, [change.path])}
          >
            <IconMinus size={10} />
          </button>
        )}
      </span>
    </li>
  );
}
