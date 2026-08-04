// App-level modals (E14-01 modal scope): every dialog renders here, driven
// by the ui store, so the Esc keybinding (close-modal) can reach the
// topmost one via `closeTopModal`.
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import ProjectSettings from "./ProjectSettings";
import SettingsModal from "./SettingsModal";
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
    </>
  );
}
