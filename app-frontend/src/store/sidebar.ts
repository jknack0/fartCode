// Sidebar state (E1-04): projects → tasks tree, pinned section, and the
// task-switch ordering contract (visible tree order, skipping collapsed).
import { create } from "zustand";
import {
  CreateTaskOptions,
  FartcodeEvent,
  ProjectDto,
  TaskDto,
  createTask as apiCreateTask,
  createProject as apiCreateProject,
  createRemoteProject as apiCreateRemoteProject,
  deleteProject as apiDeleteProject,
  deleteTask as apiDeleteTask,
  listProjects,
  listTasks,
  onFartcodeEvent,
  projectGitPull,
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
  /** Task-switch navigation (E2-10): select a task and its project. */
  switchToTask: (task: TaskDto) => void;
  toggleCollapsed: (id: string) => void;
  createTask: (projectId: string, opts?: CreateTaskOptions) => Promise<void>;
  createProject: (path: string) => Promise<void>;
  /** Remote project picker (E12-04): an existing repo on an SSH host. */
  createRemoteProject: (connectionId: string, remotePath: string) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  deleteTask: (projectId: string, taskId: string) => Promise<void>;
  togglePin: (id: string) => Promise<void>;
}

const SIDEBAR_VIEW_STATE_KEY = "view-state:app:sidebar";

// ponytail: 30s cooldown per project, in-memory only — restart just re-pulls.
const lastPullAt: Record<string, number> = {};

const PULL_COOLDOWN_MS = 30_000;

function autoPullProject(id: string) {
  const now = Date.now();
  if (now - (lastPullAt[id] ?? 0) < PULL_COOLDOWN_MS) return;
  lastPullAt[id] = now;
  projectGitPull(id).catch((e) => console.warn("project pull failed:", e));
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
    const changed = id !== get().selectedProjectId;
    set({ selectedProjectId: id, selectedTaskId: null });
    persistSidebarView();
    // Entering the project view implies wanting the root current (merged
    // worktree branches) — pull --ff-only in the background.
    if (changed) autoPullProject(id);
  },
  selectTask: (id) => {
    // A task is always selected under its own project — keeps TaskView's
    // projectId in sync when the click crosses projects.
    const { projects, tasksByProject } = get();
    const projectId = projects.find((p) =>
      (tasksByProject[p.id] ?? []).some((t) => t.id === id),
    )?.id;
    set({ selectedTaskId: id, ...(projectId ? { selectedProjectId: projectId } : {}) });
    persistSidebarView();
  },
  switchToTask: (task) => {
    set({ selectedProjectId: task.projectId, selectedTaskId: task.id });
    persistSidebarView();
  },
  toggleCollapsed: (id) => {
    set((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } }));
    persistSidebarView();
  },

  createTask: async (projectId: string, opts?: CreateTaskOptions) => {
    const task = await apiCreateTask(projectId, "New task", opts);
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

  createRemoteProject: async (connectionId, remotePath) => {
    const created = await apiCreateRemoteProject(connectionId, remotePath);
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
  onFartcodeEvent((event: FartcodeEvent) => {
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
    } else if (
      event.type === "task:created" ||
      event.type === "task:deleted" ||
      // 7a: archive/restore flip archivedAt — refetch so the board/flyout
      // drop or resurface the task (restore may come from the palette).
      event.type === "task:archived" ||
      event.type === "task:restored"
    ) {
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
    console.error("fartcode:event listen failed", e);
  });
  return () => unlisten?.();
}
