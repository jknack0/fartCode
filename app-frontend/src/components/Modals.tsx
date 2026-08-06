// App-level modals (E14-01 modal scope): every dialog renders here, driven
// by the ui store, so the Esc keybinding (close-modal) can reach the
// topmost one via `closeTopModal`.
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import ProjectSettings from "./ProjectSettings";
import SettingsModal from "./SettingsModal";
import {
  createTaskFromComment,
  listProviders,
  terminalOpenAgent,
  terminalWrite,
} from "../lib/tauri";
import { useChanges } from "../store/changes";
import { useLineComments } from "../store/line-comments";

import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

export function CreateProjectDialog({ onClose }: { onClose: () => void }) {
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const createProject = useSidebar((s) => s.createProject);

  const submit = async () => {
    if (!path.trim()) return;
    try {
      await createProject(path.trim());
      onClose();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Add project</h2>
        <label>
          Path to a local git repository
          <div className="path-picker">
            <input
              autoFocus
              value={path}
              onChange={(e) => setPath(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && submit()}
              placeholder="/path/to/repo"
            />
            <button
              type="button"
              onClick={async () => {
                try {
                  const selected = await open({ directory: true, multiple: false });
                  if (selected) setPath(selected as string);
                } catch (e) {
                  setError("Dialog failed: " + String(e));
                }
              }}
            >
              Browse…
            </button>
          </div>
        </label>
        {error && <p className="error">{error}</p>}
        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button className="primary" disabled={!path.trim()} onClick={submit}>
            Add project
          </button>
        </div>
      </div>
    </div>
  );
}

export function ConfirmDelete({
  title,
  name,
  message,
  onConfirm,
  onClose,
}: {
  title: string;
  name: string;
  message: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>{title}</h2>
        <p>
          Delete <strong>{name}</strong>? {message}
        </p>
        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button
            className="danger"
            onClick={() => {
              onConfirm();
              onClose();
            }}
          >
            Delete
          </button>
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
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Create task from comment</h2>
        <label>
          Task name
          <input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && submit()}
          />
        </label>
        <label>
          Agent provider
          <select value={provider} onChange={(e) => setProvider(e.target.value)}>
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        {error && <p className="error">{error}</p>}
        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="primary" disabled={!name.trim() || busy} onClick={submit}>
            {busy ? "Creating…" : "Create task"}
          </button>
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
  const discardTarget = useUi((s) => s.discardTarget);
  const setDiscardTarget = useUi((s) => s.setDiscardTarget);
  const quickTaskTarget = useUi((s) => s.quickTaskTarget);
  const setQuickTaskTarget = useUi((s) => s.setQuickTaskTarget);

  const { projects, tasksByProject, selectedProjectId, deleteProject, deleteTask } =
    useSidebar();

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
          onConfirm={() => {
            deleteProject(deleteProjectTarget).catch(() => {});
          }}
        />
      )}
      {deleteTaskTarget && (
        <ConfirmDelete
          title="Delete task"
          name={
            (tasksByProject[deleteTaskTarget.projectId] ?? []).find(
              (t) => t.id === deleteTaskTarget.taskId,
            )?.name ?? deleteTaskTarget.taskId
          }
          message="Running agents are stopped and the worktree is removed. The source branch stays."
          onClose={() => setDeleteTaskTarget(null)}
          onConfirm={() => {
            deleteTask(deleteTaskTarget.projectId, deleteTaskTarget.taskId).catch((e) =>
              console.error("delete task failed", e),
            );
          }}
        />
      )}
      {discardTarget && (
        <div className="modal-backdrop" onClick={() => setDiscardTarget(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Discard changes</h2>
            <p>
              Discard{" "}
              <strong>
                {discardTarget.paths.length === 1
                  ? discardTarget.paths[0]
                  : `${discardTarget.paths.length} files`}
              </strong>
              ?{" "}
              {discardTarget.hasUntracked
                ? "Untracked files are deleted from disk; tracked files revert to the staged state. This cannot be undone."
                : "The files revert to the staged state. This cannot be undone."}
            </p>
            <div className="modal-actions">
              <button onClick={() => setDiscardTarget(null)}>Cancel</button>
              <button
                className="danger"
                onClick={() => {
                  const target = discardTarget;
                  setDiscardTarget(null);
                  useChanges
                    .getState()
                    .discard(target.workspaceId, target.paths)
                    .catch((e) => console.error("discard failed", e));
                }}
              >
                Discard
              </button>
            </div>
          </div>
        </div>
      )}
      {quickTaskTarget && <QuickTaskDialog onClose={() => setQuickTaskTarget(null)} />}
    </>
  );
}
