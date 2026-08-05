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

interface UiState {
  paletteOpen: boolean;
  createProjectOpen: boolean;
  resourceOpen: boolean;
  /** Changes sidebar (E4-03): right-side panel in the task view. */
  changesOpen: boolean;
  /** App settings (E14-01 shortcut customization lives here). */
  settingsOpen: boolean;
  /** Project settings modal (opened from the sidebar gear). */
  projectSettingsOpen: boolean;
  sidebarVisible: boolean;
  deleteTaskTarget: DeleteTaskTarget | null;
  deleteProjectTarget: string | null;
  discardTarget: DiscardTarget | null;
  onboardingOpen: boolean;
  /** Bumped when keybindings change so hint renderers re-read the registry
   * (registry lives outside zustand). */
  bindingsVersion: number;

  setPaletteOpen: (open: boolean) => void;
  setCreateProjectOpen: (open: boolean) => void;
  setResourceOpen: (open: boolean) => void;
  setChangesOpen: (open: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setProjectSettingsOpen: (open: boolean) => void;
  toggleSidebarVisible: () => void;
  setSidebarVisible: (visible: boolean) => void;
  setDeleteTaskTarget: (target: DeleteTaskTarget | null) => void;
  setDeleteProjectTarget: (id: string | null) => void;
  setDiscardTarget: (target: DiscardTarget | null) => void;
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
  settingsOpen: false,
  projectSettingsOpen: false,
  sidebarVisible: true,
  deleteTaskTarget: null,
  deleteProjectTarget: null,
  discardTarget: null,
  onboardingOpen: false,
  bindingsVersion: 0,

  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  setCreateProjectOpen: (createProjectOpen) => set({ createProjectOpen }),
  setResourceOpen: (resourceOpen) => set({ resourceOpen }),
  setChangesOpen: (changesOpen) => set({ changesOpen }),
  setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
  setProjectSettingsOpen: (projectSettingsOpen) => set({ projectSettingsOpen }),
  toggleSidebarVisible: () => set((s) => ({ sidebarVisible: !s.sidebarVisible })),
  setSidebarVisible: (sidebarVisible) => set({ sidebarVisible }),
  setDeleteTaskTarget: (deleteTaskTarget) => set({ deleteTaskTarget }),
  setDeleteProjectTarget: (deleteProjectTarget) => set({ deleteProjectTarget }),
  setDiscardTarget: (discardTarget) => set({ discardTarget }),
  setOnboardingOpen: (onboardingOpen) => set({ onboardingOpen }),
  bumpBindings: () => set((s) => ({ bindingsVersion: s.bindingsVersion + 1 })),

  closeTopModal: () => {
    const s = get();
    if (s.paletteOpen) return set({ paletteOpen: false });
    if (s.discardTarget) return set({ discardTarget: null });
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
      s.onboardingOpen
    );
  },
}));
