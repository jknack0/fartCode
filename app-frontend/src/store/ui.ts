// Global UI flags + modal registry (E1-09 palette wiring, E14-01 modal
// scope). The keybinding dispatch reads this store to decide whether a
// modal scope is active and what Esc closes first.
import { create } from "zustand";

export interface DeleteTaskTarget {
  projectId: string;
  taskId: string;
}

/** Pending discard confirmation (E4-03): the modal shows the path list,
 * with an extra warning when any path is untracked (deletes the file). */
export interface DiscardTarget {
  workspaceId: string;
  paths: string[];
  hasUntracked: boolean;
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
  /** Open card-detail issue id (E17-02): takes precedence over the chat
   * panel in the project view's right region; null shows the chat. */
  boardDetailIssueId: string | null;
  /** App settings (E14-01 shortcut customization lives here). */
  settingsOpen: boolean;
  /** Project settings modal (opened from the sidebar gear). */
  projectSettingsOpen: boolean;
  sidebarVisible: boolean;
  deleteTaskTarget: DeleteTaskTarget | null;
  deleteProjectTarget: string | null;
  discardTarget: DiscardTarget | null;
  quickTaskTarget: QuickTaskTarget | null;
  onboardingOpen: boolean;
  /** Bumped when keybindings change so hint renderers re-read the registry
   * (registry lives outside zustand). */
  bindingsVersion: number;

  setPaletteOpen: (open: boolean) => void;
  setCreateProjectOpen: (open: boolean) => void;
  setResourceOpen: (open: boolean) => void;
  setChangesOpen: (open: boolean) => void;
  setProjectChatOpen: (open: boolean) => void;
  setBoardDetailIssueId: (id: string | null) => void;
  setSettingsOpen: (open: boolean) => void;
  setProjectSettingsOpen: (open: boolean) => void;
  toggleSidebarVisible: () => void;
  setSidebarVisible: (visible: boolean) => void;
  setDeleteTaskTarget: (target: DeleteTaskTarget | null) => void;
  setDeleteProjectTarget: (id: string | null) => void;
  setDiscardTarget: (target: DiscardTarget | null) => void;
  setQuickTaskTarget: (target: QuickTaskTarget | null) => void;
  setOnboardingOpen: (open: boolean) => void;
  bumpBindings: () => void;
  /** Esc handling (modal scope): close the topmost modal. */
  closeTopModal: () => void;
  modalOpen: () => boolean;
}

export const useUi = create<UiState>((set, get) => ({
  paletteOpen: false,
  createProjectOpen: false,
  resourceOpen: false,
  changesOpen: false,
  projectChatOpen: true,
  boardDetailIssueId: null,
  settingsOpen: false,
  projectSettingsOpen: false,
  sidebarVisible: true,
  deleteTaskTarget: null,
  deleteProjectTarget: null,
  discardTarget: null,
  quickTaskTarget: null,
  onboardingOpen: false,
  bindingsVersion: 0,

  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  setCreateProjectOpen: (createProjectOpen) => set({ createProjectOpen }),
  setResourceOpen: (resourceOpen) => set({ resourceOpen }),
  setChangesOpen: (changesOpen) => set({ changesOpen }),
  setProjectChatOpen: (projectChatOpen) => set({ projectChatOpen }),
  setBoardDetailIssueId: (boardDetailIssueId) => set({ boardDetailIssueId }),
  setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
  setProjectSettingsOpen: (projectSettingsOpen) => set({ projectSettingsOpen }),
  toggleSidebarVisible: () => set((s) => ({ sidebarVisible: !s.sidebarVisible })),
  setSidebarVisible: (sidebarVisible) => set({ sidebarVisible }),
  setDeleteTaskTarget: (deleteTaskTarget) => set({ deleteTaskTarget }),
  setDeleteProjectTarget: (deleteProjectTarget) => set({ deleteProjectTarget }),
  setDiscardTarget: (discardTarget) => set({ discardTarget }),
  setQuickTaskTarget: (quickTaskTarget) => set({ quickTaskTarget }),
  setOnboardingOpen: (onboardingOpen) => set({ onboardingOpen }),
  bumpBindings: () => set((s) => ({ bindingsVersion: s.bindingsVersion + 1 })),

  closeTopModal: () => {
    const s = get();
    if (s.paletteOpen) return set({ paletteOpen: false });
    if (s.discardTarget) return set({ discardTarget: null });
    if (s.quickTaskTarget) return set({ quickTaskTarget: null });
    if (s.deleteTaskTarget) return set({ deleteTaskTarget: null });
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
      s.deleteProjectTarget !== null ||
      s.discardTarget !== null ||
      s.quickTaskTarget !== null ||
      s.onboardingOpen
    );
  },
}));
