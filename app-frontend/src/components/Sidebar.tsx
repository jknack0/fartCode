// Sidebar (E1-04): projects → tasks tree with a pinned section, project
// create (⌘⇧N) / delete with confirmation, and pin toggling. The visible
// tree order is the task-switch navigation order (E2-10 contract).
import { useEffect, useState } from "react";
import ProjectSettings from "./ProjectSettings";
import { open } from "@tauri-apps/plugin-dialog";
import { useSidebar } from "../store/sidebar";
import { useUi } from "../store/ui";

function CreateProjectDialog({ onClose }: { onClose: () => void }) {
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const createProject = useSidebar((s) => s.createProject);

  const submit = async () => {
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
        <p className="muted">Path to a local git repository</p>
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
              const selected = await open({ directory: true, multiple: false });
              if (selected) setPath(selected as string);
            }}
          >
            Browse…
          </button>
        </div>
        {error && <p className="error">{error}</p>}
        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button className="primary" onClick={submit} disabled={!path.trim()}>
            Add
          </button>
        </div>
      </div>
    </div>
  );
}

function ConfirmDelete({
  name,
  onConfirm,
  onClose,
}: {
  name: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Delete project</h2>
        <p>
          Delete <strong>{name}</strong>? Tasks, worktrees, and rows are torn
          down. The repository on disk is left untouched.
        </p>
        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button className="danger" onClick={onConfirm}>
            Delete
          </button>
        </div>
      </div>
    </div>
  );
}

export default function Sidebar() {
  const {
    projects,
    tasksByProject,
    collapsed,
    selectedProjectId,
    selectedTaskId,
    selectProject,
    selectTask,
    toggleCollapsed,
    togglePin,
    deleteProject,
  } = useSidebar();

  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const createOpen = useUi((s) => s.createProjectOpen);
  const setCreateOpen = useUi((s) => s.setCreateProjectOpen);

  // ⌘⇧N / Ctrl+Shift+N → create project.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setCreateOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const pinnedCount = projects.reduce(
    (n, p) =>
      n + (tasksByProject[p.id] ?? []).filter((t) => t.isPinned && !t.archivedAt).length,
    0,
  );

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span className="brand">ade</span>
        <div className="header-actions">
          <button
            title="Project settings"
            onClick={() => setSettingsOpen(true)}
            disabled={!selectedProjectId}
          >
            ⚙
          </button>
          <button title="Add project (⌘⇧N)" onClick={() => setCreateOpen(true)}>
            +
          </button>
        </div>
      </div>

      {pinnedCount > 0 && (
        <section className="sidebar-section">
          <h3>Pinned</h3>
          <ul>
            {projects.map((p) =>
              (tasksByProject[p.id] ?? [])
                .filter((t) => t.isPinned && !t.archivedAt)
                .map((t) => (
                  <TaskRow
                    key={t.id}
                    task={t}
                    selected={selectedTaskId === t.id}
                    onSelect={() => selectTask(t.id)}
                    onPin={() => togglePin(t.id)}
                  />
                )),
            )}
          </ul>
        </section>
      )}

      <section className="sidebar-section">
        <h3>Projects</h3>
        <ul>
          {projects.map((p) => (
            <li key={p.id}>
              <div
                className={`project-row${selectedProjectId === p.id ? " selected" : ""}`}
                onClick={() => selectProject(p.id)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setDeleteTarget(p.id);
                }}
                title="Right-click to delete"
              >
                <button
                  className="chevron"
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleCollapsed(p.id);
                  }}
                >
                  {collapsed[p.id] ? "▸" : "▾"}
                </button>
                <span className="project-name">{p.name}</span>
              </div>
              {!collapsed[p.id] && (
                <ul>
                  {(tasksByProject[p.id] ?? [])
                    .filter((t) => !t.archivedAt)
                    .map((t) => (
                      <TaskRow
                        key={t.id}
                        task={t}
                        selected={selectedTaskId === t.id}
                        onSelect={() => selectTask(t.id)}
                        onPin={() => togglePin(t.id)}
                      />
                    ))}
                  {(tasksByProject[p.id] ?? []).filter((t) => !t.archivedAt).length === 0 && (
                    <li className="empty">no tasks</li>
                  )}
                </ul>
              )}
            </li>
          ))}
          {projects.length === 0 && (
            <li className="empty">
              No projects yet. Press ⌘⇧N to add one.
            </li>
          )}
        </ul>
      </section>

      {settingsOpen && selectedProjectId && (
        <ProjectSettings
          projectId={selectedProjectId}
          projectName={
            projects.find((p) => p.id === selectedProjectId)?.name ?? selectedProjectId
          }
          onClose={() => setSettingsOpen(false)}
        />
      )}
      {createOpen && <CreateProjectDialog onClose={() => setCreateOpen(false)} />}
      {deleteTarget &&
        (() => {
          const target = projects.find((p) => p.id === deleteTarget);
          return (
            <ConfirmDelete
              name={target?.name ?? deleteTarget}
              onClose={() => setDeleteTarget(null)}
              onConfirm={() => {
                deleteProject(deleteTarget).catch(() => {});
                setDeleteTarget(null);
              }}
            />
          );
        })()}
    </aside>
  );
}

function TaskRow({
  task,
  selected,
  onSelect,
  onPin,
}: {
  task: { id: string; name: string; status: string; isPinned: boolean };
  selected: boolean;
  onSelect: () => void;
  onPin: () => void;
}) {
  return (
    <li
      className={`task-row${selected ? " selected" : ""}`}
      onClick={onSelect}
      onContextMenu={(e) => {
        e.preventDefault();
        onPin();
      }}
      title="Click to open · right-click to pin/unpin"
    >
      <span className={`status-dot status-${task.status}`} />
      <span className="task-name">{task.name}</span>
      {task.isPinned && <span className="pin">📌</span>}
    </li>
  );
}
