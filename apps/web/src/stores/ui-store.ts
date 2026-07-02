import { create } from "zustand";

type UiState = {
  homeHeroCollapsed: boolean;
  selectedWorkspace: string | null;
  setHomeHeroCollapsed: (collapsed: boolean) => void;
  setSelectedWorkspace: (workspace: string | null) => void;
};

export const useUiStore = create<UiState>((set) => ({
  homeHeroCollapsed: false,
  selectedWorkspace: null,
  setHomeHeroCollapsed: (homeHeroCollapsed) => set({ homeHeroCollapsed }),
  setSelectedWorkspace: (selectedWorkspace) => set({ selectedWorkspace }),
}));
