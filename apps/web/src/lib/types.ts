export type ProviderInfo = {
  id: string;
  name: string;
  scan: boolean;
  import: boolean;
  export: boolean;
  delete: boolean;
  rename: boolean;
  resume: boolean;
  native_fork: boolean;
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

export type NativeForkPayload = {
  provider: string;
  session_id: string;
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
  sort?: "recent" | "size" | "title" | "sessions";
  search?: string;
  offset?: number;
  limit?: number;
};

export type ManagerItem = {
  id: string;
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

export type WorkspaceWithSessionsItem = {
  path: string;
  session_count: number;
  last_active_at: number | null;
};

export type WorkspacesWithSessionsParams = {
  q?: string;
  page?: number;
  page_size?: number;
};

export type WorkspacesWithSessionsResult = {
  items: WorkspaceWithSessionsItem[];
  total_count: number;
  page: number;
  page_size: number;
  total_pages: number;
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

export type ArtifactManifestKind =
  | "compression_archive"
  | "database_backup"
  | "session_export"
  | "session_backup"
  | "event_payload";

export type ArtifactStorageKind = "file" | "directory" | "unknown";

export type ArtifactVerificationStatus =
  "verified" | "missing" | "changed" | "unverifiable";

export type ArtifactRetentionState =
  "current_event_payload" | "detached_event_payload" | "retained";

export type ArtifactManifest = {
  id: string;
  artifact_kind: ArtifactManifestKind;
  storage_kind: ArtifactStorageKind;
  operation_id?: string | null;
  provider_id?: string | null;
  provider_session_id?: string | null;
  session_id?: string | null;
  projection_report_id?: string | null;
  event_id?: string | null;
  block_id?: string | null;
  path: string;
  content_hash: string;
  byte_size: number;
  mime_type?: string | null;
  format?: string | null;
  created_at_ms: number;
  metadata: Record<string, unknown>;
};

export type ArtifactVerification = {
  artifact_id: string;
  path: string;
  status: ArtifactVerificationStatus;
  expected_content_hash: string;
  actual_content_hash?: string | null;
  expected_byte_size: number;
  actual_byte_size?: number | null;
};

export type ArtifactInspectionEntry = {
  manifest: ArtifactManifest;
  verification: ArtifactVerification;
  retention_state: ArtifactRetentionState;
};

export type OrphanArtifactFile = {
  path: string;
  byte_size: number;
  modified_at_ms: number;
  managed_layout: boolean;
};

export type ArtifactInspectionReport = {
  generated_at_ms: number;
  managed_blob_root: string;
  registered: ArtifactInspectionEntry[];
  orphan_files: OrphanArtifactFile[];
};

export type ArtifactCleanupRequest = {
  retention_hours: number;
  apply: boolean;
};

export type ArtifactCleanupFailure = {
  path?: string | null;
  artifact_ids: string[];
  reason: string;
};

export type ArtifactCleanupReport = {
  applied: boolean;
  cutoff_ms: number;
  candidate_manifest_ids: string[];
  candidate_orphan_paths: string[];
  deleted_manifest_ids: string[];
  deleted_paths: string[];
  retained_shared_paths: string[];
  failures: ArtifactCleanupFailure[];
};

export type BackupRestoreStatus = "running" | "success" | "failed";

export type BackupRestoreRecord = {
  id: string;
  backup_id: string;
  status: BackupRestoreStatus;
  actor: "api" | "cli" | "tui" | "sync" | "system";
  started_at_ms: number;
  finished_at_ms?: number | null;
  error?: string | null;
};

export type BackupRecord = {
  id: string;
  artifact: ArtifactManifest;
  operation_id?: string | null;
  provider_id?: string | null;
  provider_session_id?: string | null;
  session_id?: string | null;
  source_path?: string | null;
  created_at_ms: number;
  restore_hint?: string | null;
  metadata: Record<string, unknown>;
};

export type BackupView = {
  entry: {
    backup: BackupRecord;
    latest_restore?: BackupRestoreRecord | null;
  };
  verification: ArtifactVerification;
};

export type BackupQueryParams = {
  operation_id?: string;
  provider?: string;
  provider_session_id?: string;
  restore_status?: BackupRestoreStatus;
  limit?: number;
};

export type CompressionProjection = "native" | "portable" | string;

export type CompressionProviderSupport = {
  provider_id: string;
  detects_native_source: boolean;
  native_target_projection: boolean;
  native_session_replace: boolean;
  native_session_restore: boolean;
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

export type CompressionFormat =
  "json" | "md" | "html" | "morph" | "both" | string;

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
  source_bytes_before: number;
  source_bytes_after: number;
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

export type HomeSessionLayout = "stack" | "tabs";

export type SettingsPayload = {
  sessions_per_provider: number;
  skills_catalog_page_size: number;
  language: UiLanguage;
  show_opencode_subagents: boolean;
  sort_providers_by_session_count: boolean;
  default_backup_dir: string;
  logging: Record<string, unknown>;
  home_buttons: Record<string, unknown>;
  home_session_layout: HomeSessionLayout;
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
  skills_catalog_page_size: number;
  language: UiLanguage;
  show_opencode_subagents: boolean;
  sort_providers_by_session_count?: boolean;
  default_backup_dir: string;
  logging: LogSettingsPayload;
  home_buttons: HomeButtonSettingsPayload;
  home_session_layout: HomeSessionLayout;
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

export type ProviderContentFidelity = {
  text?: MappingDisposition | null;
  thinking?: MappingDisposition | null;
  tool_call?: MappingDisposition | null;
  tool_result?: MappingDisposition | null;
  patch?: MappingDisposition | null;
  image?: MappingDisposition | null;
  file?: MappingDisposition | null;
  compressed?: MappingDisposition | null;
  provider_payload?: MappingDisposition | null;
};

export type ProviderCapabilities = {
  scan: boolean;
  import: boolean;
  export: boolean;
  delete: boolean;
  rename: boolean;
  resume: boolean;
  scan_strategy: "unknown" | "full_scan" | "indexed" | "hybrid" | string;
  page_strategy:
    "unknown" | "full_import" | "indexed_page" | "native_page" | string;
  storage_shape:
    "unknown" | "jsonl" | "sqlite" | "directory" | "mixed" | string;
  turn_quality: "unknown" | "exact" | "inferred" | "grouped" | string;
  import_fidelity: ProviderContentFidelity;
  export_fidelity: ProviderContentFidelity;
  resume_quality: "none" | "native" | "imported" | "text_only" | string;
  write_risk: {
    level: "unknown" | "low" | "medium" | "high" | string;
    multiple_files: boolean;
    sqlite: boolean;
    sidecar_files: boolean;
    index_repair: boolean;
  };
  backup_support: {
    before_write: boolean;
    restore: boolean;
    sync_only: boolean;
  };
  activity_support: {
    hook_events: boolean;
    runtime_endpoint: boolean;
    session_activity: boolean;
  };
};

export type ProviderCatalogEntry = {
  provider_id: string;
  display_name: string;
  capability_set: ProviderCapabilities;
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

export type DirectoryEntry = {
  name: string;
  path: string;
};

export type DirectoryListing = {
  path: string;
  parent: string | null;
  directories: DirectoryEntry[];
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

export type EnsureReadyPayload = {
  selected_workspace: string | null;
  repaired: boolean;
};

export type ReadinessState = "ready" | "partial" | "degraded" | "error";
export type ReadinessPhaseName =
  | "foundation"
  | "agents"
  | "sessions"
  | "session_stats"
  | "skills"
  | "usage"
  | "derived";
export type ReadinessFocus = "overview" | Exclude<ReadinessPhaseName, "foundation">;
export type ReadinessPriority = "background" | "foreground";
export type ReadinessTrigger = "startup" | "manual" | "workspace_change" | "incomplete_panel" | "retry";
export type ReadinessPhase = {
  state: ReadinessState;
  message?: string | null;
};

export type ReadinessPhases = Record<ReadinessPhaseName, ReadinessPhase>;

export type ReadinessPayload = {
  workspace: string | null;
  state: ReadinessState;
  active_operation_id?: string | null;
  recommended_focus?: ReadinessFocus | null;
  phases: ReadinessPhases;
};

export type ReadinessReconcilePayload = {
  workspace?: string | null;
  focus: ReadinessFocus;
  priority: ReadinessPriority;
  trigger: ReadinessTrigger;
};

export type ReadinessReconcileResult = {
  operation_id?: string | null;
  disposition: "started" | "joined" | "noop";
  status: string;
  readiness: ReadinessPayload;
  status_url: string;
};

export type ReadinessOperation = {
  operation_id: string;
  status: "queued" | "running" | "completed" | "failed";
  readiness: ReadinessPayload;
  trigger: ReadinessTrigger;
  priority: ReadinessPriority;
  current_phase?: ReadinessPhaseName;
  completed_phases: ReadinessPhaseName[];
  running_phases: ReadinessPhaseName[];
  pending_phases: ReadinessPhaseName[];
  failures: Array<{ phase: ReadinessPhaseName; message: string }>;
};

export type ScanWorkspacesPayload = {
  scanned_providers: number;
  failed_providers: number;
  discovered_sessions: number;
  projected_sessions: number;
  unchanged_sessions: number;
  missing_sources: number;
  unsupported_providers: number;
  failed_sessions: number;
  failures: Array<{
    provider_id: string;
    session_id?: string | null;
    source_path?: string | null;
    reason: string;
  }>;
};

export type MetaPayload = {
  version: string;
  selected_workspace: string | null;
  workspaces: WorkspaceEntry[];
  capabilities: {
    system_folder_picker: boolean;
  };
  settings: SettingsPayload;
  settings_paths: SettingsPathsPayload;
  config_file: ConfigFilePayload;
};

export type SessionListSort = "recent" | "title";

export type SessionListFields = "minimal" | "with_stats";

export type SessionListParams = {
  all?: boolean;
  provider?: string;
  dir?: string;
  workspace?: string;
  /** Projection tier. "minimal" skips per-session stat counts; "with_stats"
   *  includes them. Defaults to "with_stats" on the backend when omitted. */
  fields?: SessionListFields;
  limit?: number;
  offset?: number;
  sort?: SessionListSort;
  /** When true, the workspace feed asks providers to rescan instead of
   *  returning cached data. Used for explicit refresh actions. */
  refresh?: boolean;
};

export type SessionItem = {
  session_id: string;
  title: string | null;
  native_title?: string | null;
  display_title?: string | null;
  hidden: boolean;
  pinned: boolean;
  stale: boolean;
  preferred_targets: string[];
  project_dir: string | null;
  last_active_at: number | null;
  source_path: string | null;
  provider_id: string;
  message_count: number | null;
  size_bytes: number | null;
};

export type SessionDetailParams = {
  event_offset?: number;
  event_limit?: number;
  event_search?: string;
  event_order?: SessionEventOrder;
};

export type SessionEventOrder = "asc" | "desc";

export type SessionDetailPayload = {
  view: SessionDetailView;
  events_offset: number;
  events_limit: number | null;
  returned_event_count: number;
  has_more_events: boolean;
  event_search?: string | null;
  event_order?: SessionEventOrder | null;
  matched_event_count?: number | null;
  returned_event_indices?: number[];
};

export type MappingDisposition =
  | "preserved"
  | "normalized"
  | "downgraded"
  | "dropped"
  | "unsupported"
  | string;

export type ProjectionFidelity =
  "preserved" | "normalized" | "dropped" | string;

export type SessionProjectionReport = {
  id: string;
  provider_id: string;
  source_id?: string | null;
  operation_kind: "scan" | "import" | "export" | "rebuild" | string;
  projection_version: number;
  status: "completed" | "completed_with_loss" | "failed" | string;
  created_at: string;
  created_at_ms: number;
  summary: {
    canonical_event_count?: number | null;
    mapping_direction?: "import" | "export" | string | null;
    mapping_overall?: MappingDisposition | null;
    preserved_count: number;
    normalized_count: number;
    dropped_count: number;
  };
  item_count: number;
  items: Array<{
    item_order: number;
    fidelity: ProjectionFidelity;
    scope: string;
    field_path?: string | null;
    reason?: string | null;
    details?: unknown;
  }>;
};

export type SessionTurn = {
  id: string;
  session_id: string;
  provider_turn_id?: string | null;
  status: "unknown" | "open" | "completed" | "failed" | "interrupted" | string;
  confidence: "exact" | "inferred" | "grouped" | "unknown" | string;
  started_at_ms?: number | null;
  ended_at_ms?: number | null;
  source_range: {
    start_cursor?: string | null;
    end_cursor?: string | null;
  };
  turn_order: number;
};

export type EventRole =
  "user" | "assistant" | "tool" | "system" | "developer" | "unknown" | string;

export type SessionEventKind =
  | "message"
  | "action"
  | "observation"
  | "lifecycle"
  | "other"
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
  | {
      type: "tool_result";
      tool_call_id: string;
      content: string;
      is_error?: boolean;
    }
  | {
      type: "patch";
      summary?: string | null;
      diff_text?: string | null;
      files?: string[];
      hash?: string | null;
    }
  | { type: "command"; command: string; argv?: string[]; cwd?: string | null }
  | {
      type: "command_result";
      command?: string | null;
      exit_code?: number | null;
      stdout?: string | null;
      stderr?: string | null;
    }
  | {
      type: "file";
      path: string;
      content?: string | null;
      mime_type?: string | null;
    }
  | {
      type: "image";
      mime_type: string;
      data?: string | null;
      path?: string | null;
    }
  | { type: "compressed"; raw: CompressedBlockRaw }
  | { type: "other"; raw: unknown };

export type CompressedBlockRaw = {
  format?: string;
  source_provider_id?: string;
  summary?: string;
  source_event_ids?: string[];
  source_event_count?: number | null;
  archive_ref?: string | null;
};

export type SessionEvent = {
  id: string;
  kind: SessionEventKind;
  role: EventRole;
  timestamp: string;
  links?: EventLinks;
  blocks: EventBlock[];
  tags?: string[];
  extensions?: Record<string, unknown>;
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
  length_metrics: {
    provider_source_bytes_measured: number;
    model_visible_bytes_measured: number;
    estimated_tokens: number;
    event_count: number;
    message_count: number;
    turn_count: number;
    compressed_segment_count: number;
    archive_count: number;
  };
  stale: boolean;
  projection_report?: SessionProjectionReport | null;
  turns: SessionTurn[];
  events: SessionEvent[];
  compressed_archive_refs: string[];
};

export type SessionStalenessRefreshReport = {
  checked_sources: number;
  fresh_snapshots: number;
  stale_snapshots: number;
  missing_sources: number;
  unknown_sources: number;
};

export type SessionReprojectionReport = {
  candidate_snapshots: number;
  reprojected_snapshots: number;
  missing_sources: number;
  unsupported_providers: number;
  failed_snapshots: number;
  failures: Array<{
    provider_id: string;
    provider_session_id?: string | null;
    source_path?: string | null;
    reason: string;
  }>;
};

export type SessionActivityBucketUnit =
  "minute" | "hour" | "twelve_hour" | "adaptive";

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
  projected_sessions: number;
  sessions_with_activity: number;
};

export type SessionGroup = {
  provider_id: string;
  provider_name: string;
  sessions: SessionItem[];
};

export type FeedStateKind =
  | "fresh"
  | "warming"
  | "cold_scanning"
  | "complete"
  | "error";

export type FeedState = {
  kind: FeedStateKind;
  message?: string | null;
};

export type ProviderFeedStateKind =
  | "scanning"
  | "scanned"
  | "empty"
  | "error";

export type ProviderFeedState = {
  provider_id: string;
  kind: ProviderFeedStateKind;
  message?: string | null;
  discovered_count: number;
};

/** Lightweight workspace feed revision poll. The home view refetches the full
 *  session list only when `revision` moves; `busy` lets it poll faster while a
 *  scan is in flight and relax once the workspace settles. */
export type WorkspaceFeedRevision = {
  workspace: string;
  revision: number;
  updated_at: number;
  busy: boolean;
};

export type SessionPage = {
  groups: SessionGroup[];
  offset: number;
  limit: number;
  has_more: boolean;
  /** True when the store had no indexed sessions for the requested workspace
   *  and a background indexing pass was triggered. Clients should re-poll. */
  degraded?: boolean;
  /** Workspace feed state. Present when the request scoped to a workspace. */
  feed_state?: FeedState;
  /** Per-provider feed state from the workspace scan. */
  provider_states?: ProviderFeedState[];
  /** Unix epoch milliseconds when the feed completed a refresh. */
  refreshed_at?: number | null;
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

export type DetectedHook = {
  event: string;
  index: number;
  fingerprint: string;
  matcher?: string;
  hook_type?: string;
  command?: string;
  source: "memorph" | "third_party";
  managed_by_memorph: boolean;
};

export type DetectedHooks = {
  provider: string;
  scan_supported: boolean;
  config_path?: string | null;
  hooks: DetectedHook[];
};

export type HookOperationReport = {
  changed?: boolean;
  message?: string | null;
  status?: Record<string, unknown> | null;
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

export type ProviderConfigTone = "ok" | "warning" | "danger" | "muted";

export type ProviderConfigSource = {
  path: string;
  scope: string;
  exists: boolean;
};

export type ProviderConfigRow = {
  label: string;
  value: unknown;
  hint?: string;
  tone?: ProviderConfigTone;
};

export type ProviderConfigSection = {
  label: string;
  rows: ProviderConfigRow[];
  entry?: ProviderConfigEntry;
};

export type ProviderConfigIssue = {
  message: string;
  tone: ProviderConfigTone;
};

export type ProviderConfigView = {
  provider_id: string;
  view_id: string;
  providerId?: string;
  viewId?: string;
  title: string;
  sources?: ProviderConfigSource[];
  sections?: ProviderConfigSection[];
  issues?: ProviderConfigIssue[];
  entries?: ProviderConfigEntry[];
};

/** Metadata for a configuration entry that the backend explicitly permits removing. */
export type ProviderConfigEntry = {
  entry_id?: string;
  entryId?: string;
  id?: string;
  name?: string;
  label?: string;
  title?: string;
  scope?: string;
  source?: string;
  source_path?: string;
  sourcePath?: string;
  fingerprint?: string;
  expected_fingerprint?: string;
  expectedFingerprint?: string;
  removable?: boolean;
  can_remove?: boolean;
  canRemove?: boolean;
};

export type ProviderConfigEntryRemovalResult = {
  status?: string;
  outcome?: string;
  result?: string;
  message?: string | null;
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

export type AgentSessionManagementCapability = {
  scan: boolean;
  import: boolean;
  export: boolean;
  delete: boolean;
  rename: boolean;
  resume: boolean;
};

export type AgentHookManagementCapability = {
  install: boolean;
  verify: boolean;
  repair: boolean;
  uninstall: boolean;
  discovery: boolean;
  status: string;
};

export type AgentMcpManagementCapability = {
  list: boolean;
  inspect: boolean;
  remove?: boolean;
};

export type AgentPluginManagementCapability = {
  list: boolean;
  inspect: boolean;
};

export type AgentConfigViewCapability = {
  id: string;
  title: string;
  description: string;
};

export type AgentCapabilityManifest = {
  provider_id: string;
  session_management: AgentSessionManagementCapability;
  hook_management?: AgentHookManagementCapability | null;
  mcp_management?: AgentMcpManagementCapability | null;
  plugin_management?: AgentPluginManagementCapability | null;
  config_views: AgentConfigViewCapability[];
};

export type AgentManagementEntry = {
  provider_id: string;
  name: string;
  environment: AgentEnvironmentStatus;
  capabilities: AgentCapabilityManifest;
  settings: ProviderSettingItem[];
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

export type WorkflowStatus =
  "planned" | "in-progress" | "implemented" | "verified";

export type StatsDashboardRange = "7d" | "30d" | "90d" | "all";

export type StatsBucket = { count: number; size_bytes: number };

export type StatsBreakdownItem = {
  id: string;
  session_count: number;
  active_session_count: number;
  message_count: number;
  size_bytes: number;
  last_active_at: string | null;
};

export type StatsSessionItem = {
  provider_id: string;
  session_id: string;
  title: string;
  workspace: string | null;
  message_count: number | null;
  size_bytes: number;
  created_at: string | null;
  last_active_at: string | null;
};

export type StatsDashboard = {
  generated_at: string;
  range_start: string | null;
  completeness: {
    status: "complete" | "partial";
    incomplete_session_count: number;
  };
  overview: {
    total_sessions: number;
    active_sessions: number;
    new_sessions: number;
    total_messages: number;
    active_session_messages: number;
    total_size_bytes: number;
    stale_size_bytes: number;
    total_workspaces: number;
    active_workspaces: number;
    total_providers: number;
    active_providers: number;
    unknown_message_counts: number;
    unknown_message_timestamps: number;
    unknown_size_bytes: number;
    unknown_activity_times: number;
    unknown_created_times: number;
  };
  attention: {
    active_7d: StatsBucket;
    inactive_7_to_30d: StatsBucket;
    inactive_30_to_90d: StatsBucket;
    inactive_over_90d: StatsBucket;
    unknown: StatsBucket;
    large_sessions: StatsBucket;
    short_sessions: StatsBucket;
    large_threshold_bytes: number;
    short_max_messages: number;
  };
  timeline: Array<{
    start: string;
    active_sessions: number;
    new_sessions: number;
    active_session_messages: number;
    new_size_bytes: number;
  }>;
  providers: StatsBreakdownItem[];
  workspaces: StatsBreakdownItem[];
  top_sessions: {
    by_messages: StatsSessionItem[];
    by_size: StatsSessionItem[];
    recently_active: StatsSessionItem[];
  };
  distributions: {
    session_size: Array<{
      key: string;
      label: string;
      count: number;
      size_bytes: number;
    }>;
    message_count: Array<{
      key: string;
      label: string;
      count: number;
      size_bytes: number;
    }>;
  };
};

export type SkillAgent = {
  agent_id: string;
  name: string;
  skills_dir: string;
};

export type SkillInstallation = {
  used_by: string;
  path: string;
  managed: boolean;
  deployment_mode: "symlink" | "copy" | "external";
  link_valid: boolean;
  fingerprint: string;
  drifted: boolean;
};

export type SkillStatistics = {
  files: number;
  bytes: number;
  scripts: number;
  references: number;
  assets: number;
  previewable: number;
};

export type SkillIssue = {
  path?: string | null;
  message: string;
};

export type SkillAsset = {
  path: string;
  category: "entry" | "script" | "reference" | "asset" | "metadata" | "other";
  extension?: string | null;
  bytes: number;
  previewable: boolean;
  entry: boolean;
};

export type SkillEntry = {
  id: string;
  name: string;
  description?: string | null;
  directory: string;
  fingerprint: string;
  conflict: boolean;
  statistics: SkillStatistics;
  issues: SkillIssue[];
  installations: SkillInstallation[];
};

export type SkillDetail = SkillEntry & {
  frontmatter: Record<string, string>;
  provider_metadata: SkillAsset[];
  tags: string[];
  used_by: string[];
};

export type SkillTree = {
  skill_id: string;
  fingerprint: string;
  assets: SkillAsset[];
  issues: SkillIssue[];
};

export type SkillFilePreview = {
  path: string;
  category: string;
  extension?: string | null;
  bytes: number;
  encoding: "text" | "base64";
  mime_type?: string | null;
  content: string;
};

export type SkillsOverview = {
  agents: SkillAgent[];
  skills: SkillEntry[];
};

export type SkillMutation = {
  skill_id: string;
  used_by: string;
  source_used_by?: string;
};

export type SkillScanQueued = {
  queued: boolean;
  mode: "incremental" | "full";
  roots_scanned: number;
  skills_seen: number;
};

/** @deprecated use SkillScanQueued — kept for alias-tolerance during migration. */
export type SkillScanSummary = SkillScanQueued;

export type SkillCatalogParams = {
  query?: string;
  used_by?: string;
  scope?: "global" | "project";
  sort?: "name" | "size" | "files" | "updated";
  order?: "asc" | "desc";
  page?: number;
  pageSize?: number;
};

export type SkillCatalogInstallation = {
  used_by: string;
  scope_kind: "global" | "project";
  workspace_dir?: string | null;
  install_path: string;
  install_kind: "directory" | "symlink" | "managed-copy";
  symlink_target?: string | null;
  link_status:
    "not-applicable" | "valid" | "broken" | "outside-allowed-root" | "loop";
  status: "active" | "missing" | "removed" | "error";
};

export type SkillCatalogItem = {
  id: string;
  source_id: string;
  name: string;
  description?: string | null;
  version?: string | null;
  author?: string | null;
  bundle_hash: string;
  file_count: number;
  total_bytes: number;
  missing: boolean;
  updated_at_ms: number;
  installations: SkillCatalogInstallation[];
  tags: string[];
  used_by: string[];
};

export type SkillCatalogPage = {
  items: SkillCatalogItem[];
  page: number;
  page_size: number;
  total: number;
  used_by: string[];
  completeness: {
    status: "unknown" | "partial" | "complete" | "error";
    updated_at_ms?: number | null;
  };
  /** True when the catalog looks unpopulated (empty + unknown completeness). */
  needs_scan?: boolean;
};

export type SkillStatsParams = {
  from?: string;
  to?: string;
  provider?: string;
  workspace?: string;
  confidence?: "high" | "medium" | "low";
  skillId?: string;
  page?: number;
  pageSize?: number;
};

export type SkillStatsSummary = {
  invocations: number;
  active_skills: number;
  active_sessions: number;
  active_days: number;
  token_count?: number | null;
  last_invoked_at_ms?: number | null;
  completeness_status: "unknown" | "partial" | "complete" | "error";
};

export type SkillDailyUsage = {
  date: string;
  invocations: number;
  sessions: number;
  token_count?: number | null;
};

export type SkillRanking = {
  skill_id: string;
  name: string;
  invocations: number;
  sessions: number;
  token_count?: number | null;
  last_invoked_at_ms?: number | null;
  trend: number[];
};

export type SkillStatsBreakdownItem = {
  key: string;
  invocations: number;
  sessions: number;
};

export type SkillStatsBreakdown = {
  providers: SkillStatsBreakdownItem[];
  workspaces: SkillStatsBreakdownItem[];
};

export type SkillInvocation = {
  id: string;
  session_id: string;
  event_id?: string | null;
  provider_id: string;
  workspace_dir?: string | null;
  invoked_at_ms: number;
  detection_kind:
    | "explicit-tool"
    | "entry-path"
    | "bundle-path"
    | "explicit-name"
    | "content-evidence";
  confidence: "high" | "medium" | "low";
  evidence_text?: string | null;
  evidence_path?: string | null;
  token_count?: number | null;
};

export type SkillInvocationPage = {
  items: SkillInvocation[];
  page: number;
  page_size: number;
  total: number;
};

export type SkillContextLayer = {
  bytes: number;
  characters: number;
  token_lower: number;
  estimated_tokens: number;
  token_upper: number;
  estimated: true;
  algorithm_version: string;
  baseline_percent?: number | null;
};

export type SkillContext = {
  skill_id: string;
  name: string;
  metadata: SkillContextLayer;
  body: SkillContextLayer;
  auxiliary: SkillContextLayer;
  observed_token_min?: number | null;
  observed_token_max?: number | null;
};

export type SkillContextSummary = {
  baseline_tokens?: number | null;
  algorithm_version: string;
  skills: SkillContext[];
};

export type SkillHealthCheck = {
  check_id: string;
  category: string;
  severity: "error" | "warning" | "info" | "pass";
  title: string;
  description: string;
  evidence: string;
  recommendation: string;
  checked_at_ms: number;
};

export type SkillHealth = {
  skill_id: string;
  status: "error" | "warning" | "pass";
  score: number;
  checks: SkillHealthCheck[];
};

export type SkillHealthSummary = {
  total: number;
  errors: number;
  warnings: number;
  healthy: number;
  completeness_status: "unknown" | "partial" | "complete" | "error";
  skills: SkillHealth[];
};
export type SkillConflict = {
  id: string;
  left_skill_id: string;
  left_name: string;
  right_skill_id: string;
  right_name: string;
  conflict_kind: string;
  severity: "error" | "warning" | "info";
  similarity: number;
  overlapping_tokens: string[];
  evidence: string;
  recommendation: string;
};

export type SkillCoverageTarget = {
  target_kind: string;
  target_key: string;
  target_path?: string | null;
  section_title?: string | null;
  confidence?: "high" | "medium" | "low" | null;
  observations: number;
  first_observed_at_ms?: number | null;
  last_observed_at_ms?: number | null;
};

export type SkillCoverage = {
  skill_id: string;
  covered: number;
  total: number;
  percent: number;
  completeness_status: string;
  targets: SkillCoverageTarget[];
};

export type SkillCoverageSummaryItem = {
  skill_id: string;
  name: string;
  covered: number;
  total: number;
  percent: number;
};

export type SkillPruneItem = {
  installation_id: string;
  skill_id: string;
  name: string;
  install_path: string;
  install_kind: string;
  unused_since_ms: number;
  last_invoked_at_ms?: number | null;
  installation_bytes: number;
  metadata_tokens: number;
  low_confidence_observations: number;
  action: string;
  executable: boolean;
  blocked_reason?: string | null;
  expected_fingerprint: string;
};
export type SkillPrunePreview = {
  preview_id: string;
  days: number;
  completeness_status: string;
  blocked_reason?: string | null;
  items: SkillPruneItem[];
};
export type SkillPruneResult = {
  installation_id: string;
  status: string;
  message: string;
};

export type SkillGraphDay = {
  date: string;
  invocations: number;
  sessions: number;
  active_skills: number;
  level: number;
};
export type SkillGraph = {
  from: string;
  to: string;
  timezone: string;
  max_count: number;
  total_invocations: number;
  days: SkillGraphDay[];
};
export type SkillGraphParams = {
  from?: string;
  to?: string;
  skillId?: string;
  provider?: string;
  workspace?: string;
  timezone?: string;
};

export type SkillCoverageEvidence = {
  invocation_id: string;
  session_id: string;
  provider_id: string;
  event_id?: string | null;
  observed_at_ms: number;
  match_kind: string;
  confidence: string;
  evidence_text?: string | null;
};
export type SkillCoverageEvidencePage = {
  items: SkillCoverageEvidence[];
  page: number;
  page_size: number;
  total: number;
};
