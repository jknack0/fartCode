// Sidebar (E1-04): projects → tasks tree with a pinned section and pin
// toggling. The visible tree order is the task-switch navigation order
// (E2-10 contract). Shortcuts are commands (E14-01): ⌘⇧N add project, ⌘N
// add task, ⌘Backspace delete task, ⌘B toggles this panel.
// Project rows also carry a pull action (git pull --ff-only at the project
// root) so merged worktree branches can be brought down without leaving
// the app. Errors surface inline under the row — the repo has no toast
// system.
import { useState } from "react";
import { useUi } from "../store/ui";
import { useSidebar } from "../store/sidebar";
import { hint } from "../lib/useCommands";
import { projectGitPull } from "../lib/tauri";
import { IconChevron, IconClose, IconGear, IconPin, IconPlus, IconPull } from "./icons";
import { useGutterResize } from "../lib/useGutterResize";

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
    createTask,
  } = useSidebar();

  const sidebarVisible = useUi((s) => s.sidebarVisible);
  const toggleSidebarVisible = useUi((s) => s.toggleSidebarVisible);
  // Re-renders hint text when keybindings change (E14-01 hint rendering).
  useUi((s) => s.bindingsVersion);
  const setProjectSettingsOpen = useUi((s) => s.setProjectSettingsOpen);
  const setCreateProjectOpen = useUi((s) => s.setCreateProjectOpen);
  const setPaletteOpen = useUi((s) => s.setPaletteOpen);
  const setDeleteProjectTarget = useUi((s) => s.setDeleteProjectTarget);
  const setDeleteTaskTarget = useUi((s) => s.setDeleteTaskTarget);

  const [pullingId, setPullingId] = useState<string | null>(null);
  const [pullError, setPullError] = useState<{
    projectId: string;
    message: string;
  } | null>(null);
  const resize = useGutterResize(264, 160, 480, 1);

  const pullProject = async (projectId: string) => {
    setPullError(null);
    setPullingId(projectId);
    try {
      await projectGitPull(projectId);
    } catch (e) {
      setPullError({ projectId, message: String(e) });
    } finally {
      setPullingId(null);
    }
  };

  if (!sidebarVisible) {
    // Gmail-style collapsed rail: icon-only, projects as letter squares.
    // Expand sits at the top, mirroring the collapse control in the full
    // sidebar's header (⌘B toggles too).
    return (
      <aside className="sidebar-rail">
        <button
          title={`Show sidebar (${hint("toggle-sidebar") || "⌘B"})`}
          onClick={() => toggleSidebarVisible()}
        >
          <IconChevron />
        </button>
        <button title="Command palette" onClick={() => setPaletteOpen(true)}>
          <span className="rail-chip">{hint("open-command-palette") || "⌘K"}</span>
        </button>
        <button
          title="Project settings"
          onClick={() => setProjectSettingsOpen(true)}
          disabled={!selectedProjectId}
        >
          <IconGear />
        </button>
        <div className="rail-projects">
          {projects.map((p) => (
            <button
              key={p.id}
              className={`rail-project${selectedProjectId === p.id ? " active" : ""}`}
              title={p.name}
              onClick={() => selectProject(p.id)}
            >
              {p.name[0] ?? "?"}
            </button>
          ))}
        </div>
        <button
          title={`Add project (${hint("new-project")})`}
          onClick={() => setCreateProjectOpen(true)}
        >
          <IconPlus />
        </button>
      </aside>
    );
  }

  const pinnedCount = projects.reduce(
    (n, p) =>
      n + (tasksByProject[p.id] ?? []).filter((t) => t.isPinned && !t.archivedAt).length,
    0,
  );

  return (
    <aside className="sidebar" style={{ width: resize.width }}>
      <div className="gutter-handle sidebar-handle" {...resize.bind} />
      <div className="sidebar-header">
        <span className="brand">fartCode</span>
        <div className="header-actions">
          <button
            className="palette-chip"
            title="Command palette"
            onClick={() => setPaletteOpen(true)}
          >
            {hint("open-command-palette") || "⌘K"}
          </button>
          <button
            title="Project settings"
            onClick={() => setProjectSettingsOpen(true)}
            disabled={!selectedProjectId}
          >
            <IconGear />
          </button>
          <button
            title={`Add project (${hint("new-project")})`}
            onClick={() => setCreateProjectOpen(true)}
          >
            <IconPlus />
          </button>
          <button
            title={`Hide sidebar (${hint("toggle-sidebar") || "⌘B"})`}
            onClick={() => toggleSidebarVisible()}
          >
            <span style={{ display: "inline-flex", transform: "rotate(180deg)" }}>
              <IconChevron />
            </span>
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
                    projectId={p.id}
                    selected={selectedTaskId === t.id}
                    onSelect={() => selectTask(t.id)}
                    onPin={() => togglePin(t.id)}
                    onDelete={(projectId, taskId) =>
                      setDeleteTaskTarget({ projectId, taskId })
                    }
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
                className={`project-row${selectedProjectId === p.id && !selectedTaskId ? " selected" : ""}`}
                role="button"
                tabIndex={0}
                onClick={() => selectProject(p.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    selectProject(p.id);
                  }
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setDeleteProjectTarget(p.id);
                }}
                title="Right-click to delete"
              >
                <button
                  className={`chevron${collapsed[p.id] ? "" : " open"}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleCollapsed(p.id);
                  }}
                >
                  <IconChevron size={10} />
                </button>
                <span className="project-name">{p.name}</span>
                <button
                  className="add-task-btn"
                  title="Pull project root (git pull --ff-only)"
                  disabled={pullingId === p.id}
                  onClick={(e) => {
                    e.stopPropagation();
                    void pullProject(p.id);
                  }}
                >
                  <IconPull size={10} />
                </button>
                <button
                  className="add-task-btn"
                  title={`New task (${hint("add-task")})`}
                  onClick={(e) => {
                    e.stopPropagation();
                    createTask(p.id);
                  }}
                >
                  <IconPlus size={10} />
                </button>
              </div>
              {pullError?.projectId === p.id && (
                <p className="project-pull-error" role="alert">
                  {pullError.message}
                </p>
              )}
              {!collapsed[p.id] && (
                <ul>
                  {(tasksByProject[p.id] ?? [])
                    .filter((t) => !t.archivedAt)
                    .map((t) => (
                      <TaskRow
                        key={t.id}
                        task={t}
                        projectId={p.id}
                        selected={selectedTaskId === t.id}
                        onSelect={() => selectTask(t.id)}
                        onPin={() => togglePin(t.id)}
                        onDelete={(projectId, taskId) =>
                          setDeleteTaskTarget({ projectId, taskId })
                        }
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
              No projects yet. Press {hint("new-project") || "⌘⇧N"} to add one.
            </li>
          )}
        </ul>
      </section>
    </aside>
  );
}

function TaskRow({
  task,
  projectId,
  selected,
  onSelect,
  onPin,
  onDelete,
}: {
  task: { id: string; name: string; status: string; isPinned: boolean };
  projectId: string;
  selected: boolean;
  onSelect: () => void;
  onPin: () => void;
  onDelete: (projectId: string, taskId: string) => void;
}) {
  return (
    <li
      className={`task-row${selected ? " selected" : ""}`}
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onPin();
      }}
      title={`Click to open · right-click to pin/unpin · ${hint("delete-task") || "⌘⌫"} to delete`}
    >
      <span className={`status-dot status-${task.status}`} />
      <span className="task-name">{task.name}</span>
      {task.isPinned && (
        <span className="pin">
          <IconPin size={10} />
        </span>
      )}
      <button
        className="delete-task-btn"
        title={`Delete task (${hint("delete-task") || "⌘⌫"})`}
          onClick={(e) => {
            e.stopPropagation();
            onDelete(projectId, task.id);
          }}
        >
          <IconClose size={10} />
        </button>
    </li>
  );
}
