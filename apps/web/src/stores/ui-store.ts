import { create } from "zustand";

type UiState = {
  homeHeroCollapsed: boolean;
  selectedWorkspace: string | null;
  workspaceSwitchOpen: boolean;
  workspaceQuickSwitch: {
    open: boolean;
    paths: string[];
  };
  setHomeHeroCollapsed: (collapsed: boolean) => void;
  setSelectedWorkspace: (workspace: string | null) => void;
  setWorkspaceSwitchOpen: (open: boolean) => void;
  openWorkspaceQuickSwitch: (paths: string | string[]) => void;
  closeWorkspaceQuickSwitch: () => void;
};

export const useUiStore = create<UiState>((set) => ({
  homeHeroCollapsed: false,
  selectedWorkspace: null,
  workspaceSwitchOpen: false,
  workspaceQuickSwitch: { open: false, paths: [] },
  setHomeHeroCollapsed: (homeHeroCollapsed) => set({ homeHeroCollapsed }),
  setSelectedWorkspace: (selectedWorkspace) => set({ selectedWorkspace }),
  setWorkspaceSwitchOpen: (workspaceSwitchOpen) => set({ workspaceSwitchOpen }),
  openWorkspaceQuickSwitch: (paths) =>
    set({
      workspaceQuickSwitch: {
        open: true,
        paths: (Array.isArray(paths) ? paths : [paths]).filter(Boolean),
      },
    }),
  closeWorkspaceQuickSwitch: () =>
    set({ workspaceQuickSwitch: { open: false, paths: [] } }),
}));
