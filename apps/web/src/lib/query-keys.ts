import type { CompressionArchivesParams, ManagerFilter, SessionDetailParams, SessionListParams } from "@/lib/types";

export const queryKeys = {
  home: ["home"] as const,
  meta: ["meta"] as const,
  workspaces: ["workspaces"] as const,
  workspaceProviders: (workspace: string) => ["workspaces", "providers", workspace] as const,
  providers: ["providers"] as const,
  providerCatalog: (workspace?: string | null) => ["providers", "catalog", workspace ?? "global"] as const,
  sessionsRoot: ["sessions"] as const,
  sessions: (params: SessionListParams = {}) => ["sessions", params] as const,
  session: (provider: string, sessionId: string, params: SessionDetailParams = {}) =>
    ["session", provider, sessionId, params] as const,
  syncGroups: ["sync-groups"] as const,
  syncGroup: (groupId: string) => ["sync-group", groupId] as const,
  syncStatus: (groupId?: string) => ["sync-status", groupId ?? "all"] as const,
  manager: (view: string, filter: ManagerFilter = {}) => ["manager", view, filter] as const,
  managerQuick: (providers: string[]) => ["manager", "quick", providers] as const,
  managerQuickWorkspaces: (providers: string[]) => ["manager", "quick-workspaces", providers] as const,
  managerStats: (filter: ManagerFilter = {}) => ["manager", "stats", filter] as const,
  compression: (params: CompressionArchivesParams = {}) => ["compression", params] as const,
  compressionArchive: (archiveRef: string) => ["compression-archive", archiveRef] as const,
  compressionProviders: ["compression-providers"] as const,
  agents: ["agents"] as const,
  agentsSummary: ["agents", "summary"] as const,
  agent: (provider: string) => ["agents", provider] as const,
  hooks: ["hooks"] as const,
  hookProvider: (provider: string) => ["hooks", provider] as const,
};
