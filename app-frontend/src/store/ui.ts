// Global UI flags shared across components (E1-09 palette wiring).
import { create } from "zustand";

interface UiState {
  paletteOpen: boolean;
  createProjectOpen: boolean;
  resourceOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  setCreateProjectOpen: (open: boolean) => void;
  setResourceOpen: (open: boolean) => void;
}

export const useUi = create<UiState>((set) => ({
  paletteOpen: false,
  createProjectOpen: false,
  resourceOpen: false,
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  setCreateProjectOpen: (createProjectOpen) => set({ createProjectOpen }),
  setResourceOpen: (resourceOpen) => set({ resourceOpen }),
}));
