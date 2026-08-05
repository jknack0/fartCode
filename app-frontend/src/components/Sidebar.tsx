// Sidebar (E1-04): projects → tasks tree with a pinned section and pin
// toggling. The visible tree order is the task-switch navigation order
// (E2-10 contract). Shortcuts are commands (E14-01): ⌘⇧N add project, ⌘N
// add task, ⌘Backspace delete task, ⌘B toggles this panel.
import { useUi } from "../store/ui";
import { useSidebar } from "../store/sidebar";
import { hint } from "../lib/useCommands";
import { IconChevron, IconClose, IconGear, IconPin, IconPlus } from "./icons";

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
  // Re-renders hint text when keybindings change (E14-01 hint rendering).
  useUi((s) => s.bindingsVersion);
  const setProjectSettingsOpen = useUi((s) => s.setProjectSettingsOpen);
  const setCreateProjectOpen = useUi((s) => s.setCreateProjectOpen);
  const setPaletteOpen = useUi((s) => s.setPaletteOpen);
  const setDeleteProjectTarget = useUi((s) => s.setDeleteProjectTarget);
  const setDeleteTaskTarget = useUi((s) => s.setDeleteTaskTarget);

  if (!sidebarVisible) return null;

  const pinnedCount = projects.reduce(
    (n, p) =>
      n + (tasksByProject[p.id] ?? []).filter((t) => t.isPinned && !t.archivedAt).length,
    0,
  );

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span className="brand">
          a<span className="brand-mark" aria-hidden="true" />de
        </span>
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
                  title={`New task (${hint("add-task")})`}
                  onClick={(e) => {
                    e.stopPropagation();
                    createTask(p.id);
                  }}
                >
                  <IconPlus size={10} />
                </button>
              </div>
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
