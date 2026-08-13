// Global UI flags + modal registry (E1-09 palette wiring, E14-01 modal
// scope). The keybinding dispatch reads this store to decide whether a
// modal scope is active and what Esc closes first.
import { create } from "zustand";
import type { ScriptType } from "./scripts";

export interface DeleteTaskTarget {
  projectId: string;
  taskId: string;
}

/** Pending "Create Task from comment" dialog (E4-10, §14): everything the
 * quick-task modal needs to call create_task_from_comment + hand the prompt
 * to the spawned agent terminal. */
export interface QuickTaskTarget {
  projectId: string;
  commentId: string;
  selectedCode: string;
  enclosingFunction: string | null;
  /** Pre-filled task name (first line of the comment, truncated). */
  prefillName: string;
}

interface UiState {
  paletteOpen: boolean;
  createProjectOpen: boolean;
  resourceOpen: boolean;
  /** Changes sidebar (E4-03): right-side panel in the task view. */
  changesOpen: boolean;
  /** PM chat panel (E17-04): right-side panel in the project view. */
  projectChatOpen: boolean;
  /** Task chat panel: right-side panel in the task view, same sheet as
   * Changes (mirrors the project scope's chat panel). */
  taskChatOpen: boolean;
  /** File tree (E5-01, sheet mode): right-side panel in the task view,
   * same sheet as Changes — double-click opens the file in the main area. */
  fileTreeOpen: boolean;
  /** Open card-detail issue id (E17-02): takes precedence over the chat
   * panel in the project view's right region; null shows the chat. */
  boardDetailIssueId: string | null;
  /** ⌘J drawer (7b): bottom sheet on the task view with the lifecycle
   * script terminals. Not a modal — Esc belongs to the terminal. */
  drawerOpen: boolean;
  /** Which script's tab the drawer shows. */
  drawerScript: ScriptType;
  /** App settings (E14-01 shortcut customization lives here). */
  settingsOpen: boolean;
  /** Project settings modal (opened from the sidebar gear). */
  projectSettingsOpen: boolean;
  sidebarVisible: boolean;
  deleteTaskTarget: DeleteTaskTarget | null;
  /** Project id the create-task dialog targets (null = closed). */
  createTaskTarget: string | null;
  deleteProjectTarget: string | null;
  quickTaskTarget: QuickTaskTarget | null;
  onboardingOpen: boolean;
  /** Transient project-scope status line (board): GitHub import / auto-pull
   * outcomes (#120). Quiet, non-modal; the next write replaces it. */
  projectNotice: string | null;
  /** Current width of the right sheet (ChangesSidebar), shared so the diff
   * view can reserve it instead of being covered. */
  sheetWidth: number;
  /** Bumped when keybindings change so hint renderers re-read the registry
   * (registry lives outside zustand). */
  bindingsVersion: number;

  setPaletteOpen: (open: boolean) => void;
  setCreateProjectOpen: (open: boolean) => void;
  setResourceOpen: (open: boolean) => void;
  setChangesOpen: (open: boolean) => void;
  setProjectChatOpen: (open: boolean) => void;
  setTaskChatOpen: (open: boolean) => void;
  setFileTreeOpen: (open: boolean) => void;
  setBoardDetailIssueId: (id: string | null) => void;
  setDrawerOpen: (open: boolean) => void;
  setDrawerScript: (script: ScriptType) => void;
  setSettingsOpen: (open: boolean) => void;
  setProjectSettingsOpen: (open: boolean) => void;
  toggleSidebarVisible: () => void;
  setSidebarVisible: (visible: boolean) => void;
  setDeleteTaskTarget: (target: DeleteTaskTarget | null) => void;
  setCreateTaskTarget: (projectId: string | null) => void;
  setDeleteProjectTarget: (id: string | null) => void;
  setQuickTaskTarget: (target: QuickTaskTarget | null) => void;
  setOnboardingOpen: (open: boolean) => void;
  setProjectNotice: (text: string | null) => void;
  setSheetWidth: (width: number) => void;
  bumpBindings: () => void;
  /** Esc handling (modal scope): close the topmost modal. */
  closeTopModal: () => void;
  modalOpen: () => boolean;
}

/** The flyout is pinned; its collapsed state persists across relaunches
 * (v1 README: "⌘\ toggles it and the state persists"). */
const SIDEBAR_KEY = "fc:sidebarVisible";
function persistSidebar(visible: boolean): void {
  try {
    localStorage.setItem(SIDEBAR_KEY, visible ? "1" : "0");
  } catch {
    /* storage unavailable — in-memory only */
  }
}
function initialSidebarVisible(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_KEY) !== "0";
  } catch {
    return true;
  }
}

export const useUi = create<UiState>((set, get) => ({
  paletteOpen: false,
  createProjectOpen: false,
  resourceOpen: false,
  changesOpen: false,
  projectChatOpen: true,
  taskChatOpen: false,
  fileTreeOpen: false,
  boardDetailIssueId: null,
  drawerOpen: false,
  drawerScript: "setup",
  settingsOpen: false,
  projectSettingsOpen: false,
  sidebarVisible: initialSidebarVisible(),
  deleteTaskTarget: null,
  createTaskTarget: null,
  deleteProjectTarget: null,
  quickTaskTarget: null,
  onboardingOpen: false,
  projectNotice: null,
  sheetWidth: 400,
  bindingsVersion: 0,

  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  setCreateProjectOpen: (createProjectOpen) => set({ createProjectOpen }),
  setResourceOpen: (resourceOpen) => set({ resourceOpen }),
  setChangesOpen: (changesOpen) => set({ changesOpen }),
  setProjectChatOpen: (projectChatOpen) => set({ projectChatOpen }),
  setTaskChatOpen: (taskChatOpen) => set({ taskChatOpen }),
  setFileTreeOpen: (fileTreeOpen) => set({ fileTreeOpen }),
  setBoardDetailIssueId: (boardDetailIssueId) => set({ boardDetailIssueId }),
  setDrawerOpen: (drawerOpen) => set({ drawerOpen }),
  setDrawerScript: (drawerScript) => set({ drawerScript }),
  setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
  setProjectSettingsOpen: (projectSettingsOpen) => set({ projectSettingsOpen }),
  toggleSidebarVisible: () =>
    set((s) => {
      persistSidebar(!s.sidebarVisible);
      return { sidebarVisible: !s.sidebarVisible };
    }),
  setSidebarVisible: (sidebarVisible) => {
    persistSidebar(sidebarVisible);
    set({ sidebarVisible });
  },
  setDeleteTaskTarget: (deleteTaskTarget) => set({ deleteTaskTarget }),
  setCreateTaskTarget: (createTaskTarget) => set({ createTaskTarget }),
  setDeleteProjectTarget: (deleteProjectTarget) => set({ deleteProjectTarget }),
  setQuickTaskTarget: (quickTaskTarget) => set({ quickTaskTarget }),
  setOnboardingOpen: (onboardingOpen) => set({ onboardingOpen }),
  setProjectNotice: (projectNotice) => set({ projectNotice }),
  setSheetWidth: (sheetWidth) => set({ sheetWidth }),
  bumpBindings: () => set((s) => ({ bindingsVersion: s.bindingsVersion + 1 })),

  closeTopModal: () => {
    const s = get();
    if (s.paletteOpen) return set({ paletteOpen: false });
    if (s.quickTaskTarget) return set({ quickTaskTarget: null });
    if (s.deleteTaskTarget) return set({ deleteTaskTarget: null });
    if (s.createTaskTarget) return set({ createTaskTarget: null });
    if (s.deleteProjectTarget) return set({ deleteProjectTarget: null });
    if (s.projectSettingsOpen) return set({ projectSettingsOpen: false });
    if (s.settingsOpen) return set({ settingsOpen: false });
    if (s.createProjectOpen) return set({ createProjectOpen: false });
  },

  modalOpen: () => {
    const s = get();
    return (
      s.paletteOpen ||
      s.createProjectOpen ||
      s.settingsOpen ||
      s.projectSettingsOpen ||
      s.deleteTaskTarget !== null ||
      s.createTaskTarget !== null ||
      s.deleteProjectTarget !== null ||
      s.quickTaskTarget !== null ||
      s.onboardingOpen
    );
  },
}));
