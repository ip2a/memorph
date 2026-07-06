import { create } from "zustand";

type UiState = {
  homeHeroCollapsed: boolean;
  selectedWorkspace: string | null;
  workspaceSwitchOpen: boolean;
  setHomeHeroCollapsed: (collapsed: boolean) => void;
  setSelectedWorkspace: (workspace: string | null) => void;
  setWorkspaceSwitchOpen: (open: boolean) => void;
};

export const useUiStore = create<UiState>((set) => ({
  homeHeroCollapsed: false,
  selectedWorkspace: null,
  workspaceSwitchOpen: false,
  setHomeHeroCollapsed: (homeHeroCollapsed) => set({ homeHeroCollapsed }),
  setSelectedWorkspace: (selectedWorkspace) => set({ selectedWorkspace }),
  setWorkspaceSwitchOpen: (workspaceSwitchOpen) => set({ workspaceSwitchOpen }),
}));
