// Sidebar state (E1-04): projects → tasks tree, pinned section, and the
// task-switch ordering contract (visible tree order, skipping collapsed).
import { create } from "zustand";
import {
  AdeEvent,
  ProjectDto,
  TaskDto,
  createTask as apiCreateTask,
  createProject as apiCreateProject,
  deleteProject as apiDeleteProject,
  deleteTask as apiDeleteTask,
  listProjects,
  listTasks,
  onAdeEvent,
  setViewState,
  togglePin as apiTogglePin,
} from "../lib/tauri";

interface SidebarState {
  projects: ProjectDto[];
  tasksByProject: Record<string, TaskDto[]>;
  collapsed: Record<string, boolean>;
  selectedProjectId: string | null;
  selectedTaskId: string | null;
  loading: boolean;
  error: string | null;
  load: () => Promise<void>;
  selectProject: (id: string) => void;
  selectTask: (id: string) => void;
  toggleCollapsed: (id: string) => void;
  createTask: (projectId: string) => Promise<void>;
  createProject: (path: string) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  deleteTask: (projectId: string, taskId: string) => Promise<void>;
  togglePin: (id: string) => Promise<void>;
}

const SIDEBAR_VIEW_STATE_KEY = "view-state:app:sidebar";

export const useSidebar = create<SidebarState>((set) => ({
  projects: [],
  tasksByProject: {},
  collapsed: {},
  selectedProjectId: null,
  selectedTaskId: null,
  loading: false,
  error: null,

  load: async () => {
    set({ loading: true, error: null });
    try {
      // Restore persisted view state first (E1-08: layout restores after
      // restart).
      const saved = (await import("../lib/tauri").then((m) =>
        m.getViewState(SIDEBAR_VIEW_STATE_KEY),
      )) as {
        collapsed?: Record<string, boolean>;
        selectedProjectId?: string | null;
        selectedTaskId?: string | null;
      } | null;
      const projects = await listProjects();
      const tasksByProject: Record<string, TaskDto[]> = {};
      for (const p of projects) {
        tasksByProject[p.id] = await listTasks(p.id);
      }
      const validProject = saved?.selectedProjectId
        ? projects.find((p) => p.id === saved.selectedProjectId)
        : undefined;
      // Restore task selection too (E1-08: layout restores after restart) —
      // a saved task id only counts if it still exists under the project.
      const validTask = validProject && saved?.selectedTaskId
        ? (tasksByProject[validProject.id] ?? []).some((t) => t.id === saved.selectedTaskId)
          ? saved.selectedTaskId
          : null
        : null;
      set({
        projects,
        tasksByProject,
        collapsed: saved?.collapsed ?? {},
        selectedProjectId: validProject ? validProject.id : null,
        selectedTaskId: validTask,
        loading: false,
      });
      // Default selection: first project (acceptance: "lands on an empty (or
      // first) project").
      if (!validProject && projects.length > 0) {
        set({ selectedProjectId: projects[0].id });
      }
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  selectProject: (id) => {
    set({ selectedProjectId: id, selectedTaskId: null });
    persistSidebarView();
  },
  selectTask: (id) => {
    set({ selectedTaskId: id });
    persistSidebarView();
  },
  toggleCollapsed: (id) => {
    set((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } }));
    persistSidebarView();
  },

  createTask: async (projectId: string) => {
    const task = await apiCreateTask(projectId, "New task");
    set((s) => {
      const tasks = [...(s.tasksByProject[projectId] ?? []), task];
      return {
        tasksByProject: { ...s.tasksByProject, [projectId]: tasks },
        selectedTaskId: task.id,
      };
    });
  },

  createProject: async (path) => {
    const created = await apiCreateProject(path);
    set((s) => ({
      projects: [...s.projects, created],
      selectedProjectId: created.id,
      selectedTaskId: null,
    }));
  },

  deleteProject: async (id) => {
    await apiDeleteProject(id);
    set((s) => {
      const projects = s.projects.filter((p) => p.id !== id);
      const tasksByProject = { ...s.tasksByProject };
      delete tasksByProject[id];
      const selectedProjectId =
        s.selectedProjectId === id
          ? (projects[0]?.id ?? null)
          : s.selectedProjectId;
      return { projects, tasksByProject, selectedProjectId, selectedTaskId: null };
    });
  },

  deleteTask: async (projectId, taskId) => {
    await apiDeleteTask(projectId, taskId);
    // Local removal for immediate feedback (the backend also re-fires
    // task:deleted → wireSidebarEvents reloads; idempotent).
    set((s) => ({
      tasksByProject: {
        ...s.tasksByProject,
        [projectId]: (s.tasksByProject[projectId] ?? []).filter(
          (t) => t.id !== taskId,
        ),
      },
      selectedTaskId: s.selectedTaskId === taskId ? null : s.selectedTaskId,
    }));
  },

  togglePin: async (id) => {
    const updated = await apiTogglePin(id);
    set((s) => ({
      tasksByProject: {
        ...s.tasksByProject,
        [updated.projectId]: (s.tasksByProject[updated.projectId] ?? []).map(
          (t) => (t.id === id ? updated : t),
        ),
      },
    }));
  },
}));

// ---------------------------------------------------------------------------
// Task-switch ordering contract (E2-10 depends on this): the sidebar's
// visible tree order — pinned tasks first (in their projects' tree order),
// then projects with their tasks, skipping collapsed projects and archived
// tasks. Selecting in this order = the app's task-switch navigation order.
// ---------------------------------------------------------------------------
export function visibleTaskOrder(state: SidebarState): TaskDto[] {
  const order: TaskDto[] = [];
  // Pinned section: pinned tasks of every project, in tree order.
  for (const p of state.projects) {
    for (const t of state.tasksByProject[p.id] ?? []) {
      if (t.isPinned && !t.archivedAt) order.push(t);
    }
  }
  // Unpinned tree: each project's tasks, skipping collapsed projects.
  for (const p of state.projects) {
    if (state.collapsed[p.id]) continue;
    for (const t of state.tasksByProject[p.id] ?? []) {
      if (!t.isPinned && !t.archivedAt) order.push(t);
    }
  }
  return order;
}

/// Persists collapse + selection (fire-and-forget; the backend owns the KV).
function persistSidebarView() {
  const s = useSidebar.getState();
  setViewState(SIDEBAR_VIEW_STATE_KEY, {
    collapsed: s.collapsed,
    selectedProjectId: s.selectedProjectId,
    selectedTaskId: s.selectedTaskId,
  }).catch(() => {});
}

// Wire backend events into the store (project add/delete, task create/delete).
export function wireSidebarEvents(): () => void {
  let unlisten: (() => void) | null = null;
  onAdeEvent((event: AdeEvent) => {
    const s = useSidebar.getState();
    if (event.type === "project:deleted") {
      // The backend already deleted everything; remove locally so the
      // ProjectDeleted event can't re-invoke the API (idempotent but noisy).
      useSidebar.setState((st) => {
        const projects = st.projects.filter((p) => p.id !== event.id);
        const tasksByProject = { ...st.tasksByProject };
        delete tasksByProject[event.id];
        return {
          projects,
          tasksByProject,
          selectedProjectId:
            st.selectedProjectId === event.id ? (projects[0]?.id ?? null) : st.selectedProjectId,
          selectedTaskId: null,
        };
      });
    } else if (event.type === "project:added") {
      s.load().catch(() => {});
    } else if (event.type === "task:created" || event.type === "task:deleted") {
      // Refetch the affected project's tasks.
      const projectId = event.type === "task:created" ? event.projectId : null;
      if (projectId) {
        listTasks(projectId)
          .then((tasks) =>
            useSidebar.setState((st) => ({
              tasksByProject: { ...st.tasksByProject, [projectId]: tasks },
            })),
          )
          .catch(() => {});
      } else {
        s.load().catch(() => {});
      }
    }
  }).then((fn) => {
    unlisten = fn;
  }).catch((e) => {
    console.error("ade:event listen failed", e);
  });
  return () => unlisten?.();
}
