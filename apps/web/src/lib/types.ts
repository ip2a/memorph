export type ProviderInfo = {
  id: string;
  name: string;
  scan: boolean;
  import: boolean;
  export: boolean;
  delete: boolean;
  rename: boolean;
  resume: boolean;
};

export type ImportSessionPayload = {
  provider: string;
  file_or_id: string;
  to_dir?: string | null;
};

export type ImportSessionResult = {
  provider_name: string;
  new_session_id: string;
  resume_command?: string | null;
};

export type ExportSessionPayload = {
  provider: string;
  session_id: string;
  output_prefix?: string | null;
  format: string;
  output_dir?: string | null;
};

export type ExportSessionResult = {
  files: string[];
};

export type SwitchSessionPayload = {
  from: string;
  to: string;
  session_id?: string | null;
  to_dir?: string | null;
  target_title?: string | null;
  move_original?: boolean;
};

export type SwitchSessionResult = {
  from_name: string;
  to_name: string;
  source_session_id: string;
  target_session_id: string;
  resume_command?: string | null;
  removed_original: boolean;
};

export type CreateSyncPayload = {
  provider: string;
  session_id: string;
  targets: string[];
  to_dir?: string | null;
  title?: string | null;
};

export type RenameSyncGroupPayload = {
  title: string;
};

export type SyncRunPayload = {
  group_id: string;
  source_holding_id?: string | null;
};

export type SyncReport = {
  source_provider: string;
  source_holding_id: string;
  success: string[];
  errors: string[];
};

export type BindSyncPayload = {
  group_id: string;
  provider: string;
  session_id?: string | null;
  to_dir?: string | null;
};

export type RenameSessionPayload = {
  title: string;
};

export type RenameSessionResult = {
  provider_name: string;
  session_id: string;
  display_title: string;
  native_updated: boolean;
  warning?: string | null;
};

export type ManagerFilter = {
  providers?: string[];
  older_than_days?: number;
  older_than_ms?: number;
  larger_than_mb?: number;
  larger_than_bytes?: number;
  smaller_than_bytes?: number;
  workspace?: string;
  sort?: "recent" | "size" | string;
  limit?: number;
};

export type ManagerItem = {
  provider_id: string;
  provider_name: string;
  session_id: string;
  source_path: string | null;
  title: string | null;
  project_dir: string | null;
  last_active_at: number | null;
  size_bytes: number;
};

export type ManagerPreviewResult = {
  items: ManagerItem[];
  total_count: number;
  total_size_bytes: number;
};

export type ManagerQuickPreviewResult = ManagerPreviewResult & {
  selected_agent_count: number;
};

export type ManagerWorkspaceItem = {
  provider_id: string;
  provider_name: string;
  workspace: string;
  session_count: number;
  total_size_bytes: number;
  last_active_at: number | null;
};

export type ManagerWorkspacesResult = {
  items: ManagerWorkspaceItem[];
  total_count: number;
  total_size_bytes: number;
};

export type ManagerStatsResult = {
  selected_agent_count: number;
  current_workspace_session_count: number;
  current_workspace_size_bytes: number;
  all_workspace_count: number;
  all_workspace_session_count: number;
  all_workspace_size_bytes: number;
};

export type ManagerItemsPayload = {
  items: ManagerItem[];
  output_dir?: string | null;
};

export type ManagerWorkspacePayload = {
  provider_id: string;
  workspace: string;
  output_dir?: string | null;
};

export type ManagerCleanResult = {
  success: number;
  failed: number;
  freed_bytes: number;
  errors: string[];
};

export type ManagerBackupResult = {
  success: number;
  failed: number;
  files: string[];
  errors: string[];
};

export type CompressionProjection = "native" | "portable" | string;

export type CompressionProviderSupport = {
  provider_id: string;
  detects_native_source: boolean;
  native_target_projection: boolean;
  default_projection: CompressionProjection;
};

export type CompressionArchivesParams = {
  workspace?: string;
  offset?: number;
  limit?: number;
};

export type CompressionArchiveSummary = {
  archive_ref: string;
  created_at: string;
  canonical_id: string;
  source_provider_id: string;
  target_provider_id: string;
  workspace_dir?: string | null;
  summary_event_id: string;
  source_event_count: number;
  original_size_bytes: number;
  stored_size_bytes: number;
  compression_ratio: number;
};

export type CompressionFormat = "json" | "md" | "html" | "morph" | "both" | string;

export type ActiveCompressionPolicy = {
  protect_recent_message_events: number;
  min_candidate_bytes: number;
  min_savings_ratio_percent: number;
  mode: "plan_only" | "auto" | "manual" | string;
};

export type ApplyCompressionPayload = {
  source_provider_id: string;
  target_provider_id: string;
  session_id?: string | null;
  file?: string | null;
  policy: ActiveCompressionPolicy;
  candidate_ids?: string[];
  output_prefix?: string | null;
  format?: CompressionFormat;
};

export type ActiveCompressionReport = {
  source_provider_id: string;
  target_provider_id: string;
  dry_run: boolean;
  session_event_count: number;
  message_event_count: number;
  already_compressed_event_count: number;
  original_estimated_bytes: number;
  compressed_estimated_bytes: number;
  estimated_bytes_saved: number;
  estimated_tokens_saved: number;
  candidates: Array<{ id: string; archive_refs?: string[] }>;
  skipped: Array<Record<string, unknown>>;
  archive_refs: string[];
};

export type ApplyCompressionResult = {
  files: string[];
  archive_refs: string[];
  report: ActiveCompressionReport;
};

export type RestoreCompressionPayload = {
  archive_ref: string;
  output_prefix?: string | null;
  format: CompressionFormat;
};

export type RestoreCompressionResult = {
  files: string[];
};

export type UiLanguage = "auto" | "en" | "zh" | string;

export type WorkspaceEntry = {
  path: string;
  name?: string;
  last_viewed_at?: number;
  providers?: string[];
};

export type SettingsPayload = {
  sessions_per_provider: number;
  language: UiLanguage;
  show_opencode_subagents: boolean;
  sort_providers_by_session_count: boolean;
  default_backup_dir: string;
  logging: Record<string, unknown>;
  home_buttons: Record<string, unknown>;
  agent_order: string[];
  primary_agents: string[];
  server?: ServerSettingsPayload;
};

export type LogSettingsPayload = {
  max_size_bytes?: number | null;
  retention_days?: number | null;
};

export type HomeButtonSettingsPayload = {
  view?: boolean;
  compress?: boolean;
  switch?: boolean;
  export?: boolean;
  sync?: boolean;
  delete?: boolean;
};

export type ServerSettingsPayload = {
  web_port: number;
  api_port: number;
};

export type UpdateSettingsPayload = {
  sessions_per_provider: number;
  language: UiLanguage;
  show_opencode_subagents: boolean;
  sort_providers_by_session_count?: boolean;
  default_backup_dir: string;
  logging: LogSettingsPayload;
  home_buttons: HomeButtonSettingsPayload;
  agent_order: string[];
  primary_agents: string[];
  server: ServerSettingsPayload;
};

export type ProviderCatalogPreferenceList = {
  global: string[];
  workspace: string[];
};

export type ProviderCatalogUpdatePayload = {
  sort_order: ProviderCatalogPreferenceList;
  hidden_state: ProviderCatalogPreferenceList;
  workspace?: string | null;
};

export type ProviderCatalogEntry = {
  provider_id: string;
  display_name: string;
  filter_tags?: string[];
  hidden_state?: {
    global?: boolean;
    workspace?: boolean;
  };
  install_state?: {
    is_installed?: boolean;
    exec_path?: string | null;
    exec_dir?: string | null;
    config_path?: string | null;
    install_method?: string | null;
  };
};

export type ProviderCatalogPayload = {
  providers: ProviderCatalogEntry[];
};

export type SelectPathPayload = {
  start_path?: string | null;
};

export type SelectPathResult = {
  path: string | null;
};

export type OpenExternalPayload = {
  url: string;
};

export type OpenExternalResult = {
  opened: boolean;
};

export type UpdateCheckPayload = {
  current_version: string;
  latest_version: string;
  install_source: string;
  install_source_label: string;
  has_update: boolean;
  update_command: string;
  release_url: string;
};

export type SettingsPathsPayload = {
  backup_dir_input: string;
  backup_dir_resolved: string;
  backup_dir_base: string;
  log_dir: string;
  log_file_name: string;
  log_file_path: string;
};

export type ConfigFilePayload = {
  path: string;
  format: string;
  content: string;
};

export type MetaPayload = {
  version: string;
  selected_workspace: string | null;
  workspaces: WorkspaceEntry[];
  settings: SettingsPayload;
  settings_paths: SettingsPathsPayload;
  config_file: ConfigFilePayload;
};

export type SessionListSort = "recent" | "title" | "hook_attention";

export type SessionHookFilter = "all" | "attention" | "weak" | "runtime" | "no_hook" | "no_match" | "linked";

export type SessionListParams = {
  all?: boolean;
  provider?: string;
  dir?: string;
  workspace?: string;
  details?: boolean;
  limit?: number;
  offset?: number;
  sort?: SessionListSort;
  hook_filter?: SessionHookFilter;
};

export type HookRuntimeSummary = Record<string, unknown>;

export type SessionHookDiagnosis = {
  kind?: string;
  message?: string;
  provider_status?: string;
  provider_runtime_sessions?: number;
  actions?: Array<Record<string, unknown>>;
  [key: string]: unknown;
};

export type SessionItem = {
  session_id: string;
  title: string | null;
  native_title?: string | null;
  display_title?: string | null;
  hidden: boolean;
  pinned: boolean;
  preferred_targets: string[];
  project_dir: string | null;
  last_active_at: number | null;
  source_path: string | null;
  provider_id: string;
  message_count: number | null;
  size_bytes: number | null;
  hook_runtime_summary?: HookRuntimeSummary | null;
  hook_diagnosis?: SessionHookDiagnosis | null;
};

export type SessionDetailParams = {
  event_offset?: number;
  event_limit?: number;
};

export type MappingDisposition = "preserved" | "normalized" | "downgraded" | "dropped" | "unsupported" | string;

export type EventRole = "user" | "assistant" | "tool" | "system" | "developer" | "unknown" | string;

export type SessionEventKind =
  | "message"
  | "tool_call"
  | "tool_result"
  | "command"
  | "command_result"
  | "patch"
  | "lifecycle"
  | "artifact"
  | "unknown"
  | string;

export type EventLinks = {
  parent_event_id?: string | null;
  provider_parent_id?: string | null;
  turn_index?: number | null;
  related_event_ids?: string[];
};

export type UsageStats = {
  input_tokens?: number | null;
  output_tokens?: number | null;
  total_tokens?: number | null;
};

export type EventSource = {
  provider_id: string;
  original_id?: string | null;
  original_role?: string | null;
  phase?: string | null;
};

export type EventMetadata = {
  source: EventSource;
  model?: string | null;
  usage?: UsageStats | null;
  fidelity: MappingDisposition;
  provider_ext?: Record<string, unknown>;
};

export type EventBlock =
  | { type: "text"; text: string }
  | { type: "thinking"; text: string; signature?: string | null }
  | { type: "tool_call"; tool_call_id: string; name: string; input?: unknown }
  | { type: "tool_result"; tool_call_id: string; content: string; is_error?: boolean }
  | { type: "patch"; summary?: string | null; diff_text?: string | null; files?: string[]; hash?: string | null }
  | { type: "command"; command: string; argv?: string[]; cwd?: string | null }
  | { type: "command_result"; command?: string | null; exit_code?: number | null; stdout?: string | null; stderr?: string | null }
  | { type: "file"; path: string; content?: string | null; mime_type?: string | null }
  | { type: "image"; mime_type: string; data?: string | null; path?: string | null }
  | { type: "provider_payload"; kind: string; payload: unknown }
  | { type: "compressed"; source_provider_id: string; summary: string; source_event_ids?: string[]; source_event_count?: number | null; archive_ref?: string | null }
  | { type: "unknown"; raw: unknown };

export type SessionEvent = {
  id: string;
  kind: SessionEventKind;
  role: EventRole;
  timestamp: string;
  links?: EventLinks;
  blocks: EventBlock[];
  metadata: EventMetadata;
};

export type CompressionArchive = {
  version: number;
  created_at: string;
  canonical_id: string;
  source_provider_id: string;
  target_provider_id: string;
  workspace_dir?: string | null;
  summary_event_id: string;
  source_event_ids: string[];
  events: SessionEvent[];
};

export type SessionArtifactKind = "file" | "image" | "patch" | "attachment" | "unknown" | string;

export type SessionArtifact = {
  id: string;
  kind: SessionArtifactKind;
  path?: string | null;
  mime_type?: string | null;
  content?: string | null;
  metadata?: Record<string, unknown>;
};

export type LocalSessionState = {
  display_title?: string | null;
  archived: boolean;
  hidden: boolean;
  pinned: boolean;
  notes?: string | null;
  tags: string[];
  preferred_targets: string[];
  compressed_archive_refs: string[];
};

export type SessionDetailView = {
  provider_id: string;
  provider_name: string;
  session_id: string;
  canonical_id: string;
  title?: string | null;
  native_title?: string | null;
  display_title?: string | null;
  workspace_dir?: string | null;
  created_at?: string | null;
  last_active_at?: string | null;
  source_path?: string | null;
  resume_command?: string | null;
  local_state: LocalSessionState;
  event_count: number;
  message_count: number;
  artifact_count: number;
  hook_runtime_summary?: HookRuntimeSummary | null;
  hook_diagnosis?: SessionHookDiagnosis | null;
  hook_runtime_sessions: unknown[];
  events: SessionEvent[];
  artifacts: SessionArtifact[];
  compressed_archive_refs: string[];
};

export type SessionDetailPayload = {
  view: SessionDetailView;
  events_offset: number;
  events_limit: number | null;
  returned_event_count: number;
  has_more_events: boolean;
  hook_runtime_sessions: unknown[];
};

export type SessionActivityBucketUnit = "minute" | "hour" | "twelve_hour";

export type SessionActivityBucket = {
  start: string;
  end: string;
  event_count: number;
  message_count: number;
  activity_score: number;
};

export type SessionActivityTimeline = {
  provider_id: string;
  session_id: string;
  created_at?: string | null;
  last_active_at?: string | null;
  bucket_unit: SessionActivityBucketUnit;
  bucket_seconds: number;
  buckets: SessionActivityBucket[];
  total_events: number;
  total_messages: number;
  total_activity: number;
};

export type ProviderActivityTimeline = {
  provider_id: string;
  hours: number;
  bucket_seconds: number;
  range_start: string;
  range_end: string;
  buckets: SessionActivityBucket[];
  total_activity: number;
  sessions_scanned: number;
  sessions_considered: number;
};

export type SessionGroup = {
  provider_id: string;
  provider_name: string;
  sessions: SessionItem[];
};

export type SyncHolding = {
  id: string;
  provider: string;
  session_id: string;
  target_dir: string | null;
  created_at: number;
  last_active_at: number | null;
  last_sync_at: number | null;
  last_sync_from: string | null;
  last_error: string | null;
  hook_runtime_summary?: HookRuntimeSummary | null;
  hook_diagnosis?: SessionHookDiagnosis | null;
  hook_runtime_sessions?: unknown[];
};

export type SyncGroup = {
  id: string;
  title: string;
  source_provider: string | null;
  created_at: number;
  updated_at: number;
  holdings: SyncHolding[];
};

export type AgentEnvironmentStatus = {
  installed: boolean;
  executable_path?: string | null;
  executable_dir?: string | null;
  config_path: string;
  install_method: string;
  executable_version?: string | null;
};

export type HookInstallStatus = {
  provider?: string;
  status?: string;
  message?: string | null;
  installed_version?: string | null;
  current_version?: string | null;
  config_path?: string | null;
  last_event_at?: string | null;
  [key: string]: unknown;
};

export type HookCapabilities = {
  detect?: boolean;
  verify?: boolean;
  install?: boolean;
  repair?: boolean;
  uninstall?: boolean;
  [key: string]: unknown;
};

export type HookProfileEvent = {
  name: string;
  blocking?: boolean;
  [key: string]: unknown;
};

export type HookProviderProfile = {
  events?: HookProfileEvent[];
  [key: string]: unknown;
};

export type ProviderHookDiagnosisAggregate = {
  total_sessions?: number;
  linked?: number;
  weakly_linked?: number;
  hook_needs_attention?: number;
  no_session_match?: number;
  no_active_runtime?: number;
  no_events_yet?: number;
  hook_not_installed?: number;
  active_runtime_sessions?: number;
  recommended_actions?: Array<{
    setting_id: string;
    label: string;
    reason?: string;
  }>;
  [key: string]: unknown;
};

export type HookServerStatus = {
  running: boolean;
  endpoint?: string | null;
  pid?: number | null;
  started_at?: string | null;
};

export type HookOverviewSummary = {
  providers: number;
  supported_providers: number;
  installed_ok: number;
  not_installed: number;
  needs_attention: number;
  active_runtime_sessions: number;
  linked_sessions: number;
  weakly_linked_sessions: number;
  no_session_match: number;
  recent_errors: number;
};

export type HookToolCall = {
  id?: string | null;
  name?: string | null;
  input?: unknown;
  [key: string]: unknown;
};

export type HookMessage = {
  role?: string | null;
  content?: string | null;
  [key: string]: unknown;
};

export type HookEventRecord = {
  event_id: string;
  provider: string;
  event_type: string;
  provider_session_id?: string | null;
  run_id?: string | null;
  cwd?: string | null;
  tool?: HookToolCall | null;
  message?: HookMessage | null;
  timestamp: string;
  [key: string]: unknown;
};

export type HookRuntimeSession = {
  runtime_id: string;
  provider: string;
  provider_session_id?: string | null;
  run_id?: string | null;
  cwd?: string | null;
  pid?: number | null;
  correlation?: Record<string, unknown> | null;
  model?: string | null;
  session_title?: string | null;
  status: string;
  current_tool?: HookToolCall | null;
  last_error?: string | null;
  last_event_at: string;
  updated_at: string;
  [key: string]: unknown;
};

export type HookErrorRecord = {
  timestamp: string;
  scope: string;
  message: string;
};

export type HookOverviewPayload = {
  generated_at: string;
  summary: HookOverviewSummary;
  server: HookServerStatus;
  providers: AgentManagementEntry[];
  runtime_sessions: HookRuntimeSession[];
  recent_errors: HookErrorRecord[];
  recent_events: HookEventRecord[];
};

export type HookProviderOverviewPayload = {
  generated_at: string;
  provider: AgentManagementEntry;
  runtime_sessions: HookRuntimeSession[];
  recent_events: HookEventRecord[];
  recent_errors: HookErrorRecord[];
};

export type HookOperationReport = {
  changed?: boolean;
  message?: string | null;
  status?: HookInstallStatus;
  [key: string]: unknown;
};

export type ProviderSettingItem = {
  id: string;
  title: string;
  description: string;
  scope: "global" | "workspace" | "session" | string;
  kind: "toggle" | "action" | "view" | string;
  value?: unknown;
};

export type ProviderSettingOutput = {
  type?: string;
  data?: unknown;
  [key: string]: unknown;
};

export type CodexWorkspaceRepairItem = {
  session_id: string;
  title?: string | null;
  rollout_path: string;
  workspace_dir?: string | null;
  previous_model_provider?: string | null;
  current_model_provider: string;
  updated_model_provider: boolean;
  added_to_index: boolean;
  updated_index_title: boolean;
};

export type CodexWorkspaceRepairReport = {
  workspace_dir: string;
  current_model_provider: string;
  scanned_rollouts: number;
  workspace_session_count: number;
  hidden_session_count: number;
  repaired_session_count: number;
  reindexed_session_count: number;
  retitled_session_count?: number;
  backup_dir?: string | null;
  sqlite_rows_updated: number;
  sqlite_provider_rows_updated: number;
  sqlite_user_event_rows_updated: number;
  sqlite_cwd_rows_updated: number;
  pruned_backup_count: number;
  skipped_rollout_files?: string[];
  touched_sessions: CodexWorkspaceRepairItem[];
};

export type AgentManagementEntry = {
  provider_id: string;
  name: string;
  environment: AgentEnvironmentStatus;
  hook: HookInstallStatus;
  hook_strategy?: string | null;
  hook_capabilities: HookCapabilities;
  hook_diagnosis: ProviderHookDiagnosisAggregate;
  hook_profile?: HookProviderProfile | null;
  hook_required_events: string[];
  settings: ProviderSettingItem[];
  installed?: boolean;
  executable_path?: string;
  executable_dir?: string;
  config_path?: string;
  install_method?: string;
};

export type AgentManagementPayload = {
  providers: AgentManagementEntry[];
};

export type ProviderSettingsPayload = {
  provider_id: string;
  settings: ProviderSettingItem[];
};

export type ProviderSummary = {
  id: string;
  name: string;
  sessionCount: number;
  workspaceCount?: number;
};

export type SessionSummary = {
  provider: string;
  id: string;
  title: string;
  updatedAt?: string;
  workspace?: string;
};

export type WorkflowStatus = "planned" | "in-progress" | "implemented" | "verified";
