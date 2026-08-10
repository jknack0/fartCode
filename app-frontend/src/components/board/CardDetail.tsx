// Card detail (E17-02, #56 + dogfood; column-aware per E18-07): clicking a
// board card swaps the sheet to this inspector — the header names the
// card's COLUMN (resolved from board_columns, never a lane label table)
// and its dot reads the live agent, the agent row dispatches or reattaches,
// and the ticket body edits commit-card style (E4-06 pattern): dirty
// title/body behind an explicit Save, busy phase, inline errors, draft
// retained on failure for retry.
//
// E19-06 (#75, handoff v3 §8f) adds the Dossier group under the header:
// the app-written timeline over the agent-written section for the focused
// step. All of it is parsed in Rust (`fartcode_core::dossier_view`) — a
// dossier's structure is a trust boundary, and this file does not
// re-derive it.

import { type KeyboardEvent as ReactKeyboardEvent, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import {
  acpStart,
  dossierRead,
  issueDelete,
  issueDispatch,
  issueLink,
  issueList,
  issueUnlink,
  issueUpdate,
  listProviders,
  onFartcodeEvent,
  terminalOpenAgent,
  terminalWrite,
  type DossierDto,
  type DossierTimelineEntryDto,
  type IssueDto,
  type TaskDto,
} from "../../lib/tauri";
import { renderMarkdown } from "../../lib/markdown";
import {
  columnIdForIssue,
  blockerColumnName,
} from "../../lib/columnConfig";
import { useColumns } from "../../store/columns";
import { useConversations } from "../../store/conversations";
import { ensureDossierConsent } from "../../store/dossierConsent";
import { useScripts } from "../../store/scripts";
import { useSidebar } from "../../store/sidebar";
import { useUi } from "../../store/ui";
import { pmPromptForProject } from "../projectChat/pmPrompt";
import { agentLive, elapsedShort } from "./runState";

const NO_COLUMNS: never[] = [];
const NO_SECTIONS: never[] = [];

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
  // Dossier (§8f). `null` = this card has none — no group renders at all.
  const [dossier, setDossier] = useState<DossierDto | null>(null);
  /** Which agent-written section the inset card shows; j/k walks it. */
  const [sectionIdx, setSectionIdx] = useState(0);
  /** Ticks the running step's elapsed. Derived from the launch stamp, never
   * stored (DESIGN.md). */
  const [, setTick] = useState(0);
  // Body: rendered markdown by default; Edit toggles the textarea.
  const [editing, setEditing] = useState(false);
  // Select-to-prompt (diff-review pattern): selection in the rendered body
  // grows a FAB → popover → prompt + excerpt go to the PM project chat.
  const [sel, setSel] = useState<{ text: string; left: number; top: number } | null>(null);
  const [askOpen, setAskOpen] = useState(false);
  const [askPrompt, setAskPrompt] = useState("");
  const [askSending, setAskSending] = useState(false);
  const asideRef = useRef<HTMLElement | null>(null);
  const mdRef = useRef<HTMLDivElement | null>(null);
  const projectTasks = useSidebar((s) => s.tasksByProject[projectId]);
  const columns = useColumns((s) => s.byProject[projectId] ?? NO_COLUMNS);
  const agentByTask = useScripts((s) => s.agentByTask);
  const close = () => useUi.getState().setBoardDetailIssueId(null);

  useEffect(() => {
    void useColumns.getState().load(projectId);
  }, [projectId]);

  useEffect(() => {
    let cancelled = false;
    const reload = () => {
      // The dossier reloads on exactly the events the card does — a step
      // settling is what appends both a breadcrumb and (usually) a section,
      // and it lands here as issue:updated / task:status_changed. A failed
      // read is not an error surface: the card simply has no dossier.
      void dossierRead(issueId)
        .then((d) => !cancelled && setDossier(d))
        .catch(() => !cancelled && setDossier(null));
      return issueList(projectId)
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
    };
    setDossier(null);
    setSectionIdx(0);
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

  const sections = dossier?.sections ?? NO_SECTIONS;
  const focusedSection = sections[Math.min(sectionIdx, sections.length - 1)] ?? null;

  // A new section (the step that just settled) takes the focus — walking
  // back to older reasoning is what j/k is for.
  useEffect(() => {
    setSectionIdx(Math.max(0, sections.length - 1));
  }, [sections.length]);

  // Only a running step moves; the rest of the group is static text.
  const stepRunning = dossier?.timeline.some((e) => e.running) ?? false;
  useEffect(() => {
    if (!stepRunning) return;
    const t = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(t);
  }, [stepRunning]);

  /** j/k walk the dossier's sections (§8f's footer hint).
   *
   * Scoped to DOM focus inside the sheet and stopped from propagating,
   * because the BOARD binds j/k globally to walk cards — one key, two
   * handlers is one too many. Focus reaches the group by click or Tab,
   * which is exactly what the footer advertises.
   *
   * **Ownership is decided before the section count.** Bailing early on a
   * one-section dossier let the key the sheet advertised fall through and
   * walk the board behind it — the sheet has the key while it is showing a
   * dossier, whether or not there is anywhere to walk. */
  const walkSections = (e: ReactKeyboardEvent<HTMLElement>) => {
    if (!dossier || e.metaKey || e.ctrlKey || e.altKey) return;
    const key = e.key.toLowerCase();
    if (key !== "j" && key !== "k") return;
    const t = e.target as HTMLElement | null;
    if (
      t &&
      (t.tagName === "INPUT" ||
        t.tagName === "TEXTAREA" ||
        t.tagName === "SELECT" ||
        t.isContentEditable)
    ) {
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (sections.length < 2) return;
    setSectionIdx((i) => {
      const cur = Math.min(i, sections.length - 1);
      return key === "j" ? Math.min(cur + 1, sections.length - 1) : Math.max(cur - 1, 0);
    });
  };

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
    issueUpdate(issueId, { title: title.trim(), body: body || null })
      .then((iss) => {
        setIssue(iss);
        setEditing(false);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setSaving(false));
  };

  const cancelEdit = () => {
    if (issue) {
      setTitle(issue.title);
      setBody(issue.body ?? "");
    }
    setEditing(false);
  };

  /** Selection in the rendered body → FAB anchored below it. */
  const onBodyMouseUp = () => {
    const s = window.getSelection();
    const host = asideRef.current;
    const md = mdRef.current;
    if (!s || s.isCollapsed || !host || !md) {
      setSel(null);
      return;
    }
    const text = s.toString().trim();
    if (!text || !md.contains(s.anchorNode) || !md.contains(s.focusNode)) {
      setSel(null);
      return;
    }
    const rect = s.getRangeAt(0).getBoundingClientRect();
    const box = host.getBoundingClientRect();
    setSel({
      text,
      left: Math.max(4, Math.min(rect.left - box.left, box.width - 308)),
      top: rect.bottom - box.top + 6,
    });
    setAskOpen(false);
  };

  const closeAsk = () => {
    setSel(null);
    setAskOpen(false);
    setAskPrompt("");
  };

  /** Send excerpt + prompt to the PM project chat, then swap the sheet to
   * the chat so the reply (and its approval card) is visible. */
  const ask = async () => {
    const q = askPrompt.trim();
    if (!q || !sel || !issue || askSending) return;
    setAskSending(true);
    setError(null);
    try {
      const provider = (await listProviders()).find((p) =>
        p.capabilities.includes("acp"),
      );
      if (!provider) throw new Error("no ACP-capable provider available");
      const conv = await useConversations.getState().ensureProject(projectId, provider.id);
      await acpStart(conv.id);
      const full =
        `Ticket "${issue.title}" (issueId: ${issue.id}) — selected excerpt:\n` +
        "```\n" +
        sel.text +
        "\n```\n\n" +
        q;
      await useConversations
        .getState()
        .sendPrompt(conv.id, full, await pmPromptForProject(projectId));
      closeAsk();
      const ui = useUi.getState();
      ui.setBoardDetailIssueId(null);
      ui.setProjectChatOpen(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setAskSending(false);
    }
  };

  const setAcceptance = (items: string[]) =>
    void apply(issueUpdate(issueId, { acceptance: items }));

  const linkedTask: TaskDto | undefined = issue?.linkedTaskId
    ? (projectTasks ?? []).find((t) => t.id === issue.linkedTaskId)
    : undefined;

  /** Dispatch the card (E17-03) — spawn or reattach, then hand off to
   * the task view. Errors surface inline.
   *
   * First-dispatch consent (#74, §8e) is asked HERE, ahead of
   * `issue_dispatch`, because this is the moment §8e actually names: the
   * worktree provisions inside that call, and `create_for_task` only mints
   * a dossier on the arm where the task is being created. Consent arriving
   * afterwards would be too late for this card forever — the `Some(task)`
   * arm can never mint it retroactively. */
  const dispatch = async () => {
    if (!issue || dispatching) return;
    setDispatching(true);
    setError(null);
    try {
      // Declining still dispatches; only a withdrawn ask stops here.
      if (!(await ensureDossierConsent(issue.projectId, issue))) return;
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

  // The dot reads the AGENT, not the column (ADR-0037 / the TaskHeader-dot
  // finding): a live agent terminal pulses amber, review is the hollow
  // needs-you ring, everything else is idle. A column never colours it.
  const dotStatus = linkedTask
    ? agentLive(agentByTask[linkedTask.id], linkedTask.status)
      ? "in_progress"
      : linkedTask.status === "review"
        ? "needs-you"
        : null
    : null;
  const columnName = issue
    ? (columns.find((c) => c.id === columnIdForIssue(issue, columns))?.name ??
      issue.lane)
    : "";

  return (
    <aside
      className="card-detail"
      data-issue-id={issueId}
      ref={asideRef}
      onKeyDown={walkSections}
    >
      <header className="card-detail-header">
        <span className="card-detail-lane">
          <span
            className={`status-dot${dotStatus ? ` status-${dotStatus}` : ""}`}
          />
          {columnName}
          {issue?.blocked && (
            <span className="card-detail-blocked-note">
              blocked by {issue.blockers.length}
            </span>
          )}
        </span>
        <div className="card-detail-header-actions">
          {issue &&
            (linkedTask ? (
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
            ))}
          <button className="card-detail-close" onClick={close} aria-label="Close detail">
            ×
          </button>
        </div>
      </header>

      {error && (
        <p className="error card-detail-error" role="alert">
          {error}
        </p>
      )}

      {issue && (
        <div className="card-detail-scroll">
          <div className="card-detail-body">
            <input
              className="card-detail-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && save()}
              aria-label="Issue title"
            />
            {issue.provider && (
              <span className="card-detail-agent">
                {issue.provider}
                {issue.model ? <em>· {issue.model}</em> : null}
              </span>
            )}

            {/* Dossier (§8f). A card without one renders NOTHING here —
                declined consent and pre-E19 cards are not empty states.
                A dossier whose agent skipped the append renders its
                timeline and no inset section, never a nag. */}
            {dossier && (
              <section
                className="card-detail-dossier"
                aria-label="Dossier"
                tabIndex={0}
              >
                <div className="card-detail-dossier-head">
                  <h3>Dossier</h3>
                  <button
                    className="card-detail-link card-detail-dossier-path"
                    title={dossier.hostPath}
                    onClick={() => void open(dossier.hostPath).catch(() => {})}
                  >
                    {dossier.path}
                  </button>
                </div>
                {dossier.timeline.length > 0 && (
                  <ol className="card-detail-timeline">
                    {dossier.timeline.map((entry, i) => (
                      <li key={`${entry.stamp}-${i}`}>
                        <span className="card-detail-timeline-date">
                          {timelineDate(entry)}
                        </span>
                        {entry.text}
                        {entry.running && (
                          <span className="card-detail-timeline-now">
                            {/* No zoned stamp, no elapsed: `running` alone
                                beats a duration computed from a date the
                                backend could not parse. */}
                            {entry.at
                              ? ` · running · ${elapsedShort(entry.at)}`
                              : " · running"}
                          </span>
                        )}
                      </li>
                    ))}
                  </ol>
                )}
                {focusedSection && (
                  <article className="card-detail-dossier-section">
                    <div className="card-detail-dossier-heading">
                      ## {focusedSection.heading}
                    </div>
                    <div className="card-detail-dossier-text">
                      {focusedSection.body}
                    </div>
                    <div className="card-detail-dossier-foot">
                      {`${sections.length} section${
                        sections.length === 1 ? "" : "s"
                      } · j k walk · ⌘K finds them`}
                    </div>
                  </article>
                )}
              </section>
            )}

            {editing ? (
              <>
                <textarea
                  className="card-detail-body-edit"
                  value={body}
                  placeholder="Describe the ticket…"
                  rows={10}
                  autoFocus
                  onChange={(e) => setBody(e.target.value)}
                  aria-label="Issue description"
                />
                <div className="card-detail-save">
                  {dirty && (
                    <span className="card-detail-dirty" aria-live="polite">
                      Unsaved changes
                    </span>
                  )}
                  <button onClick={cancelEdit}>Cancel</button>
                  <button className="primary" disabled={!dirty || saving} onClick={save}>
                    {saving ? "Saving…" : "Save"}
                  </button>
                </div>
              </>
            ) : (
              <div className="card-detail-md-wrap">
                <button
                  className="card-detail-edit-key"
                  onClick={() => setEditing(true)}
                >
                  Edit
                </button>
                <div
                  className="card-detail-md"
                  ref={mdRef}
                  onMouseUp={onBodyMouseUp}
                  onDoubleClick={() => setEditing(true)}
                >
                  {body ? (
                    renderMarkdown(body)
                  ) : (
                    <p className="muted">No description — double-click to add one.</p>
                  )}
                </div>
                {dirty && (
                  <div className="card-detail-save">
                    <span className="card-detail-dirty" aria-live="polite">
                      Unsaved changes
                    </span>
                    <button onClick={cancelEdit}>Cancel</button>
                    <button className="primary" disabled={saving} onClick={save}>
                      {saving ? "Saving…" : "Save"}
                    </button>
                  </div>
                )}
              </div>
            )}

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
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <div className="card-detail-ac-add">
              <input
                value={newAc}
                placeholder="+ Add criterion"
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
                    <button
                      className="card-detail-edge-title card-detail-edge-jump"
                      title={`Open ${b.title}`}
                      onClick={() => useUi.getState().setBoardDetailIssueId(b.id)}
                    >
                      {b.title}
                    </button>
                    {/* The blocker's own column, resolved backend-side the
                        same mirror-first way the board resolves membership
                        — so this row and that card's header can never name
                        two different columns. */}
                    <em>{blockerColumnName(b, columns)}</em>
                    <button
                      className="row-remove"
                      aria-label={`Remove blocker ${b.title}`}
                      onClick={() => void apply(issueUnlink(issueId, b.id))}
                    >
                      ×
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
                    {/* In-app navigation — plain text, never link-out blue. */}
                    <button className="card-detail-edge-jump" onClick={openTask}>
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
          </div>

          <div className="card-detail-footer">
            {confirmDelete ? (
              // Key-first confirm: esc keeps, ↵ deletes (the focused red
              // button activates natively). stopPropagation keeps the
              // board's global ↵/esc handlers out of it.
              <div
                className="card-detail-confirm"
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.stopPropagation();
                    setConfirmDelete(false);
                  } else if (e.key === "Enter") {
                    e.stopPropagation();
                  }
                }}
              >
                <span className="card-detail-confirm-note">Delete this issue?</span>
                <button
                  type="button"
                  className="card-detail-keep"
                  onClick={() => setConfirmDelete(false)}
                >
                  esc keep
                </button>
                <button
                  type="button"
                  className="card-detail-delete-go"
                  autoFocus
                  onClick={() =>
                    void issueDelete(issueId)
                      .then(close)
                      .catch((e) => setError(String(e)))
                  }
                >
                  ↵ delete
                </button>
              </div>
            ) : (
              <button
                className="card-detail-delete"
                onClick={() => setConfirmDelete(true)}
              >
                delete issue
              </button>
            )}
          </div>
        </div>
      )}

      {sel && !askOpen && (
        <button
          className="diff-sel-fab"
          style={{ left: sel.left, top: sel.top }}
          onClick={() => setAskOpen(true)}
        >
          Ask PM
        </button>
      )}
      {sel && askOpen && (
        <div
          className="diff-sel-popover"
          style={{ left: sel.left, top: sel.top }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              closeAsk();
            }
          }}
        >
          <div className="diff-sel-header">
            {sel.text.length > 80 ? `${sel.text.slice(0, 80)}…` : sel.text}
          </div>
          <textarea
            value={askPrompt}
            placeholder="Change this part of the ticket…  (Enter sends, ⇧Enter breaks)"
            rows={3}
            autoFocus
            disabled={askSending}
            onChange={(e) => setAskPrompt(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void ask();
              }
            }}
          />
          <div className="diff-sel-actions">
            <span className="diff-sel-dest">→ Project chat</span>
            <button onClick={closeAsk} disabled={askSending}>
              Cancel
            </button>
            <button
              className="primary"
              disabled={!askPrompt.trim() || askSending}
              onClick={() => void ask()}
            >
              {askSending ? "Sending…" : "Send to PM"}
            </button>
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

/** Timeline date prefix (frame 8f: "aug 6"). The backend hands over an
 * explicitly-zoned `at`; a stamp it could not parse renders as its own
 * date part rather than as a wrong date. */
function timelineDate(entry: DossierTimelineEntryDto): string {
  if (!entry.at) return entry.stamp.split(" ")[0] ?? entry.stamp;
  const d = new Date(entry.at);
  if (Number.isNaN(d.getTime())) return entry.stamp;
  return d
    .toLocaleDateString(undefined, { month: "short", day: "numeric" })
    .toLowerCase();
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
