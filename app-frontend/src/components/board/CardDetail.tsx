// Card detail (E17-02, #56 + dogfood): clicking a board card swaps the
// sheet to this inspector — the lane column carries its dot, the agent
// row dispatches or reattaches, and the ticket body edits commit-card
// style (E4-06 pattern): dirty title/body behind an explicit Save, busy
// phase, inline errors, draft retained on failure for retry.

import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import {
  issueDelete,
  issueDispatch,
  issueLink,
  issueList,
  issueUnlink,
  issueUpdate,
  onFartcodeEvent,
  terminalOpenAgent,
  terminalWrite,
  type IssueDto,
  type Lane,
  type TaskDto,
} from "../../lib/tauri";
import { useSidebar } from "../../store/sidebar";
import { useUi } from "../../store/ui";
import { IconClose } from "../icons";
import { LANE_LABEL } from "./BoardView";

/** Lane → linked-task status mapping used for the lane dot. */
const LANE_DOT_STATUS: Record<Lane, string | null> = {
  backlog: null,
  ready: null,
  in_progress: "in_progress",
  in_review: "review",
  done: "done",
};

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
  const [saving, setSaving] = useState(false);
  const [newAc, setNewAc] = useState("");
  const [edgeTarget, setEdgeTarget] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [dispatching, setDispatching] = useState(false);
  const projectTasks = useSidebar((s) => s.tasksByProject[projectId]);
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
    const unlisten = onFartcodeEvent((ev) => {
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
      // Task status changes recolor the lane dot.
      if (ev.type === "task:deleted" || ev.type === "task:status_changed") {
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

  // Commit-card pattern: dirty draft + explicit Save (busy phase,
  // inline error, draft retained for retry).
  const dirty =
    issue !== null &&
    (title.trim() !== issue.title || body !== (issue.body ?? ""));

  const save = () => {
    if (!dirty || saving) return;
    setSaving(true);
    setError(null);
    void apply(issueUpdate(issueId, { title: title.trim(), body: body || null })).finally(
      () => setSaving(false),
    );
  };

  const setAcceptance = (items: string[]) =>
    void apply(issueUpdate(issueId, { acceptance: items }));

  const linkedTask: TaskDto | undefined = issue?.linkedTaskId
    ? (projectTasks ?? []).find((t) => t.id === issue.linkedTaskId)
    : undefined;

  /** Dispatch the card (E17-03) — spawn or reattach, then hand off to
   * the task view. Errors surface inline. */
  const dispatch = async () => {
    if (!issue || dispatching) return;
    setDispatching(true);
    setError(null);
    try {
      const outcome = await issueDispatch(issueId);
      if (outcome.reattached) {
        useSidebar.getState().switchToTask(outcome.task);
        return;
      }
      const terminalId = await terminalOpenAgent(outcome.task.id, outcome.provider, 24, 80);
      await terminalWrite(terminalId, `\u001b[200~${outcome.prompt}\u001b[201~\r`);
      useSidebar.getState().switchToTask(outcome.task);
    } catch (e) {
      setError(String(e));
    } finally {
      setDispatching(false);
    }
  };

  const openTask = () => {
    const task = linkedTask ?? (projectTasks ?? []).find((t) => t.id === issue?.linkedTaskId);
    if (task) useSidebar.getState().switchToTask(task);
  };

  if (!issue && !error) {
    return (
      <aside className="card-detail">
        <p className="card-detail-loading muted">Loading…</p>
      </aside>
    );
  }

  const blockable = siblings.filter(
    (s) => s.id !== issueId && !(issue?.blockers ?? []).some((b) => b.id === s.id),
  );

  const dotStatus = issue
    ? linkedTask?.status ?? LANE_DOT_STATUS[issue.lane]
    : null;

  return (
    <aside className="card-detail" data-issue-id={issueId}>
      <header className="card-detail-header">
        <span className="card-detail-lane">
          <span
            className={`card-detail-lane-dot${dotStatus ? ` status-${dotStatus}` : ""}`}
          />
          {LANE_LABEL[issue?.lane ?? "backlog"] ?? issue?.lane}
        </span>
        <button className="card-detail-close" onClick={close} aria-label="Close detail">
          <IconClose size={10} />
        </button>
      </header>

      {error && (
        <p className="error card-detail-error" role="alert">
          {error}
        </p>
      )}

      {issue && (
        <div className="card-detail-scroll">
          <div className="card-detail-body">
            <div className="card-detail-agent-row">
              {issue.provider && (
                <span className="card-detail-agent">
                  {issue.provider}
                  {issue.model ? <em>· {issue.model}</em> : null}
                </span>
              )}
              {issue.blocked && (
                <span className="card-detail-blocked-note">
                  blocked by {issue.blockers.length}
                </span>
              )}
              {linkedTask ? (
                <button className="primary card-detail-dispatch" onClick={openTask}>
                  Open task
                </button>
              ) : (
                <button
                  className="primary card-detail-dispatch"
                  disabled={dispatching}
                  onClick={() => void dispatch()}
                  title="Create a worktree and launch an agent for this issue"
                >
                  {dispatching ? "Dispatching…" : "Dispatch"}
                </button>
              )}
            </div>

            <input
              className="card-detail-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && save()}
              aria-label="Issue title"
            />
            <textarea
              className="card-detail-body-edit"
              value={body}
              placeholder="Describe the ticket…"
              rows={6}
              onChange={(e) => setBody(e.target.value)}
              aria-label="Issue description"
            />
            <div className="card-detail-save">
              {dirty && (
                <span className="card-detail-dirty" aria-live="polite">
                  Unsaved changes
                </span>
              )}
              <button
                className="primary"
                disabled={!dirty || saving}
                onClick={save}
              >
                {saving ? "Saving…" : "Save"}
              </button>
            </div>

            <dl className="card-detail-meta">
              {issue.externalRef && (
                <div className="card-detail-meta-row">
                  <dt>Source</dt>
                  <dd>
                    <button
                      className="card-detail-link"
                      onClick={() => void open(issue.externalRef!).catch(() => {})}
                    >
                      {ghLabel(issue.externalRef)}
                    </button>
                  </dd>
                </div>
              )}
              {issue.prdPath && (
                <div className="card-detail-meta-row">
                  <dt>PRD</dt>
                  <dd>
                    <code>{issue.prdPath}</code>
                    {issue.prdSection ? <em> · {issue.prdSection}</em> : ""}
                  </dd>
                </div>
              )}
              {linkedTask && (
                <div className="card-detail-meta-row">
                  <dt>Task</dt>
                  <dd>
                    <button className="card-detail-link" onClick={openTask}>
                      {linkedTask.name}
                    </button>
                    <em> · {linkedTask.status.replace("_", " ")}</em>
                  </dd>
                </div>
              )}
              {issue.createdAt && (
                <div className="card-detail-meta-row">
                  <dt>Created</dt>
                  <dd>{stamp(issue.createdAt)}</dd>
                </div>
              )}
              {issue.updatedAt && issue.updatedAt !== issue.createdAt && (
                <div className="card-detail-meta-row">
                  <dt>Updated</dt>
                  <dd>{stamp(issue.updatedAt)}</dd>
                </div>
              )}
            </dl>

            <h3>Acceptance</h3>
            {issue.acceptance.length === 0 ? (
              <p className="card-detail-empty muted">No criteria yet.</p>
            ) : (
              <ul className="card-detail-ac">
                {issue.acceptance.map((ac, i) => (
                  <li key={i}>
                    <span className="card-detail-ac-text">{ac}</span>
                    <button
                      className="row-remove"
                      aria-label="Remove criterion"
                      onClick={() =>
                        setAcceptance(issue.acceptance.filter((_, j) => j !== i))
                      }
                    >
                      <IconClose size={9} />
                    </button>
                  </li>
                ))}
              </ul>
            )}
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
            {issue.blockers.length === 0 ? (
              <p className="card-detail-empty muted">Nothing blocks this issue.</p>
            ) : (
              <ul className="card-detail-edges">
                {issue.blockers.map((b) => (
                  <li key={b.id}>
                    <span className="card-detail-edge-title">{b.title}</span>
                    <em>{LANE_LABEL[b.lane] ?? b.lane}</em>
                    <button
                      className="row-remove"
                      aria-label={`Remove blocker ${b.title}`}
                      onClick={() => void apply(issueUnlink(issueId, b.id))}
                    >
                      <IconClose size={9} />
                    </button>
                  </li>
                ))}
              </ul>
            )}
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
          </div>

          <div className="card-detail-footer">
            {confirmDelete ? (
              <>
                <span className="card-detail-confirm-note">Delete this issue?</span>
                <button
                  className="danger"
                  onClick={() =>
                    void issueDelete(issueId)
                      .then(close)
                      .catch((e) => setError(String(e)))
                  }
                >
                  Delete
                </button>
                <button onClick={() => setConfirmDelete(false)}>Keep</button>
              </>
            ) : (
              <button
                className="card-detail-delete"
                onClick={() => setConfirmDelete(true)}
              >
                Delete issue
              </button>
            )}
          </div>
        </div>
      )}
    </aside>
  );
}

/** "https://github.com/o/r/issues/12" → "o/r#12"; anything else passes
 * through truncated. */
function ghLabel(url: string): string {
  const m = url.match(/github\.com\/([^/]+\/[^/]+)\/issues\/(\d+)/);
  return m ? `${m[1]}#${m[2]}` : url.replace(/^https?:\/\//, "");
}

/** Backend timestamps are RFC3339; render "Aug 7, 11:04". */
function stamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}
