// Sidebar state (E1-04): projects → tasks tree, pinned section, and the
// task-switch ordering contract (visible tree order, skipping collapsed).
import { create } from "zustand";
import { importGithubIssues } from "../lib/github-sync";
import { useUi } from "./ui";
import {
  CreateTaskOptions,
  FartcodeEvent,
  ProjectDto,
  TaskDto,
  createTask as apiCreateTask,
  createProject as apiCreateProject,
  createRemoteProject as apiCreateRemoteProject,
  cloneProject as apiCloneProject,
  cloneRemoteProject as apiCloneRemoteProject,
  newRemoteProject as apiNewRemoteProject,
  deleteProject as apiDeleteProject,
  deleteTask as apiDeleteTask,
  getProjectSettings,
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
  /** Task ids whose pasted-prompt name awaits its LLM title (skeleton renders meanwhile). */
  pendingTitle: Record<string, boolean>;
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
  markTitlePending: (taskId: string) => void;
  createProject: (path: string) => Promise<void>;
  /** Remote project picker (E12-04): an existing repo on an SSH host. */
  createRemoteProject: (connectionId: string, remotePath: string) => Promise<void>;
  /** FLOWS.md F2: clone a URL into the local projects directory. */
  cloneProject: (url: string) => Promise<void>;
  /** E12-04 Clone: clone a URL on the SSH host, then add it. */
  cloneRemoteProject: (connectionId: string, url: string) => Promise<void>;
  /** E12-04 New: init a fresh repo on the SSH host, then add it. */
  newRemoteProject: (connectionId: string, name: string) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  deleteTask: (projectId: string, taskId: string) => Promise<void>;
  togglePin: (id: string) => Promise<void>;
}

/** Mirrors SUMMARIZE_NAME_THRESHOLD in fartcode-app/src/commands/tasks.rs:
 * names past 60 chars (or multiline) get a background LLM title. */
export const titleWillSummarize = (name: string) => name.length > 60 || name.includes("\n");

const SIDEBAR_VIEW_STATE_KEY = "view-state:app:sidebar";

// ponytail: 30s cooldown per project, in-memory only — restart just re-pulls.
const lastPullAt: Record<string, number> = {};

const PULL_COOLDOWN_MS = 30_000;

function autoPullProject(id: string) {
  const now = Date.now();
  if (now - (lastPullAt[id] ?? 0) < PULL_COOLDOWN_MS) return;
  lastPullAt[id] = now;
  projectGitPull(id).catch((e) =>
    useUi.getState().setProjectNotice(`project pull failed: ${String(e)}`),
  );
}

/** GitHub import on project add, gated by the `autoImport` project setting
 * (#120): `false` opts out; `true`/unset keeps the historical auto-import.
 * Reports through the board's quiet status line. */
async function autoImportGithubIssues(projectId: string): Promise<void> {
  try {
    const settings = await getProjectSettings(projectId);
    if (settings.autoImport === false) return;
    await importGithubIssues(projectId);
  } catch (e) {
    useUi.getState().setProjectNotice(`github import failed: ${String(e)}`);
  }
}

export const useSidebar = create<SidebarState>((set, get) => ({
  projects: [],
  tasksByProject: {},
  pendingTitle: {},
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
      // The task:created event's refetch can land before this promise
      // resolves — only append when the refetch hasn't delivered it yet.
      const existing = s.tasksByProject[projectId] ?? [];
      const tasks = existing.some((t) => t.id === task.id)
        ? existing
        : [...existing, task];
      return {
        tasksByProject: { ...s.tasksByProject, [projectId]: tasks },
        selectedTaskId: task.id,
      };
    });
  },

  /** Skeleton window for a pasted-prompt title: set at create, cleared by
   * task:renamed. The summary is best-effort (claude and the ollama
   * fallback can both fail) and failure emits nothing — the timeout
   * uncovers the full name so the skeleton can't shimmer forever. */
  markTitlePending: (taskId: string) => {
    set((s) => ({ pendingTitle: { ...s.pendingTitle, [taskId]: true } }));
    setTimeout(() => {
      set((s) => {
        if (!s.pendingTitle[taskId]) return s;
        const { [taskId]: _dropped, ...pendingTitle } = s.pendingTitle;
        return { ...s, pendingTitle };
      });
    }, 10_000);
  },

  createProject: async (path) => {
    const created = await apiCreateProject(path);
    set((s) => ({
      projects: [...s.projects, created],
      selectedProjectId: created.id,
      selectedTaskId: null,
    }));
    // Import the checkout's open GitHub issues ONCE, here — the board no
    // longer syncs on mount (lib/github-sync.ts).
    void autoImportGithubIssues(created.id);
  },

  createRemoteProject: async (connectionId, remotePath) => {
    const created = await apiCreateRemoteProject(connectionId, remotePath);
    set((s) => ({
      projects: [...s.projects, created],
      selectedProjectId: created.id,
      selectedTaskId: null,
    }));
    // Import the checkout's open GitHub issues ONCE, here — the board no
    // longer syncs on mount (lib/github-sync.ts).
    void autoImportGithubIssues(created.id);
  },

  cloneProject: async (url) => {
    const created = await apiCloneProject(url);
    set((s) => ({
      projects: [...s.projects, created],
      selectedProjectId: created.id,
      selectedTaskId: null,
    }));
    // Import the checkout's open GitHub issues ONCE, here — the board no
    // longer syncs on mount (lib/github-sync.ts).
    void autoImportGithubIssues(created.id);
  },

  cloneRemoteProject: async (connectionId, url) => {
    const created = await apiCloneRemoteProject(connectionId, url);
    set((s) => ({
      projects: [...s.projects, created],
      selectedProjectId: created.id,
      selectedTaskId: null,
    }));
    // Import the checkout's open GitHub issues ONCE, here — the board no
    // longer syncs on mount (lib/github-sync.ts).
    void autoImportGithubIssues(created.id);
  },

  newRemoteProject: async (connectionId, name) => {
    const created = await apiNewRemoteProject(connectionId, name);
    set((s) => ({
      projects: [...s.projects, created],
      selectedProjectId: created.id,
      selectedTaskId: null,
    }));
    // Import the checkout's open GitHub issues ONCE, here — the board no
    // longer syncs on mount (lib/github-sync.ts).
    void autoImportGithubIssues(created.id);
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
    } else if (event.type === "task:renamed") {
      // LLM title landed (create_task's background summary): swap the name
      // in place — no refetch, the row is already on screen.
      useSidebar.setState((st) => {
        // Title landed — reveal the hidden task, and select it when it was
        // waiting (creation deferred the select to this moment).
        const wasPending = Boolean(st.pendingTitle[event.taskId]);
        const { [event.taskId]: _dropped, ...pendingTitle } = st.pendingTitle;
        return {
          pendingTitle,
          ...(wasPending
            ? { selectedProjectId: event.projectId, selectedTaskId: event.taskId }
            : {}),
          tasksByProject: {
            ...st.tasksByProject,
            [event.projectId]: (st.tasksByProject[event.projectId] ?? []).map((t) =>
              t.id === event.taskId ? { ...t, name: event.name } : t,
            ),
          },
        };
      });
    } else if (
      event.type === "task:created" ||
      event.type === "task:deleted" ||
      // 7a: archive/restore flip archivedAt — refetch so the board/flyout
      // drop or resurface the task (restore may come from the palette).
      event.type === "task:archived" ||
      event.type === "task:restored"
    ) {
      // Pasted-prompt names get a background LLM title — mark BEFORE the
      // refetch (or the modal's own append) paints the row, so the
      // skeleton shows from first paint. The modal path also marks after
      // its invoke resolves, but this event usually wins that race.
      if (event.type === "task:created" && titleWillSummarize(event.name)) {
        useSidebar.getState().markTitlePending(event.id);
      }
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
