// Sidebar state (E1-04): projects → tasks tree, pinned section, and the
// task-switch ordering contract (visible tree order, skipping collapsed).
import { create } from "zustand";
import {
  AdeEvent,
  ProjectDto,
  TaskDto,
  createProject as apiCreateProject,
  deleteProject as apiDeleteProject,
  listProjects,
  listTasks,
  onAdeEvent,
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
  createProject: (path: string) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  togglePin: (id: string) => Promise<void>;
}

export const useSidebar = create<SidebarState>((set, get) => ({
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
      const projects = await listProjects();
      const tasksByProject: Record<string, TaskDto[]> = {};
      for (const p of projects) {
        tasksByProject[p.id] = await listTasks(p.id);
      }
      set({ projects, tasksByProject, loading: false });
      // Default selection: first project (acceptance: "lands on an empty (or
      // first) project").
      if (!get().selectedProjectId && projects.length > 0) {
        set({ selectedProjectId: projects[0].id });
      }
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  selectProject: (id) =>
    set({ selectedProjectId: id, selectedTaskId: null }),
  selectTask: (id) => set({ selectedTaskId: id }),
  toggleCollapsed: (id) =>
    set((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } })),

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
