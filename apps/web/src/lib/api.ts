import type {
  AgentManagementEntry,
  AgentManagementPayload,
  ArtifactCleanupReport,
  ArtifactCleanupRequest,
  ArtifactInspectionReport,
  ApplyCompressionPayload,
  ApplyCompressionResult,
  BackupQueryParams,
  BackupRestoreRecord,
  BackupView,
  BindSyncPayload,
  CompressionArchive,
  CompressionArchivesParams,
  CompressionArchiveSummary,
  CompressionProviderSupport,
  CreateSyncPayload,
  DirectoryListing,
  ExportSessionPayload,
  ExportSessionResult,
  HookOperationReport,
  HookOverviewPayload,
  HookProviderOverviewPayload,
  InstalledHooks,
  ImportSessionPayload,
  ImportSessionResult,
  MetaPayload,
  ManagerFilter,
  ManagerBackupResult,
  ManagerCleanResult,
  ManagerItemsPayload,
  ManagerPreviewResult,
  ManagerQuickPreviewResult,
  ManagerStatsResult,
  ManagerWorkspacePayload,
  NativeForkPayload,
  ManagerWorkspacesResult,
  WorkspacesWithSessionsParams,
  WorkspacesWithSessionsResult,
  ProviderSettingItem,
  ProviderSettingOutput,
  OpenExternalPayload,
  OpenExternalResult,
  ProviderInfo,
  ProviderCatalogPayload,
  ProviderCatalogUpdatePayload,
  RenameSyncGroupPayload,
  RenameSessionPayload,
  RenameSessionResult,
  RestoreCompressionPayload,
  RestoreCompressionResult,
  SelectPathPayload,
  SelectPathResult,
  SessionDetailParams,
  SessionDetailPayload,
  SessionActivityTimeline,
  ProviderActivityTimeline,
  SessionGroup,
  SessionListParams,
  SessionPage,
  SessionReprojectionReport,
  SkillMutation,
  SkillCatalogPage,
  SkillCatalogParams,
  SkillScanSummary,
  SkillDailyUsage,
  SkillContext,
  SkillContextSummary,
  SkillConflict,
  SkillCoverage,
  SkillCoverageSummaryItem,
  SkillCoverageEvidencePage,
  SkillHealth,
  SkillHealthSummary,
  SkillGraph,
  SkillGraphParams,
  SkillInvocationPage,
  SkillRanking,
  SkillPrunePreview,
  SkillPruneResult,
  SkillStatsParams,
  SkillStatsSummary,
  SkillStatsBreakdown,
  SkillsOverview,
  SkillDetail,
  SkillTree,
  SkillFilePreview,
  SessionStalenessRefreshReport,
  SwitchSessionPayload,
  SwitchSessionResult,
  SyncGroup,
  SyncHolding,
  SyncReport,
  SyncRunPayload,
  StatsDashboard,
  StatsDashboardRange,
  UpdateCheckPayload,
  UpdateSettingsPayload,
  WorkspaceEntry,
} from "@/lib/types";

export class ApiError extends Error {
  status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

const backendUnavailableStatuses = new Set([502, 503, 504]);

const backendUnavailableMessages = [
  "failed to fetch",
  "networkerror",
  "network request failed",
  "load failed",
  "connection refused",
  "econnrefused",
  "http 502",
  "http 503",
  "http 504",
];

export function isBackendUnavailableError(error: unknown): boolean {
  if (!error) return false;

  if (error instanceof ApiError) {
    return backendUnavailableStatuses.has(error.status);
  }

  if (error instanceof TypeError) {
    return true;
  }

  if (error instanceof Error) {
    const message = error.message.trim().toLowerCase();
    return backendUnavailableMessages.some((pattern) => message.includes(pattern));
  }

  return false;
}

type ApiEnvelope<T> = {
  ok?: boolean;
  data?: T;
  error?: string;
};

export async function api<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const headers = new Headers(options.headers);
  headers.set("Accept", "application/json");

  const body = options.body;
  if (
    body !== undefined &&
    !(body instanceof FormData) &&
    !headers.has("Content-Type")
  ) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(path, {
    ...options,
    headers,
    body,
  });

  const raw = (await response.json().catch(() => null)) as
    ApiEnvelope<T> | T | null;

  if (!response.ok) {
    const message =
      raw && typeof raw === "object" && "error" in raw
        ? raw.error
        : `HTTP ${response.status}`;
    throw new ApiError(message || `HTTP ${response.status}`, response.status);
  }

  if (raw && typeof raw === "object" && "ok" in raw) {
    if (!raw.ok) {
      throw new ApiError(
        raw.error || `HTTP ${response.status}`,
        response.status,
      );
    }
    return raw.data as T;
  }

  return raw as T;
}

function buildQuery(
  params: Record<string, string | number | boolean | null | undefined>,
) {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "") continue;
    search.set(key, String(value));
  }
  const query = search.toString();
  return query ? `?${query}` : "";
}

export function getMeta() {
  return api<MetaPayload>("/api/v1/meta");
}

export function updateSettings(payload: UpdateSettingsPayload) {
  return api<MetaPayload["settings"]>("/api/v1/settings", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function updateProviderCatalog(payload: ProviderCatalogUpdatePayload) {
  return api<ProviderCatalogPayload>("/api/v1/providers/catalog", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
}

export function getProviderCatalog(workspace?: string | null) {
  return api<ProviderCatalogPayload>(
    `/api/v1/providers/catalog${buildQuery({ workspace })}`,
  );
}

export function selectFolder(payload: SelectPathPayload) {
  return api<SelectPathResult>("/api/v1/system/select-folder", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listDirectories(path?: string | null) {
  return api<DirectoryListing>(
    `/api/v1/filesystem/directories${buildQuery({ path })}`,
  );
}

export function selectFile(payload: SelectPathPayload) {
  return api<SelectPathResult>("/api/v1/system/select-file", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function openExternal(payload: OpenExternalPayload) {
  return api<OpenExternalResult>("/api/v1/system/open-external", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function checkForUpdate() {
  return api<UpdateCheckPayload>("/api/v1/update-check");
}

export function listWorkspaces() {
  return api<WorkspaceEntry[]>("/api/v1/workspaces");
}

export function listWorkspacesWithSessions(
  params: WorkspacesWithSessionsParams = {},
) {
  return api<WorkspacesWithSessionsResult>(
    `/api/v1/workspaces/with-sessions${buildQuery(params)}`,
  );
}

export function deleteWorkspaceHistory(workspace: string) {
  return api<WorkspaceEntry[]>("/api/v1/workspaces/history", {
    method: "DELETE",
    body: JSON.stringify({ workspace }),
  });
}

export function getWorkspaceProviders(workspace: string) {
  return api<string[]>(
    `/api/v1/workspaces/providers${buildQuery({ workspace })}`,
  );
}

export function updateWorkspaceProviders(
  workspace: string,
  providers: string[],
) {
  return api<string[]>("/api/v1/workspaces/providers", {
    method: "PUT",
    body: JSON.stringify({ workspace, providers }),
  });
}

export function listAgentsSummary() {
  return api<AgentManagementPayload>("/api/v1/agents/summary");
}

export function getAgent(provider: string) {
  return api<AgentManagementEntry>(
    `/api/v1/agents/${encodeURIComponent(provider)}`,
  );
}

export function detectAgent(provider: string) {
  return api<AgentManagementEntry>(
    `/api/v1/agents/${encodeURIComponent(provider)}/detect`,
    {
      method: "POST",
    },
  );
}

export function updateProviderSetting(
  provider: string,
  settingId: string,
  value: unknown,
) {
  return api<ProviderSettingItem>(
    `/api/v1/providers/${encodeURIComponent(provider)}/settings/${encodeURIComponent(settingId)}`,
    {
      method: "PUT",
      body: JSON.stringify({ value }),
    },
  );
}

export function runProviderSetting(
  provider: string,
  settingId: string,
  workspace?: string | null,
) {
  return api<ProviderSettingOutput>(
    `/api/v1/providers/${encodeURIComponent(provider)}/settings/${encodeURIComponent(settingId)}`,
    {
      method: "POST",
      body: JSON.stringify({ workspace: workspace || null }),
    },
  );
}

export function getHooksOverview() {
  return api<HookOverviewPayload>("/api/v1/hooks/overview");
}

export function getHookProviderOverview(provider: string) {
  return api<HookProviderOverviewPayload>(
    `/api/v1/hooks/providers/${encodeURIComponent(provider)}/overview`,
  );
}

export function listInstalledHooks(provider: string) {
  return api<InstalledHooks>(
    `/api/v1/hooks/providers/${encodeURIComponent(provider)}/installed`,
  );
}

export function removeInstalledHook(
  provider: string,
  event: string,
  index: number,
  fingerprint: string,
) {
  return api<InstalledHooks>(
    `/api/v1/hooks/providers/${encodeURIComponent(provider)}/installed/${encodeURIComponent(event)}/${index}/${encodeURIComponent(fingerprint)}`,
    { method: "DELETE" },
  );
}

export function runHookProviderOperation(provider: string, operation: string) {
  return api<HookOperationReport>(
    `/api/v1/hooks/providers/${encodeURIComponent(provider)}/operations/${encodeURIComponent(operation)}`,
    { method: "POST" },
  );
}

export function listProviders() {
  return api<ProviderInfo[]>("/api/v1/providers");
}

export function importSession(payload: ImportSessionPayload) {
  return api<ImportSessionResult>("/api/v1/import", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function exportSession(payload: ExportSessionPayload) {
  return api<ExportSessionResult>("/api/v1/export", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function switchSession(payload: SwitchSessionPayload) {
  return api<SwitchSessionResult>("/api/v1/switch", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function nativeForkSession(payload: NativeForkPayload) {
  return api<SwitchSessionResult>("/api/v1/native-fork", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function createSyncGroup(payload: CreateSyncPayload) {
  return api<SyncGroup>("/api/v1/sync", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function renameSyncGroup(
  groupId: string,
  payload: RenameSyncGroupPayload,
) {
  return api<string>(`/api/v1/sync/${encodeURIComponent(groupId)}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  });
}

export function removeSyncGroup(
  groupId: string,
  deleteProviderSessions: boolean,
) {
  return api<string>(
    `/api/v1/sync/${encodeURIComponent(groupId)}${buildQuery({ delete_provider_sessions: deleteProviderSessions })}`,
    {
      method: "DELETE",
    },
  );
}

export function bindSyncGroup(payload: BindSyncPayload) {
  return api<SyncHolding>("/api/v1/sync/bind", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function runSyncGroup(payload: SyncRunPayload) {
  return api<SyncReport>("/api/v1/sync/sync", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function unbindSyncHolding(groupId: string, holdingId: string) {
  return api<string>(
    `/api/v1/sync/holdings/${encodeURIComponent(groupId)}/${encodeURIComponent(holdingId)}`,
    {
      method: "DELETE",
    },
  );
}

export function getManagerQuickPreview(providers: string[] = []) {
  return api<ManagerQuickPreviewResult>(
    `/api/v1/manager/quick-preview${buildQuery({ providers: providers.join(",") })}`,
  );
}

export function getManagerQuickWorkspaces(providers: string[] = []) {
  return api<ManagerWorkspacesResult>(
    `/api/v1/manager/quick-workspaces${buildQuery({ providers: providers.join(",") })}`,
  );
}

export function getManagerStats(filter: ManagerFilter) {
  return api<ManagerStatsResult>("/api/v1/manager/stats", {
    method: "POST",
    body: JSON.stringify(filter),
  });
}

export function getManagerPreview(filter: ManagerFilter) {
  return api<ManagerPreviewResult>("/api/v1/manager/preview", {
    method: "POST",
    body: JSON.stringify(filter),
  });
}

export function getManagerWorkspaces(filter: ManagerFilter) {
  return api<ManagerWorkspacesResult>("/api/v1/manager/workspaces", {
    method: "POST",
    body: JSON.stringify(filter),
  });
}

export function cleanManagerItems(payload: ManagerItemsPayload) {
  return api<ManagerCleanResult>("/api/v1/manager/clean", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function backupManagerItems(payload: ManagerItemsPayload) {
  return api<ManagerBackupResult>("/api/v1/manager/backup", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function cleanManagerWorkspace(payload: ManagerWorkspacePayload) {
  return api<ManagerCleanResult>("/api/v1/manager/clean-workspace", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function backupManagerWorkspace(payload: ManagerWorkspacePayload) {
  return api<ManagerBackupResult>("/api/v1/manager/backup-workspace", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function inspectArtifacts() {
  return api<ArtifactInspectionReport>("/api/v1/artifacts/inspection");
}

export function cleanupArtifacts(payload: ArtifactCleanupRequest) {
  return api<ArtifactCleanupReport>("/api/v1/artifacts/cleanup", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listBackups(params: BackupQueryParams = {}) {
  return api<BackupView[]>(`/api/v1/backups${buildQuery(params)}`);
}

export function getBackup(backupId: string) {
  return api<BackupView>(`/api/v1/backups/${encodeURIComponent(backupId)}`);
}

export function restoreBackup(backupId: string) {
  return api<BackupRestoreRecord>(
    `/api/v1/backups/${encodeURIComponent(backupId)}/restore`,
    {
      method: "POST",
    },
  );
}

export function listCompressionArchives(
  params: CompressionArchivesParams = {},
) {
  return api<CompressionArchiveSummary[]>(
    `/api/v1/compression/archives${buildQuery(params)}`,
  );
}

export function getCompressionArchive(archiveRef: string) {
  return api<CompressionArchive>(
    `/api/v1/compression/archive${buildQuery({ archive_ref: archiveRef })}`,
  );
}

export function listCompressionProviders() {
  return api<CompressionProviderSupport[]>("/api/v1/compression/providers");
}

export function applyCompression(payload: ApplyCompressionPayload) {
  return api<ApplyCompressionResult>("/api/v1/compression/apply", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function restoreCompressionArchive(payload: RestoreCompressionPayload) {
  return api<RestoreCompressionResult>("/api/v1/compression/restore", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function getStatsDashboard(params: {
  all: boolean;
  workspace?: string | null;
  range: StatsDashboardRange;
}) {
  return api<StatsDashboard>(`/api/v1/stats/dashboard${buildQuery(params)}`);
}

export function listSessions(params: SessionListParams = {}) {
  return api<SessionGroup[]>(`/api/v1/sessions${buildQuery(params)}`);
}

export function listSessionPage(params: SessionListParams = {}) {
  return api<SessionPage>(`/api/v1/sessions/page${buildQuery(params)}`);
}

export function refreshSessionStaleness() {
  return api<SessionStalenessRefreshReport>("/api/v1/sessions/refresh-stale", {
    method: "POST",
  });
}

export function reprojectStaleSessions(provider?: string | null) {
  return api<SessionReprojectionReport>("/api/v1/sessions/reproject-stale", {
    method: "POST",
    body: JSON.stringify({ provider: provider || null }),
  });
}

export function getSession(
  provider: string,
  sessionId: string,
  params: SessionDetailParams = {},
) {
  return api<SessionDetailPayload>(
    `/api/v1/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}${buildQuery(params)}`,
  );
}

export function getProviderActivity(
  provider: string,
  params: {
    all?: boolean;
    all_time?: boolean;
    hours?: number;
    workspace?: string;
  } = {},
) {
  return api<ProviderActivityTimeline>(
    `/api/v1/providers/${encodeURIComponent(provider)}/activity${buildQuery(params)}`,
  );
}

export function getSessionActivity(provider: string, sessionId: string) {
  return api<SessionActivityTimeline>(
    `/api/v1/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}/activity`,
  );
}

export function renameSession(
  provider: string,
  sessionId: string,
  payload: RenameSessionPayload,
) {
  return api<RenameSessionResult>(
    `/api/v1/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}`,
    {
      method: "PATCH",
      body: JSON.stringify(payload),
    },
  );
}

export function deleteSession(provider: string, sessionId: string) {
  return api<string>(
    `/api/v1/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}`,
    {
      method: "DELETE",
    },
  );
}

export function listSyncGroups() {
  return api<SyncGroup[]>("/api/v1/sync/status");
}

export function getSyncGroup(groupId: string) {
  return api<SyncGroup>(
    `/api/v1/sync/status${buildQuery({ group_id: groupId })}`,
  );
}

export function getSkills(params: SkillCatalogParams = {}) {
  return api<SkillCatalogPage>(`/api/v1/skills${buildQuery(params)}`);
}

export function scanSkills(mode: "incremental" | "full", workspace?: string) {
  return api<SkillScanSummary>("/api/v1/skills/scan", {
    method: "POST",
    body: JSON.stringify({ mode, workspace }),
  });
}

export function getSkillContextSummary(
  provider?: string,
  baselineTokens?: number,
) {
  return api<SkillContextSummary>(
    `/api/v1/skills/context/summary${buildQuery({ provider, baselineTokens })}`,
  );
}

export function getSkillContext(skillId: string, baselineTokens?: number) {
  return api<SkillContext>(
    `/api/v1/skills/${encodeURIComponent(skillId)}/context${buildQuery({ baselineTokens })}`,
  );
}

export function getSkillHealthSummary() {
  return api<SkillHealthSummary>("/api/v1/skills/health/summary");
}

export function getSkillHealth(skillId: string) {
  return api<SkillHealth>(
    `/api/v1/skills/${encodeURIComponent(skillId)}/health`,
  );
}

export function getSkillConflicts(skillId?: string) {
  return api<SkillConflict[]>(
    skillId
      ? `/api/v1/skills/${encodeURIComponent(skillId)}/conflicts`
      : "/api/v1/skills/conflicts",
  );
}

export function getSkillCoverageSummary(range = "90d") {
  return api<SkillCoverageSummaryItem[]>(
    `/api/v1/skills/coverage/summary${buildQuery({ range })}`,
  );
}

export function getSkillCoverage(skillId: string, range = "90d") {
  return api<SkillCoverage>(
    `/api/v1/skills/${encodeURIComponent(skillId)}/coverage${buildQuery({ range })}`,
  );
}

export function getSkillCoverageEvidence(
  skillId: string,
  targetKey: string,
  page = 1,
) {
  return api<SkillCoverageEvidencePage>(
    `/api/v1/skills/${encodeURIComponent(skillId)}/coverage/${encodeURIComponent(targetKey)}/evidence${buildQuery({ page, pageSize: 20 })}`,
  );
}

export function previewSkillPrune(days = 30) {
  return api<SkillPrunePreview>("/api/v1/skills/prune/preview", {
    method: "POST",
    body: JSON.stringify({ days }),
  });
}
export function executeSkillPrune(
  preview: SkillPrunePreview,
  installationIds: string[],
) {
  return api<SkillPruneResult[]>("/api/v1/skills/prune/execute", {
    method: "POST",
    body: JSON.stringify({
      preview_id: preview.preview_id,
      items: preview.items
        .filter((item) => installationIds.includes(item.installation_id))
        .map((item) => ({
          installation_id: item.installation_id,
          expected_fingerprint: item.expected_fingerprint,
        })),
      confirmation: "REMOVE_MANAGED_INSTALLATIONS",
    }),
  });
}

export function getSkillGraph(params: SkillGraphParams = {}) {
  return api<SkillGraph>(`/api/v1/skills/graph${buildQuery(params)}`);
}

export function getSkillStatsSummary(params: SkillStatsParams = {}) {
  return api<SkillStatsSummary>(
    `/api/v1/skills/stats/summary${buildQuery(params)}`,
  );
}

export function getSkillStatsDaily(params: SkillStatsParams = {}) {
  return api<SkillDailyUsage[]>(
    `/api/v1/skills/stats/daily${buildQuery(params)}`,
  );
}

export function getSkillStatsBreakdown(params: SkillStatsParams = {}) {
  return api<SkillStatsBreakdown>(
    `/api/v1/skills/stats/breakdown${buildQuery(params)}`,
  );
}

export function getSkillStatsRanking(params: SkillStatsParams = {}) {
  return api<SkillRanking[]>(
    `/api/v1/skills/stats/ranking${buildQuery(params)}`,
  );
}

export function getSkillInvocations(
  skillId: string,
  params: SkillStatsParams = {},
) {
  return api<SkillInvocationPage>(
    `/api/v1/skills/${encodeURIComponent(skillId)}/invocations${buildQuery(params)}`,
  );
}

export function getSkillDetail(skillId: string) {
  return api<SkillDetail>(`/api/v1/skills/${encodeURIComponent(skillId)}`);
}

export function getSkillTree(skillId: string) {
  return api<SkillTree>(`/api/v1/skills/${encodeURIComponent(skillId)}/tree`);
}

export function getSkillFilePreview(
  skillId: string,
  path: string,
  provider?: string,
) {
  return api<SkillFilePreview>(
    `/api/v1/skills/${encodeURIComponent(skillId)}/file${buildQuery({ path, provider })}`,
  );
}
export function installSkill(payload: SkillMutation) {
  return api<SkillsOverview>("/api/v1/skills/install", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function uninstallSkill(payload: SkillMutation) {
  return api<SkillsOverview>("/api/v1/skills/install", {
    method: "DELETE",
    body: JSON.stringify(payload),
  });
}
