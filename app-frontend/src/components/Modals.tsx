// App-level modals (E14-01 modal scope): every dialog renders here, driven
// by the ui store, so the Esc keybinding (close-modal) can reach the
// topmost one via `closeTopModal`.
//
// Styling: overlay-card grammar (design_handoff_v2 §5h composer options +
// §7a delete/archive confirm) — classes live in styles/modals.css.
import { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import ProjectSettings from "./ProjectSettings";
import SettingsModal from "./SettingsModal";
import {
  BranchRef,
  CreateTaskOptions,
  RemoteEntryDto,
  SshConnectionDto,
  archiveTask,
  createTask as apiCreateTask,
  createTaskFromComment,
  gitCommitState,
  issueEnterColumn,
  issueList,
  listLineComments,
  listProjectBranches,
  listProviders,
  remoteBrowse,
  sshConnectionList,
  terminalListForTask,
  terminalListPersisted,
  terminalOpenAgent,
  taskShip,
  terminalWrite,
  waitForTerminalReady,
} from "../lib/tauri";
import { maybeOfferWorktreeCleanup } from "../lib/taskPipeline";
import { hint } from "../lib/useCommands";
import { useLineComments } from "../store/line-comments";

import { titleWillSummarize, useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

/** True when the key event originates in a text-entry element — single-key
 * shortcuts (a, ↵) must never fire while the user is typing. */
function typingTarget(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  if (!t) return false;
  return t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable;
}

export function CreateProjectDialog({ onClose }: { onClose: () => void }) {
  const [mode, setMode] = useState<"local" | "remote">("local");
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const createProject = useSidebar((s) => s.createProject);
  const createRemoteProject = useSidebar((s) => s.createRemoteProject);
  const cloneProjectAction = useSidebar((s) => s.cloneProject);
  const cloneRemoteProjectAction = useSidebar((s) => s.cloneRemoteProject);
  const newRemoteProjectAction = useSidebar((s) => s.newRemoteProject);

  // Remote picker (E12-04): connections from the profile store; the browser
  // walks the host over the POOLED session (`remote_browse`, E12-06).
  const [connections, setConnections] = useState<SshConnectionDto[] | null>(null);
  const [connectionId, setConnectionId] = useState("");
  const [cwd, setCwd] = useState("");
  const [entries, setEntries] = useState<RemoteEntryDto[]>([]);
  const [browsing, setBrowsing] = useState(false);

  // Pick is the default; clone (both tabs) swaps the input for a git URL,
  // new (remote only) for a repository name — the browser is irrelevant to
  // either.
  const [kind, setKind] = useState<"pick" | "clone" | "new">("pick");
  const [url, setUrl] = useState("");
  const [repoName, setRepoName] = useState("");

  // `new` has no local meaning — falling back beats a dead submit button.
  useEffect(() => {
    if (mode === "local" && kind === "new") setKind("pick");
  }, [mode, kind]);

  // Lazy: the connection list loads the first time the remote tab opens.
  useEffect(() => {
    if (mode !== "remote" || connections !== null) return;
    sshConnectionList()
      .then((list) => {
        setConnections(list);
        const first = list[0];
        if (first) setConnectionId((id) => id || first.id);
      })
      .catch((e) => setError(String(e)));
  }, [mode, connections]);

  const browse = async (target?: string) => {
    if (!connectionId) return;
    setBrowsing(true);
    setError(null);
    try {
      const list = await remoteBrowse(connectionId, target);
      setEntries(list.filter((en) => en.kind === "dir"));
      if (target) setCwd(target);
      else {
        // No target = the host's login dir; the backend never echoes it
        // back, so recover it from an entry's parent.
        const first = list[0];
        if (first) setCwd(first.path.replace(/\/[^/]*$/, "") || "/");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBrowsing(false);
    }
  };

  // First listing when a connection is chosen (or switched).
  useEffect(() => {
    if (mode !== "remote" || !connectionId) return;
    setCwd("");
    setEntries([]);
    void browse();
  }, [mode, connectionId]);

  const parent = cwd.replace(/\/[^/]*$/, "") || "/";
  const target =
    kind === "clone"
      ? url.trim()
      : kind === "new"
        ? repoName.trim()
        : mode === "local"
          ? path.trim()
          : cwd.trim();

  const submit = async () => {
    if (!target || busy || (mode === "remote" && !connectionId)) return;
    setBusy(true);
    try {
      if (kind === "clone") {
        if (mode === "local") await cloneProjectAction(target);
        else await cloneRemoteProjectAction(connectionId, target);
      } else if (kind === "new") {
        await newRemoteProjectAction(connectionId, target);
      } else if (mode === "local") await createProject(target);
      else await createRemoteProject(connectionId, target);
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="fc-overlay-card fc-composer"
        role="dialog"
        aria-label="Add project"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="fc-src-toggle" role="tablist" aria-label="Project source">
          <button
            type="button"
            role="tab"
            aria-selected={mode === "local"}
            className={mode === "local" ? "on" : ""}
            onClick={() => setMode("local")}
          >
            local
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={mode === "remote"}
            className={mode === "remote" ? "on" : ""}
            onClick={() => setMode("remote")}
          >
            remote · ssh
          </button>
          <button
            type="button"
            className={`fc-clone-pill${kind === "clone" ? " on" : ""}`}
            aria-pressed={kind === "clone"}
            onClick={() => setKind((k) => (k === "clone" ? "pick" : "clone"))}
          >
            clone url
          </button>
          {mode === "remote" && (
            <button
              type="button"
              className={`fc-new-pill${kind === "new" ? " on" : ""}`}
              aria-pressed={kind === "new"}
              onClick={() => setKind((k) => (k === "new" ? "pick" : "new"))}
            >
              new repo
            </button>
          )}
        </div>
        {mode === "remote" && (
          <div className="fc-opt-rows">
            <div className="fc-opt-row">
              <span>host</span>
              <span className="fc-opt-value">
                <span className="fc-opt-ellipsis">
                  {connections === null
                    ? "…"
                    : connections.length === 0
                      ? "no connections — add one in settings"
                      : (connections.find((c) => c.id === connectionId)?.name ?? "pick a host")}
                </span>
                <span aria-hidden>⌄</span>
                <select
                  value={connectionId}
                  aria-label="SSH connection"
                  onChange={(e) => setConnectionId(e.target.value)}
                >
                  {(connections ?? []).map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name} · {c.username}@{c.host}
                    </option>
                  ))}
                </select>
              </span>
            </div>
          </div>
        )}
        {kind === "new" ? (
          <div className="fc-input-row">
            <span className="fc-input-glyph" aria-hidden>
              ›
            </span>
            <input
              className="fc-input mono"
              autoFocus
              value={repoName}
              onChange={(e) => setRepoName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void submit()}
              placeholder="repository name — git init on the host"
              aria-label="New repository name"
            />
          </div>
        ) : kind === "clone" ? (
          <div className="fc-input-row">
            <span className="fc-input-glyph" aria-hidden>
              ›
            </span>
            <input
              className="fc-input mono"
              autoFocus
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void submit()}
              placeholder={
                mode === "local"
                  ? "https://… or git@… — clones into your projects dir"
                  : "https://… or git@… — clones on the host"
              }
              aria-label="Repository URL to clone"
            />
          </div>
        ) : mode === "local" ? (
          <div className="fc-input-row">
            <span className="fc-input-glyph" aria-hidden>
              ›
            </span>
            <input
              className="fc-input mono"
              autoFocus
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void submit()}
              placeholder="/path/to/repo"
              aria-label="Path to a local git repository"
            />
            <button
              type="button"
              className="fc-input-action"
              onClick={async () => {
                try {
                  const selected = await open({ directory: true, multiple: false });
                  if (selected) setPath(selected as string);
                } catch (e) {
                  setError("Dialog failed: " + String(e));
                }
              }}
            >
              browse…
            </button>
          </div>
        ) : (
          <>
            <div className="fc-input-row">
              <span className="fc-input-glyph" aria-hidden>
                ›
              </span>
              <input
                className="fc-input mono"
                value={cwd}
                onChange={(e) => setCwd(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void browse(cwd.trim() || undefined)}
                placeholder="/path/on/host"
                aria-label="Remote repository path"
              />
              <button
                type="button"
                className="fc-input-action"
                disabled={!connectionId || browsing}
                onClick={() => void browse(cwd.trim() || undefined)}
              >
                {browsing ? "listing…" : "list"}
              </button>
            </div>
            <ul className="fc-remote-list" aria-label="Remote directories">
              {cwd && cwd !== "/" && (
                <li>
                  <button type="button" onClick={() => void browse(parent)}>
                    ..
                  </button>
                </li>
              )}
              {entries.map((en) => (
                <li key={en.path}>
                  <button type="button" onClick={() => void browse(en.path)}>
                    {en.name}/
                  </button>
                </li>
              ))}
              {!browsing && entries.length === 0 && (
                <li className="fc-remote-empty">no subdirectories</li>
              )}
            </ul>
          </>
        )}
        {error && (
          <p className="fc-modal-error row" role="alert">
            {error}
          </p>
        )}
        <div className="fc-modal-foot">
          <div className="fc-modal-foot-side">
            <button type="button" onClick={onClose}>
              <span className="fc-key">esc</span> cancel
            </button>
          </div>
          <div className="fc-modal-foot-side">
            <button
              type="button"
              disabled={!target || busy || (mode === "remote" && !connectionId)}
              onClick={submit}
            >
              {busy ? (
                "adding…"
              ) : (
                <>
                  <span className="fc-key">↵</span>{" "}
                  {kind === "clone"
                    ? mode === "local"
                      ? "clone project"
                      : "clone on host"
                    : kind === "new"
                      ? "create on host"
                      : mode === "local"
                        ? "add project"
                        : "add remote project"}
                </>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/** 5h "from" choices: base ref (fresh generated branch), the live checkout,
 * or an existing branch to check out — the pre-redesign picker's exact
 * capabilities, restyled into one options row. */
type FromValue = "base" | `b:${string}`;

export function CreateTaskDialog({
  projectId,
  onClose,
}: {
  projectId: string;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [branches, setBranches] = useState<BranchRef[]>([]);
  const [from, setFrom] = useState<FromValue>("base");
  const [worktree, setWorktree] = useState(true);
  const [touched, setTouched] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Set once create returns for a prompt-length name: the modal stays open
  // until the LLM title lands (task:renamed clears pendingTitle and selects
  // the task) or the store's 10s cap reveals it — either way we close.
  const [waitingId, setWaitingId] = useState<string | null>(null);
  const stillNaming = useSidebar((s) =>
    waitingId ? (s.pendingTitle[waitingId] ?? false) : false,
  );
  useEffect(() => {
    if (waitingId && !stillNaming) onClose();
  }, [waitingId, stillNaming, onClose]);
  const baseRef =
    useSidebar((s) => s.projects.find((p) => p.id === projectId))?.baseRef ??
    "main";
  // hint() re-render on rebinding (key footer).
  useUi((s) => s.bindingsVersion);

  useEffect(() => {
    listProjectBranches(projectId)
      .then(setBranches)
      .catch(() => setBranches([])); // picker degrades to the two fixed rows
  }, [projectId]);

  // Remote twins of a local name (and origin/HEAD) are dropped — they check
  // out the same commit as their local.
  const branchNames = useMemo(() => {
    const locals = new Set(
      branches.filter((b) => !b.is_remote).map((b) => b.name),
    );
    return branches
      .filter((b) => !b.name.endsWith("/HEAD"))
      .filter(
        (b) => !b.is_remote || !locals.has(b.name.replace(/^[^/]+\//, "")),
      )
      .map((b) => b.name);
  }, [branches]);

  const fromLabel = !worktree
    ? "current checkout"
    : from === "base"
      ? `new branch from ${baseRef}`
      : from.slice(2);

  const submit = async () => {
    if (busy) return;
    setBusy(true);
    try {
      // Untouched options = project defaults: send nothing the user didn't
      // pick. Empty name keeps the pre-composer default.
      const opts: CreateTaskOptions = {};
      if (touched) {
        if (!worktree) {
          opts.workspace = "project-root";
        } else {
          opts.workspace = "new-worktree";
          if (from.startsWith("b:")) opts.branch = from.slice(2);
        }
      }
      const task = await apiCreateTask(projectId, name.trim() || "New task", opts);
      // Prompt-length names rename ~1-2s later (background LLM title) —
      // the task stays hidden (and unselected) until the title lands;
      // task:renamed reveals AND selects it.
      const hideUntilTitled = titleWillSummarize(task.name);
      if (hideUntilTitled) useSidebar.getState().markTitlePending(task.id);
      // Hold the modal open (naming banner) until the titled task is
      // actually on the left sheet — the watcher effect closes it.
      if (hideUntilTitled) setWaitingId(task.id);
      // Mirror the sidebar store's createTask bookkeeping (that path
      // hardcodes the name — see notes): append + select immediately; the
      // task:created event refetch keeps it consistent.
      useSidebar.setState((s) => {
        // The task:created event's refetch can land before this promise
        // resolves — only append when the refetch hasn't delivered it yet.
        const existing = s.tasksByProject[projectId] ?? [];
        return {
          tasksByProject: {
            ...s.tasksByProject,
            [projectId]: existing.some((t) => t.id === task.id)
              ? existing
              : [...existing, task],
          },
          ...(hideUntilTitled
            ? {}
            : { selectedProjectId: projectId, selectedTaskId: task.id }),
        };
      });
      if (!hideUntilTitled) onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="fc-overlay-card fc-composer"
        role="dialog"
        aria-label="New task"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          // ↵ create & start from anywhere in the card except an open menu.
          if (e.key === "Enter" && (e.target as HTMLElement).tagName !== "SELECT") {
            e.preventDefault();
            void submit();
          }
        }}
      >
        <div className="fc-input-row">
          <span className="fc-input-glyph" aria-hidden>
            ›
          </span>
          <input
            className="fc-input"
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="describe the task…"
            aria-label="Task name"
          />
        </div>
        <div className="fc-opt-rows">
          <div className={worktree ? "fc-opt-row" : "fc-opt-row fc-opt-dim"}>
            <span>branch</span>
            <span className="fc-opt-value">
              <span className="fc-opt-ellipsis">{fromLabel}</span>
              {worktree && <span aria-hidden>⌄</span>}
              <select
                value={from}
                disabled={!worktree}
                aria-label="Start from"
                onChange={(e) => {
                  setFrom(e.target.value as FromValue);
                  setTouched(true);
                }}
              >
                <option value="base">new branch from {baseRef} · default</option>
                {branchNames.map((n) => (
                  <option key={n} value={`b:${n}`}>
                    {n}
                  </option>
                ))}
              </select>
            </span>
          </div>
        </div>
        {error && (
          <p className="fc-modal-error row" role="alert">
            {error}
          </p>
        )}
        <div className="fc-modal-foot">
          <div className="fc-modal-foot-side">
            <label className="fc-foot-check">
              <span
                className={"fc-check" + (worktree ? " on" : "")}
                aria-hidden
              >
                {worktree ? "✓" : ""}
              </span>
              <input
                type="checkbox"
                checked={worktree}
                aria-label="Isolate in a fresh worktree"
                onChange={(e) => {
                  setWorktree(e.target.checked);
                  setTouched(true);
                }}
              />
              isolated worktree
            </label>
          </div>
          <div className="fc-modal-foot-side">
            <button type="button" disabled={busy} onClick={submit}>
              {busy || waitingId ? (
                <span className="fc-btn-spinner" role="status" aria-label="creating task" />
              ) : (
                <>
                  <span className="fc-key">↵</span> create & start
                </>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/** Middle-ellipsis for long worktree/branch names in the confirm title —
 * the tail (the distinguishing suffix) matters more than the middle. */
function truncate(name: string, max = 36): string {
  if (name.length <= max) return name;
  const head = Math.ceil((max - 1) * 0.6);
  const tail = max - 1 - head;
  return `${name.slice(0, head)}…${name.slice(-tail)}`;
}

/** #134 kill-row label: `terminal <slot>` from the decoded session id's
 * suffix; the full decoded id when the suffix doesn't parse — never the
 * opaque `fartCode-<base64>` tmux name (ADR-0026). */
function tmuxSessionLabel(id: string, projectId: string, taskId: string): string {
  const prefix = `${projectId}:${taskId}:terminal:`;
  const suffix = id.startsWith(prefix) ? id.slice(prefix.length) : "";
  return /^\d+$/.test(suffix) ? `terminal ${suffix}` : id;
}

/** 7a delete/archive confirm — the one place that itemizes. ⌘⌫ deletes
 * (the app's only red action label), `a` archives instead (worktree +
 * branch survive; restore via ⌘K). */
function DeleteTaskConfirm({
  projectId,
  taskId,
  onClose,
}: {
  projectId: string;
  taskId: string;
  onClose: () => void;
}) {
  const task = useSidebar((s) =>
    (s.tasksByProject[projectId] ?? []).find((t) => t.id === taskId),
  );
  const project = useSidebar((s) => s.projects.find((p) => p.id === projectId));
  const deleteTask = useSidebar((s) => s.deleteTask);
  useUi((s) => s.bindingsVersion);
  const [agentRunning, setAgentRunning] = useState(false);
  const [terminalCount, setTerminalCount] = useState(0);
  // #134: live persisted tmux sessions the delete sweep will kill — from
  // the tmux server, so restarts and orphans are covered (the in-memory
  // list above knows nothing about those).
  const [persistedSessions, setPersistedSessions] = useState<string[]>([]);
  const [commentCount, setCommentCount] = useState(0);
  // #135: board cards whose dispatch link points at this task — the FK
  // clears `linked_task_id` silently, so the confirm must say so.
  const [linkedCards, setLinkedCards] = useState<{ id: string; title: string }[]>([]);
  const [branch, setBranch] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Project-root tasks live in the repo checkout — no worktree to remove,
  // no branch of their own to keep.
  const isWorktree =
    !!task?.workspaceId && task.workspaceId !== project?.repositoryWorkspaceId;

  useEffect(() => {
    let cancelled = false;
    terminalListForTask(taskId)
      .then((ts) => {
        if (cancelled) return;
        setAgentRunning(ts.some((t) => t.kind === "agent" && t.running));
        setTerminalCount(ts.length);
      })
      .catch(() => {});
    // Best-effort like every probe here (grill decision 4): an unreachable
    // session host reads as no rows, never an error state.
    terminalListPersisted(taskId)
      .then((ids) => {
        if (!cancelled) setPersistedSessions(ids);
      })
      .catch(() => {});
    listLineComments(taskId)
      .then((cs) => {
        if (!cancelled) setCommentCount(cs.length);
      })
      .catch(() => {});
    issueList(projectId)
      .then((issues) => {
        if (cancelled) return;
        setLinkedCards(
          issues
            .filter((i) => i.linkedTaskId === taskId)
            .map((i) => ({ id: i.id, title: i.title })),
        );
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [projectId, taskId]);

  const workspaceId = isWorktree ? task?.workspaceId : null;
  useEffect(() => {
    if (!workspaceId) return;
    let cancelled = false;
    gitCommitState(workspaceId)
      .then((s) => {
        if (!cancelled) setBranch(s.branch);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  const doDelete = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await deleteTask(projectId, taskId);
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const doArchive = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await archiveTask(taskId);
      // The sidebar event wiring has no task:archived handler — mark the row
      // locally so the card leaves the tree now (see notes).
      useSidebar.setState((s) => ({
        tasksByProject: {
          ...s.tasksByProject,
          [projectId]: (s.tasksByProject[projectId] ?? []).map((t) =>
            t.id === taskId ? { ...t, archivedAt: new Date().toISOString() } : t,
          ),
        },
        selectedTaskId: s.selectedTaskId === taskId ? null : s.selectedTaskId,
      }));
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  // Keys while open: ⌘⌫ delete · a archive (esc = global close-modal).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (typingTarget(e)) return;
      if (e.key === "Backspace" && e.metaKey) {
        e.preventDefault();
        void doDelete();
      } else if (
        e.key.toLowerCase() === "a" &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.altKey
      ) {
        e.preventDefault();
        void doArchive();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  const countParts = [
    commentCount > 0 &&
      `${commentCount} line comment${commentCount === 1 ? "" : "s"}`,
    terminalCount > 0 && `${terminalCount} terminal${terminalCount === 1 ? "" : "s"}`,
  ].filter(Boolean);

  const deleteKey = hint("delete-task") || "⌘⌫";

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="fc-overlay-card fc-confirm"
        role="dialog"
        aria-label="Delete task"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="fc-confirm-body">
          <div className="fc-confirm-title">
            Delete {truncate(branch ?? task?.name ?? `#${taskId.slice(0, 8)}`)}?
          </div>
          <div className="fc-confirm-list">
            {agentRunning && (
              <div className="fc-confirm-live">
                <span className="status-dot status-in_progress" aria-hidden />
                <span>kills the running agent</span>
              </div>
            )}
            {isWorktree && (
              <div>
                {branch
                  ? "removes the worktree · branch kept"
                  : "removes the worktree"}
              </div>
            )}
            {linkedCards.map((c) => (
              <div key={c.id}>unlinks card "{truncate(c.title)}"</div>
            ))}
            {countParts.length > 0 && <div>deletes {countParts.join(" · ")}</div>}
            {persistedSessions.map((id) => (
              <div key={id}>{`kills tmux ${tmuxSessionLabel(id, projectId, taskId)}`}</div>
            ))}
          </div>
          {error && (
            <p className="fc-modal-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="fc-modal-foot">
          <div className="fc-modal-foot-side">
            <button type="button" disabled={busy} onClick={doArchive}>
              <span className="fc-key">a</span> archive instead
            </button>
          </div>
          <div className="fc-modal-foot-side">
            <button
              type="button"
              className="fc-danger"
              disabled={busy}
              onClick={doDelete}
            >
              {busy ? (
                "deleting…"
              ) : (
                <>
                  <span className="fc-key">{deleteKey}</span> delete
                </>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export function ConfirmDelete({
  title,
  name,
  message,
  confirmLabel = "Delete",
  onConfirm,
  onClose,
}: {
  title: string;
  name: string;
  message: string;
  confirmLabel?: string;
  /** Awaited: the modal stays open and shows the failure inline when the
   * destructive action rejects — no silent deletes. */
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const confirm = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  // ↵ confirms (esc = global close-modal). Not the 7a red path — delete-task
  // has its own itemizing confirm; this label stays neutral.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (typingTarget(e)) return;
      if (e.key === "Enter") {
        e.preventDefault();
        void confirm();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="fc-overlay-card fc-confirm"
        role="dialog"
        aria-label={title}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="fc-confirm-body">
          <div className="fc-confirm-title">
            {confirmLabel} <span className="fc-confirm-id">{name}</span>?
          </div>
          <div className="fc-confirm-list">
            <div>{message}</div>
          </div>
          {error && (
            <p className="fc-modal-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="fc-modal-foot">
          <div className="fc-modal-foot-side">
            <button type="button" disabled={busy} onClick={onClose}>
              <span className="fc-key">esc</span> cancel
            </button>
          </div>
          <div className="fc-modal-foot-side">
            <button type="button" disabled={busy} onClick={confirm}>
              {busy ? (
                `${confirmLabel.toLowerCase()}…`
              ) : (
                <>
                  <span className="fc-key">↵</span> {confirmLabel.toLowerCase()}
                </>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/** Quick-task dialog (§14 "Create Task"): pre-filled name + provider pick;
 * submit creates the provisioned task linked to the comment, spawns the
 * agent terminal in it, and pastes the §14 prompt. */
export function QuickTaskDialog({ onClose }: { onClose: () => void }) {
  const target = useUi((s) => s.quickTaskTarget);
  const [name, setName] = useState(target?.prefillName ?? "");
  const [provider, setProvider] = useState("");
  const [providers, setProviders] = useState<{ id: string; name: string }[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const switchToTask = useSidebar((s) => s.switchToTask);

  useEffect(() => {
    listProviders()
      .then((ps) => {
        setProviders(ps);
        const preferred = ps.find((p) => p.id === "claude") ?? ps[0];
        if (preferred) setProvider(preferred.id);
      })
      .catch(() => {});
  }, []);

  if (!target) return null;

  const submit = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const result = await createTaskFromComment({
        projectId: target.projectId,
        name: name.trim(),
        commentId: target.commentId,
        selectedCode: target.selectedCode,
        enclosingFunction: target.enclosingFunction,
      });
      useLineComments.getState().markLinked(target.commentId, result.task.id);
      // Spawn the agent in the new task's worktree and hand it the prompt
      // (bracketed paste so the multi-line template lands as one block).
      if (provider) {
        try {
          const terminalId = await terminalOpenAgent(result.task.id, provider, 24, 80);
          await waitForTerminalReady(terminalId);
          await terminalWrite(terminalId, `\u001b[200~${result.prompt}\u001b[201~\r`);
        } catch {
          // No agent available — the task + link still stand; the user can
          // start the agent from the task view.
        }
      }
      switchToTask(result.task);
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="fc-overlay-card fc-composer"
        role="dialog"
        aria-label="Create task from comment"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.target as HTMLElement).tagName !== "SELECT") {
            e.preventDefault();
            void submit();
          }
        }}
      >
        <div className="fc-input-row">
          <span className="fc-input-glyph" aria-hidden>
            ›
          </span>
          <input
            className="fc-input"
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="Task name"
          />
        </div>
        <div className="fc-opt-rows">
          <div className="fc-opt-row">
            <span>agent</span>
            <span className="fc-opt-value">
              <span className="fc-opt-ellipsis">{provider || "none"}</span>
              <span aria-hidden>⌄</span>
              <select
                value={provider}
                aria-label="Agent provider"
                onChange={(e) => setProvider(e.target.value)}
              >
                {providers.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            </span>
          </div>
          <div className="fc-opt-row">
            <span>from</span>
            <span className="fc-opt-value fc-opt-plain">
              <span className="fc-opt-ellipsis">line comment</span>
            </span>
          </div>
        </div>
        {error && (
          <p className="fc-modal-error row" role="alert">
            {error}
          </p>
        )}
        <div className="fc-modal-foot">
          <div className="fc-modal-foot-side">
            <button type="button" disabled={busy} onClick={onClose}>
              <span className="fc-key">esc</span> cancel
            </button>
          </div>
          <div className="fc-modal-foot-side">
            <button type="button" disabled={!name.trim() || busy} onClick={submit}>
              {busy ? (
                "creating…"
              ) : (
                <>
                  <span className="fc-key">↵</span> create & start
                </>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export default function Modals() {
  const createProjectOpen = useUi((s) => s.createProjectOpen);
  const setCreateProjectOpen = useUi((s) => s.setCreateProjectOpen);
  const settingsOpen = useUi((s) => s.settingsOpen);
  const setSettingsOpen = useUi((s) => s.setSettingsOpen);
  const projectSettingsOpen = useUi((s) => s.projectSettingsOpen);
  const setProjectSettingsOpen = useUi((s) => s.setProjectSettingsOpen);
  const deleteProjectTarget = useUi((s) => s.deleteProjectTarget);
  const setDeleteProjectTarget = useUi((s) => s.setDeleteProjectTarget);
  const deleteTaskTarget = useUi((s) => s.deleteTaskTarget);
  const setDeleteTaskTarget = useUi((s) => s.setDeleteTaskTarget);
  const createTaskTarget = useUi((s) => s.createTaskTarget);
  const setCreateTaskTarget = useUi((s) => s.setCreateTaskTarget);
  const quickTaskTarget = useUi((s) => s.quickTaskTarget);
  const setQuickTaskTarget = useUi((s) => s.setQuickTaskTarget);
  const shipPrompt = useUi((s) => s.shipPrompt);
  const setShipPrompt = useUi((s) => s.setShipPrompt);

  const { projects, selectedProjectId, deleteProject } = useSidebar();
  const taskName = (projectId: string, taskId: string) =>
    (useSidebar.getState().tasksByProject[projectId] ?? []).find((t) => t.id === taskId)
      ?.name ?? taskId;

  return (
    <>
      {createProjectOpen && (
        <CreateProjectDialog onClose={() => setCreateProjectOpen(false)} />
      )}
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
      {projectSettingsOpen && selectedProjectId && (
        <ProjectSettings
          projectId={selectedProjectId}
          projectName={
            projects.find((p) => p.id === selectedProjectId)?.name ?? selectedProjectId
          }
          onClose={() => setProjectSettingsOpen(false)}
        />
      )}
      {deleteProjectTarget && (
        <ConfirmDelete
          title="Delete project"
          name={
            projects.find((p) => p.id === deleteProjectTarget)?.name ?? deleteProjectTarget
          }
          message="Tasks, worktrees, and rows are torn down. The repository on disk is left untouched."
          onClose={() => setDeleteProjectTarget(null)}
          onConfirm={() => deleteProject(deleteProjectTarget)}
        />
      )}
      {createTaskTarget && (
        <CreateTaskDialog
          projectId={createTaskTarget}
          onClose={() => setCreateTaskTarget(null)}
        />
      )}
      {deleteTaskTarget && (
        <DeleteTaskConfirm
          projectId={deleteTaskTarget.projectId}
          taskId={deleteTaskTarget.taskId}
          onClose={() => setDeleteTaskTarget(null)}
        />
      )}
      {quickTaskTarget && <QuickTaskDialog onClose={() => setQuickTaskTarget(null)} />}
      {shipPrompt && (
        <ConfirmDelete
          title="Ship task"
          name={taskName(shipPrompt.issue.projectId, shipPrompt.taskId)}
          confirmLabel="Commit & ship"
          message="The worktree has uncommitted changes. They are committed, the branch is squash-merged into the project root, and the root branch is pushed. Esc keeps the card where it is."
          onClose={() => setShipPrompt(null)}
          onConfirm={async () => {
            await taskShip(shipPrompt.taskId, { autoCommit: true });
            await issueEnterColumn(shipPrompt.issue.id, shipPrompt.column.id);
            maybeOfferWorktreeCleanup(shipPrompt.issue, shipPrompt.column);
          }}
        />
      )}
    </>
  );
}
