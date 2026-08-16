// Changes sidebar (E4-03, restyled to design_handoff_v2 §5d): right panel
// for the selected task — 46px header ("Changes" + task/branch ref), mono
// Changed/Staged sections with status letters, single-key staging while the
// panel has focus (a stage all / s stage / u unstage / d discard after an
// inline overlay-card confirm), the commit card, and the footer hint.
// Git plumbing verbs (fetch/pull/push/publish) live in ⌘K, not here.
// Refresh is event-driven through the changes store; nothing here polls.
import { useEffect, useRef, useState } from "react";
import { openDiffTab } from "../lib/diff-tabs";
import { useChanges } from "../store/changes";
import { useCommitState } from "../store/commit-state";
import { usePr } from "../store/pr";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";
import { provisionTask, type DiffSide, type GitChangeDto } from "../lib/tauri";
import CommitCard from "./CommitCard";
import GitFooter from "./GitFooter";
import PullRequestPanel from "./PullRequestPanel";
import CardDetail from "./board/CardDetail";
import FileTreeView from "./FileTreeView";
import ProjectChatPanel from "./projectChat/ProjectChatPanel";
import TaskChatPanel from "./TaskChatPanel";
import { useGutterResize } from "../lib/useGutterResize";

const GLYPH: Record<GitChangeDto["status"], string> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  conflicted: "C",
};

interface DiscardTargetLocal {
  path: string;
  hasUntracked: boolean;
}

/** "+n −n" stats fragment ("" when the row has no counted lines). */
function statLine(change: GitChangeDto): string {
  const parts: string[] = [];
  if (change.additions > 0) parts.push(`+${change.additions}`);
  if (change.deletions > 0) parts.push(`−${change.deletions}`);
  return parts.join(" ");
}

export default function ChangesSidebar() {
  const open = useUi((s) => s.changesOpen);
  const detailIssueId = useUi((s) => s.boardDetailIssueId);
  const projectChatOpen = useUi((s) => s.projectChatOpen);
  const taskChatOpen = useUi((s) => s.taskChatOpen);
  const fileTreeOpen = useUi((s) => s.fileTreeOpen);
  const { projects, tasksByProject, selectedProjectId, selectedTaskId } = useSidebar();
  const task = selectedProjectId
    ? (tasksByProject[selectedProjectId] ?? []).find((t) => t.id === selectedTaskId)
    : undefined;
  const taskId = task?.id ?? null;
  // Project view (no task): the sidesheet shows the project ROOT checkout
  // via the repository workspace (E17 dogfood: GitHub icon opens this).
  const project = selectedProjectId
    ? projects.find((p) => p.id === selectedProjectId)
    : undefined;
  const workspaceId = task?.workspaceId ?? project?.repositoryWorkspaceId ?? null;
  const entry = useChanges((s) => (workspaceId ? s.byWorkspace[workspaceId] : undefined));
  const commitEntry = useCommitState((s) =>
    workspaceId ? s.byWorkspace[workspaceId] : undefined,
  );
  const [provisioning, setProvisioning] = useState(false);
  const [panelTab, setPanelTab] = useState<"changes" | "prs">("changes");
  // Single-key target: the row last hovered or focused (§5d "s = stage
  // focused/hovered file").
  const [active, setActive] = useState<{ side: DiffSide; path: string } | null>(null);
  const [discardTarget, setDiscardTarget] = useState<DiscardTargetLocal | null>(null);
  const mainRef = useRef<HTMLDivElement | null>(null);
  const resize = useGutterResize(400, 280, 640, -1);

  useEffect(() => {
    if (open && workspaceId) {
      void useChanges.getState().ensure(workspaceId);
      void useCommitState.getState().ensure(workspaceId);
    }
  }, [open, workspaceId]);

  // E17 project scope: the sheet shows ONE of changes | PM chat (they
  // alternate, never stack), and a board card click swaps the whole sheet
  // to card detail. Task scope mirrors it: changes ⇄ task chat.
  const showDetail = !taskId && detailIssueId !== null;
  const showChat = !taskId && projectChatOpen && selectedProjectId !== null;
  const showTaskChat = Boolean(taskId && taskChatOpen);
  // File tree mode: same sheet, alternates with changes/chat (never stacks).
  const showFileTree = Boolean(fileTreeOpen && workspaceId);

  // §5d single keys need DOM focus on the panel; give it focus when the
  // changes surface appears (open / tab / sub-view edge), and only on that
  // edge — re-renders mid-session never steal focus from an input elsewhere.
  const changesVisible =
    open &&
    (taskId !== null || workspaceId !== null) &&
    panelTab === "changes" &&
    !showDetail &&
    !showChat &&
    !showTaskChat &&
    !showFileTree;
  useEffect(() => {
    if (changesVisible) mainRef.current?.focus();
  }, [changesVisible]);

  // DESIGN.md: the right panel (PM chat / card detail) is 400px; the drag
  // handle still wins above the floor. Published to the ui store so the
  // diff view reserves this width instead of sitting under the sheet.
  const sheetWidth =
    showDetail || showChat || showTaskChat
      ? Math.max(resize.width, 400)
      : resize.width;
  useEffect(() => {
    useUi.getState().setSheetWidth(sheetWidth);
  }, [sheetWidth]);

  if (!open || (!taskId && !workspaceId)) return null;

  const snapshot = entry?.snapshot ?? null;
  const changeCount = snapshot ? snapshot.staged.length + snapshot.unstaged.length : 0;

  // Header ref: "#<task> · <branch>" when known. Local tasks carry no
  // number today (linkedIssue is unset), so this degrades to the branch.
  const linked = task?.linkedIssue as { number?: number } | null | undefined;
  const headerRef = [
    linked && typeof linked.number === "number" ? `#${linked.number}` : null,
    commitEntry?.state?.branch ?? null,
  ]
    .filter(Boolean)
    .join(" · ");

  const activeChange = (side: DiffSide): GitChangeDto | undefined => {
    if (!active || active.side !== side || !snapshot) return undefined;
    const list = side === "unstaged" ? snapshot.unstaged : snapshot.staged;
    return list.find((c) => c.path === active.path);
  };

  const doDiscard = () => {
    if (!discardTarget || !workspaceId) return;
    const { path } = discardTarget;
    setDiscardTarget(null);
    void useChanges.getState().discard(workspaceId, [path]);
    mainRef.current?.focus();
  };

  const cancelDiscard = () => {
    setDiscardTarget(null);
    mainRef.current?.focus();
  };

  // §5d single keys, live while the panel (or a row in it) has focus.
  // Editable targets (commit message, add-remote inputs) keep their keys.
  const onPanelKey = (e: React.KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (discardTarget) {
      // The overlay card handles d/esc itself when focused; this is the
      // fallback for keys landing elsewhere in the panel.
      if (e.key === "d") {
        e.preventDefault();
        e.stopPropagation();
        doDiscard();
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        cancelDiscard();
      }
      return;
    }
    if (panelTab === "prs") {
      // §5e `r` = refresh, delegated here so it works before the .fc-pr
      // element itself has focus. PullRequestPanel's own handler
      // preventDefaults without stopping propagation — skip when it
      // already ran so one keypress never syncs twice.
      if (e.key === "r" && !e.shiftKey && workspaceId && !e.defaultPrevented) {
        e.preventDefault();
        e.stopPropagation();
        void usePr.getState().sync(workspaceId);
      }
      return;
    }
    if (panelTab !== "changes" || !workspaceId || !snapshot) return;
    let handled = true;
    if (e.key === "a") {
      if (snapshot.unstaged.length > 0) void useChanges.getState().stageAll(workspaceId);
    } else if (e.key === "s") {
      const c = activeChange("unstaged");
      if (c) void useChanges.getState().stage(workspaceId, [c.path]);
      else handled = false;
    } else if (e.key === "u") {
      const c = activeChange("staged");
      if (c) void useChanges.getState().unstage(workspaceId, [c.path]);
      else handled = false;
    } else if (e.key === "d") {
      const c = activeChange("unstaged");
      if (c) setDiscardTarget({ path: c.path, hasUntracked: c.status === "added" });
      else handled = false;
    } else {
      handled = false;
    }
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
      mainRef.current?.focus();
    }
  };

  return (
    <aside
      className={`changes-panel${showDetail ? " detail-open" : ""}`}
      style={{ width: sheetWidth }}
    >
      <div className="gutter-handle sheet-handle" {...resize.bind} />
      {showDetail ? (
        <CardDetail projectId={selectedProjectId!} issueId={detailIssueId} />
      ) : showChat ? (
        // Keyed: the panel holds the resolved conversation id in state, and
        // a project switch must not leave the PREVIOUS project's PM agent
        // on screen (and receiving prompts) until the new one resolves.
        <ProjectChatPanel key={selectedProjectId} projectId={selectedProjectId} />
      ) : showFileTree ? (
        <div className="fc-changes-main sheet-file-tree">
          <div className="fc-changes-header">
            <span className="fc-changes-title">Files</span>
            <span className="fc-changes-header-side">
              {headerRef && (
                <span className="fc-changes-ref" title={headerRef}>
                  {headerRef}
                </span>
              )}
              <button
                className="fc-sheet-close"
                aria-label="Close panel"
                title="Close panel"
                onClick={() => useUi.getState().setChangesOpen(false)}
              >
                ×
              </button>
            </span>
          </div>
          <FileTreeView taskId={taskId ?? ""} workspaceId={workspaceId!} active />
        </div>
      ) : taskId && taskChatOpen ? (
        <TaskChatPanel key={taskId} projectId={selectedProjectId!} taskId={taskId} />
      ) : (
        <div
          className="fc-changes-main"
          ref={mainRef}
          tabIndex={0}
          role="region"
          aria-label="Changes"
          onKeyDown={onPanelKey}
        >
          <div className="fc-changes-header">
            <span className="fc-changes-title">Changes</span>
            <span className="fc-changes-header-side">
              {headerRef && (
                <span className="fc-changes-ref" title={headerRef}>
                  {headerRef}
                </span>
              )}
              <button
                className="fc-sheet-close"
                aria-label="Close panel"
                title="Close panel"
                onClick={() => useUi.getState().setChangesOpen(false)}
              >
                ×
              </button>
            </span>
          </div>

          {workspaceId && (
            <div className="fc-panel-tabs" role="tablist">
              <span
                role="tab"
                aria-selected={panelTab === "changes"}
                className={`fc-panel-tab${panelTab === "changes" ? " active" : ""}`}
                onClick={() => setPanelTab("changes")}
              >
                Changed
              </span>
              <span
                role="tab"
                aria-selected={panelTab === "prs"}
                className={`fc-panel-tab${panelTab === "prs" ? " active" : ""}`}
                onClick={() => setPanelTab("prs")}
              >
                Pull Requests
              </span>
            </div>
          )}

          {workspaceId && panelTab === "prs" ? (
            <div className="changes-scroll">
              <PullRequestPanel workspaceId={workspaceId} />
            </div>
          ) : (
            <div className="changes-scroll fc-changes-scroll">
              {!workspaceId ? (
                <p className="changes-empty">
                  This task has no workspace yet — changes appear once it's provisioned.
                </p>
              ) : taskId && entry?.error && entry.error.includes("workspace has no local path") ? (
                <div className="changes-empty">
                  <p>This task's workspace isn't on disk yet.</p>
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
                <p className="changes-empty">Reading worktree…</p>
              ) : entry?.error && !snapshot ? (
                <div className="changes-error">
                  <p className="error">{entry.error}</p>
                  <button onClick={() => void useChanges.getState().refetch(workspaceId)}>
                    Retry
                  </button>
                </div>
              ) : snapshot?.truncated ? (
                <p className="changes-empty">
                  Too many changed files to show — refine on the command line.
                </p>
              ) : snapshot && changeCount === 0 ? (
                <p className="changes-empty">No changes</p>
              ) : snapshot ? (
                <>
                  <ChangeSection
                    title="Changed"
                    side="unstaged"
                    changes={snapshot.unstaged}
                    taskId={taskId}
                    workspaceId={workspaceId}
                    active={active}
                    onActive={setActive}
                    onRequestDiscard={(c) =>
                      setDiscardTarget({ path: c.path, hasUntracked: c.status === "added" })
                    }
                  />
                  <ChangeSection
                    title="Staged"
                    side="staged"
                    changes={snapshot.staged}
                    taskId={taskId}
                    workspaceId={workspaceId}
                    active={active}
                    onActive={setActive}
                    onRequestDiscard={null}
                  />
                </>
              ) : null}

            </div>
          )}

          {/* The commit surface is DOCKED, not the last row of the list.
              Inside the scroller it drifted below the fold the moment a task
              touched more files than the sheet is tall — the input you need
              after reviewing them is the one thing that must never require a
              scroll to reach. Sibling of the scroller, so the file list
              yields (flex: 1) and the dock keeps its height. */}
          {workspaceId && snapshot && panelTab !== "prs" && (
            <div className="fc-changes-foot">
              <CommitCard workspaceId={workspaceId} />
              <GitFooter workspaceId={workspaceId} />
            </div>
          )}

          {discardTarget && workspaceId && (
            <DiscardConfirm
              target={discardTarget}
              onCancel={cancelDiscard}
              onConfirm={doDiscard}
            />
          )}
        </div>
      )}
    </aside>
  );
}

/** §5d inline confirm: overlay card in the sidebar — esc cancel / d discard. */
function DiscardConfirm({
  target,
  onCancel,
  onConfirm,
}: {
  target: DiscardTargetLocal;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);
  return (
    <div
      className="fc-confirm"
      ref={ref}
      tabIndex={-1}
      role="alertdialog"
      aria-label={`Discard changes to ${target.path}`}
      onKeyDown={(e) => {
        if (e.metaKey || e.ctrlKey || e.altKey) return;
        if (e.key === "d") {
          e.preventDefault();
          e.stopPropagation();
          onConfirm();
        } else if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <div className="fc-confirm-body">
        Discard changes to <span className="fc-confirm-path">{target.path}</span>?
        <span className="fc-confirm-sub">
          {target.hasUntracked
            ? "The file is untracked — it moves to .fartCode/trash."
            : "Unstaged edits to this file are lost."}
        </span>
      </div>
      <div className="fc-confirm-footer">
        <span role="button" className="fc-confirm-act" onClick={onCancel}>
          <span className="fc-key">esc</span> cancel
        </span>
        <span role="button" className="fc-confirm-act" onClick={onConfirm}>
          <span className="fc-key">d</span> discard
        </span>
      </div>
    </div>
  );
}

function ChangeSection({
  title,
  side,
  changes,
  taskId,
  workspaceId,
  active,
  onActive,
  onRequestDiscard,
}: {
  title: string;
  side: DiffSide;
  changes: GitChangeDto[];
  /** Null at project scope (no task) — rows render without diff-tab
   * opening, which needs a task's tab surface. */
  taskId: string | null;
  workspaceId: string;
  active: { side: DiffSide; path: string } | null;
  onActive: (row: { side: DiffSide; path: string } | null) => void;
  /** Null on the staged section (discard is an unstaged verb). */
  onRequestDiscard: ((change: GitChangeDto) => void) | null;
}) {
  return (
    <section className="fc-section">
      <h3 className="fc-section-label">
        <span>{title}</span>
        <span className="fc-section-meta">
          {changes.length}
          {side === "unstaged" && changes.length > 0 && (
            <>
              {" · "}
              <span
                role="button"
                className="fc-act"
                aria-label="Stage all changes"
                onClick={() => void useChanges.getState().stageAll(workspaceId)}
              >
                a stage all
              </span>
            </>
          )}
        </span>
      </h3>
      {changes.length > 0 && (
        <ul className="fc-rows">
          {changes.map((change) => (
            <ChangeRow
              key={`${side}:${change.path}`}
              change={change}
              side={side}
              taskId={taskId}
              workspaceId={workspaceId}
              active={active?.side === side && active?.path === change.path}
              onActive={onActive}
              onRequestDiscard={onRequestDiscard}
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
  active,
  onActive,
  onRequestDiscard,
}: {
  change: GitChangeDto;
  side: DiffSide;
  taskId: string | null;
  workspaceId: string;
  active: boolean;
  onActive: (row: { side: DiffSide; path: string } | null) => void;
  onRequestDiscard: ((change: GitChangeDto) => void) | null;
}) {
  const conflicted = change.status === "conflicted";
  const stats = statLine(change);
  return (
    <li
      className={`fc-row ${side}${conflicted ? " conflicted" : ""}${active ? " active" : ""}${taskId ? "" : " no-diff"}`}
      tabIndex={-1}
      title={taskId ? change.path : `${change.path} — diff opens in the task view`}
      onMouseEnter={() => onActive({ side, path: change.path })}
      onMouseLeave={() => onActive(null)}
      onFocus={() => onActive({ side, path: change.path })}
      onClick={(e) => {
        if (!taskId) return;
        openDiffTab({
          taskId,
          workspaceId,
          path: change.path,
          origPath: change.origPath,
          side,
          preview: e.detail < 2,
        });
      }}
    >
      <span className="fc-row-status" aria-label={change.status}>
        {GLYPH[change.status]}
      </span>
      <span className="fc-row-path">
        {change.origPath ? (
          <>
            <span className="fc-row-orig">{change.origPath}</span>
            <span className="fc-row-arrow"> → </span>
            {change.path}
          </>
        ) : (
          change.path
        )}
      </span>
      <span className="fc-row-meta" onClick={(e) => e.stopPropagation()}>
        {conflicted ? (
          "conflict"
        ) : (
          <>
            {stats && `${stats} · `}
            {side === "unstaged" ? (
              <>
                <span
                  role="button"
                  className="fc-act"
                  aria-label={`Stage ${change.path}`}
                  onClick={() => void useChanges.getState().stage(workspaceId, [change.path])}
                >
                  s
                </span>
                {" · "}
                <span
                  role="button"
                  className="fc-act"
                  aria-label={`Discard changes to ${change.path}`}
                  onClick={() => onRequestDiscard?.(change)}
                >
                  d
                </span>
              </>
            ) : (
              <span
                role="button"
                className="fc-act"
                aria-label={`Unstage ${change.path}`}
                onClick={() => void useChanges.getState().unstage(workspaceId, [change.path])}
              >
                u
              </span>
            )}
          </>
        )}
      </span>
    </li>
  );
}
