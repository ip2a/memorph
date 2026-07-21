use anyhow::{anyhow, Context};
use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::{
    agent_management, cache, config, core, hooks, logging, provider_settings,
    providers::catalog::{build_catalog, sort_catalog, CatalogInput, ProviderCatalog},
    storage::activity_store::{
        ActivityActor, ActivityOperationKind, ActivityQuery, ActivityStatus,
    },
    sync as session_sync,
};

mod compression;
mod management;
mod providers;
mod router;
mod sessions;
pub use router::router;

type FolderPicker =
    dyn Fn(Option<String>) -> anyhow::Result<Option<String>> + Send + Sync + 'static;
type FilePicker = dyn Fn(Option<String>) -> anyhow::Result<Option<String>> + Send + Sync + 'static;

static FOLDER_PICKER: OnceLock<Arc<FolderPicker>> = OnceLock::new();
static FILE_PICKER: OnceLock<Arc<FilePicker>> = OnceLock::new();

#[derive(Serialize)]
struct ApiResponse<T: Serialize> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    fn success(data: T) -> Json<Self> {
        Json(Self {
            ok: true,
            data: Some(data),
            error: None,
        })
    }
}

fn api_error(status: StatusCode, msg: impl ToString) -> impl IntoResponse {
    let message = msg.to_string();
    logging::error("api_error", &message);
    (
        status,
        Json(ApiResponse::<()> {
            ok: false,
            data: None,
            error: Some(message),
        }),
    )
}

pub fn register_folder_picker<F>(picker: F) -> bool
where
    F: Fn(Option<String>) -> anyhow::Result<Option<String>> + Send + Sync + 'static,
{
    FOLDER_PICKER.set(Arc::new(picker)).is_ok()
}

pub fn register_file_picker<F>(picker: F) -> bool
where
    F: Fn(Option<String>) -> anyhow::Result<Option<String>> + Send + Sync + 'static,
{
    FILE_PICKER.set(Arc::new(picker)).is_ok()
}

#[derive(Debug, Serialize)]
struct ProviderInfo {
    id: String,
    name: String,
    scan: bool,
    import: bool,
    export: bool,
    delete: bool,
    rename: bool,
    resume: bool,
}

#[derive(Debug, Serialize)]
struct ProviderSettingsPayload {
    provider_id: String,
    settings: Vec<provider_settings::ProviderSettingItem>,
}

#[derive(Debug, Serialize)]
struct AgentManagementPayload {
    providers: Vec<agent_management::AgentManagementEntry>,
}

#[derive(Debug, Serialize)]
struct AgentManagementSummaryPayload {
    providers: Vec<agent_management::AgentManagementSummaryEntry>,
}

#[derive(Debug, Serialize)]
struct SettingsPayload {
    sessions_per_provider: usize,
    language: config::UiLanguage,
    show_opencode_subagents: bool,
    sort_providers_by_session_count: bool,
    #[serde(default)]
    default_backup_dir: String,
    logging: config::LogPreferences,
    home_buttons: config::HomeButtonConfig,
    agent_order: Vec<String>,
    primary_agents: Vec<String>,
    server: config::ServerPreferences,
}

#[derive(Debug, Serialize)]
struct ConfigFilePayload {
    path: String,
    format: &'static str,
    content: String,
}

#[derive(Debug, Serialize)]
struct SettingsPathsPayload {
    backup_dir_input: String,
    backup_dir_resolved: String,
    backup_dir_base: String,
    log_dir: String,
    log_file_name: &'static str,
    log_file_path: String,
}

#[derive(Debug, Serialize)]
struct MetaPayload {
    version: &'static str,
    selected_workspace: Option<String>,
    workspaces: Vec<config::WorkspaceEntry>,
    capabilities: CapabilitiesPayload,
    settings: SettingsPayload,
    settings_paths: SettingsPathsPayload,
    config_file: ConfigFilePayload,
}

#[derive(Debug, Serialize)]
struct CapabilitiesPayload {
    system_folder_picker: bool,
}

#[derive(Debug, Serialize)]
struct SessionDetailPayload {
    view: core::SessionDetailView,
    events_offset: usize,
    events_limit: Option<usize>,
    returned_event_count: usize,
    has_more_events: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hook_runtime_sessions: Vec<hooks::model::RuntimeSession>,
}

#[derive(Debug, Serialize)]
struct SessionStalenessRefreshPayload {
    checked_sources: usize,
    fresh_snapshots: usize,
    stale_snapshots: usize,
    missing_sources: usize,
    unknown_sources: usize,
}

#[derive(Debug, Deserialize)]
struct SessionReprojectStaleRequest {
    provider: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionProjectionBootstrapRequest {
    provider: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionReprojectionPayload {
    candidate_snapshots: usize,
    reprojected_snapshots: usize,
    missing_sources: usize,
    unsupported_providers: usize,
    failed_snapshots: usize,
    failures: Vec<core::SessionReprojectionFailure>,
}

fn fallback_backup_dir_base() -> std::path::PathBuf {
    std::env::current_dir()
        .or_else(|_| {
            dirs::home_dir().ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

fn backup_dir_base_path(workspace: Option<&str>) -> std::path::PathBuf {
    workspace
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            config::selected_workspace()
                .ok()
                .flatten()
                .map(std::path::PathBuf::from)
        })
        .unwrap_or_else(fallback_backup_dir_base)
}

fn resolve_backup_output_dir(output_dir: &str, workspace: Option<&str>) -> std::path::PathBuf {
    let output_path = std::path::PathBuf::from(output_dir);
    if output_path.is_absolute() {
        return output_path;
    }

    backup_dir_base_path(workspace).join(output_path)
}

fn display_settings_path(path: &std::path::Path) -> String {
    let visible = crate::utils::user_visible_path(&path.to_string_lossy());
    let Ok(home_dir) = config::effective_home_dir() else {
        return visible;
    };
    let home_visible = crate::utils::user_visible_path(&home_dir.to_string_lossy());
    if visible == home_visible {
        return "~".to_string();
    }
    if let Some(stripped) = visible.strip_prefix(&(home_visible.clone() + "/")) {
        return format!("~/{}", stripped);
    }
    if let Some(stripped) = visible.strip_prefix(&(home_visible + "\\")) {
        return format!("~/{}", stripped.replace('\\', "/"));
    }
    visible
}

#[derive(Debug, Serialize)]
struct SyncHoldingPayload {
    id: String,
    provider: String,
    session_id: String,
    target_dir: Option<String>,
    created_at: i64,
    last_active_at: Option<i64>,
    last_sync_at: Option<i64>,
    last_sync_from: Option<String>,
    last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_runtime_summary: Option<hooks::augmentation::HookRuntimeSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hook_diagnosis: Option<hooks::augmentation::SessionHookDiagnosis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hook_runtime_sessions: Vec<hooks::model::RuntimeSession>,
}

#[derive(Debug, Serialize)]
struct SyncGroupPayload {
    id: String,
    title: String,
    #[serde(default)]
    source_provider: Option<String>,
    created_at: i64,
    updated_at: i64,
    holdings: Vec<SyncHoldingPayload>,
}

#[derive(Deserialize)]
struct SettingsBody {
    sessions_per_provider: usize,
    language: config::UiLanguage,
    show_opencode_subagents: bool,
    #[serde(default)]
    sort_providers_by_session_count: Option<bool>,
    default_backup_dir: String,
    #[serde(default)]
    logging: config::LogPreferences,
    home_buttons: config::HomeButtonConfig,
    #[serde(default)]
    agent_order: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    primary_agents: Vec<String>,
    server: Option<config::ServerPreferences>,
}

#[derive(Deserialize)]
struct SelectFolderBody {
    start_path: Option<String>,
}

#[derive(Serialize)]
struct SelectFolderPayload {
    path: Option<String>,
}

#[derive(Deserialize)]
struct DirectoryQuery {
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct DirectoryEntryPayload {
    name: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct DirectoryListingPayload {
    path: String,
    parent: Option<String>,
    directories: Vec<DirectoryEntryPayload>,
}

#[derive(Deserialize)]
struct SelectFileBody {
    start_path: Option<String>,
}

#[derive(Serialize)]
struct SelectFilePayload {
    path: Option<String>,
}

#[derive(Deserialize)]
struct OpenExternalBody {
    url: String,
}

#[derive(Serialize)]
struct OpenExternalPayload {
    opened: bool,
}

#[derive(Debug, Serialize)]
struct UpdateCheckPayload {
    current_version: &'static str,
    latest_version: String,
    install_source: String,
    install_source_label: String,
    has_update: bool,
    update_command: String,
    release_url: String,
}

#[derive(Debug, Deserialize)]
struct NpmLatestPayload {
    version: String,
}

#[derive(Debug, Deserialize)]
struct PypiProjectPayload {
    info: PypiProjectInfo,
}

#[derive(Debug, Deserialize)]
struct PypiProjectInfo {
    version: String,
}

#[derive(Debug, Deserialize)]
struct GitHubReleasePayload {
    tag_name: String,
}

fn provider_info_list() -> Vec<ProviderInfo> {
    crate::providers::all_provider_ids()
        .iter()
        .filter_map(|id| crate::providers::find_provider(id))
        .map(|provider| {
            let capabilities = provider.capabilities();
            ProviderInfo {
                id: provider.id().to_string(),
                name: provider.name().to_string(),
                scan: capabilities.scan,
                import: capabilities.import,
                export: capabilities.export,
                delete: capabilities.delete,
                rename: capabilities.rename,
                resume: capabilities.resume,
            }
        })
        .collect()
}

async fn build_provider_catalog_light(workspace: Option<&str>) -> anyhow::Result<ProviderCatalog> {
    if let Some(catalog) = cache::catalog_cache().get(workspace) {
        return Ok(catalog);
    }

    let prefs = config::web_preferences()?;
    let ordered_ids = config::ordered_provider_ids(&prefs);
    let hidden_global = config::global_hidden_provider_ids(&prefs);
    let hidden_workspace = config::workspace_hidden_provider_ids(workspace);
    let workspace_order = config::workspace_ordered_provider_ids(workspace);

    // Detect environment concurrently.
    let mut env_handles = Vec::new();
    for id in &ordered_ids {
        let id = id.clone();
        env_handles.push(tokio::task::spawn_blocking(move || {
            (
                id.clone(),
                crate::agent_environment::detect_provider_environment_fast(&id),
            )
        }));
    }
    let mut env_by_provider: std::collections::HashMap<
        String,
        crate::agent_environment::AgentEnvironmentStatus,
    > = std::collections::HashMap::new();
    for handle in env_handles {
        if let Ok((id, env)) = handle.await {
            env_by_provider.insert(id, env);
        }
    }

    let mut catalog = build_catalog(CatalogInput {
        ordered_ids: &ordered_ids,
        hidden_global: &hidden_global,
        hidden_workspace: &hidden_workspace,
        has_sessions: &|_| false,
        environment: &|id| {
            env_by_provider.get(id).cloned().unwrap_or_else(|| {
                crate::agent_environment::AgentEnvironmentStatus {
                    installed: false,
                    executable_path: None,
                    executable_dir: None,
                    config_path: String::new(),
                    install_method: String::new(),
                    executable_version: None,
                }
            })
        },
        active_time: &|_| (0, 0),
    });

    // Apply explicit sort_order indices.
    let global_index: std::collections::HashMap<&str, usize> = prefs
        .agent_display
        .sort_order
        .global
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    let workspace_index: std::collections::HashMap<&str, usize> = workspace_order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();

    for provider in &mut catalog.providers {
        provider.sort_order.global = global_index
            .get(provider.provider_id.as_str())
            .map(|index| *index as i64)
            .unwrap_or(-1);
        provider.sort_order.workspace = workspace_index
            .get(provider.provider_id.as_str())
            .map(|index| *index as i64)
            .unwrap_or(-1);
    }

    sort_catalog(&mut catalog);

    cache::catalog_cache().set(workspace, catalog.clone());

    Ok(catalog)
}

async fn build_provider_catalog_active(
    workspace: Option<&str>,
) -> anyhow::Result<cache::ProviderActiveCatalog> {
    if let Some(catalog) = cache::active_catalog_cache().get(workspace) {
        return Ok(catalog);
    }

    let ordered_ids = config::ordered_provider_ids(&config::web_preferences()?);
    let workspace_opt = workspace.filter(|value| !value.is_empty());
    let snapshots = tokio::task::spawn_blocking(|| {
        let conn = crate::storage::local_store::open_database()?;
        crate::storage::snapshot_store::SnapshotStore::new(&conn).list_session_snapshots()
    })
    .await
    .context("Provider activity projection task failed")??;
    let result = provider_active_catalog_from_snapshots(&ordered_ids, workspace_opt, &snapshots);

    cache::active_catalog_cache().set(workspace, result.clone());

    Ok(result)
}

fn provider_active_catalog_from_snapshots(
    ordered_ids: &[String],
    workspace: Option<&str>,
    snapshots: &[crate::storage::snapshot_store::ProjectedSessionSnapshotRow],
) -> cache::ProviderActiveCatalog {
    let providers = ordered_ids
        .iter()
        .map(|id| {
            let provider = crate::providers::find_provider(id);
            let sessions: Vec<_> = snapshots
                .iter()
                .filter(|session| session.provider_id == *id)
                .collect();
            let workspace_sessions: Vec<_> = sessions
                .iter()
                .copied()
                .filter(|session| {
                    workspace.map_or(true, |workspace| {
                        provider.as_ref().is_some_and(|provider| {
                            provider.workspace_matches(
                                session.workspace_dir.as_deref(),
                                Some(workspace),
                            )
                        })
                    })
                })
                .collect();
            cache::ProviderActiveInfo {
                provider_id: id.clone(),
                has_sessions: !workspace_sessions.is_empty(),
                active_time: crate::providers::catalog::ActiveTime {
                    global: sessions
                        .iter()
                        .filter_map(|session| session.last_active_at_ms)
                        .max()
                        .unwrap_or(0),
                    workspace: workspace_sessions
                        .iter()
                        .filter_map(|session| session.last_active_at_ms)
                        .max()
                        .unwrap_or(0),
                },
            }
        })
        .collect();
    cache::ProviderActiveCatalog { providers }
}

fn invalidate_catalog_cache() {
    cache::invalidate_catalog_caches();
}

fn invalidate_compression_archives_cache() {
    cache::compression_archives_cache().invalidate_all();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallSource {
    Npm,
    PythonPip,
    PythonPipx,
    PythonUvTool,
    DesktopApp,
}

impl InstallSource {
    fn slug(self) -> &'static str {
        match self {
            InstallSource::Npm => "npm",
            InstallSource::PythonPip => "pip",
            InstallSource::PythonPipx => "pipx",
            InstallSource::PythonUvTool => "uv-tool",
            InstallSource::DesktopApp => "desktop-app",
        }
    }

    fn label(self) -> &'static str {
        match self {
            InstallSource::Npm => "npm",
            InstallSource::PythonPip => "PyPI/pip",
            InstallSource::PythonPipx => "PyPI/pipx",
            InstallSource::PythonUvTool => "PyPI/uv tool",
            InstallSource::DesktopApp => "GitHub Desktop App",
        }
    }

    fn release_url(self) -> &'static str {
        match self {
            InstallSource::Npm => "https://www.npmjs.com/package/memorph",
            InstallSource::PythonPip | InstallSource::PythonPipx | InstallSource::PythonUvTool => {
                "https://pypi.org/project/memorph/"
            }
            InstallSource::DesktopApp => "https://github.com/ip2a/memorph/releases/latest",
        }
    }

    fn update_command(self, python_executable: Option<String>) -> String {
        match self {
            InstallSource::Npm => "npm install -g memorph@latest".to_string(),
            InstallSource::PythonPip => format!(
                "{} -m pip install --upgrade memorph",
                python_executable.unwrap_or_else(|| "python".to_string())
            ),
            InstallSource::PythonPipx => "pipx upgrade memorph".to_string(),
            InstallSource::PythonUvTool => "uv tool upgrade memorph".to_string(),
            InstallSource::DesktopApp => {
                "Open the latest GitHub release and download the updated DMG.".to_string()
            }
        }
    }
}

fn normalize_str_path(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

fn normalize_path(value: &std::path::Path) -> String {
    normalize_str_path(&value.to_string_lossy())
}

fn looks_like_uv_tool(value: Option<&str>) -> bool {
    value
        .map(|value| normalize_str_path(value).contains("/uv/tools/"))
        .unwrap_or(false)
}

fn looks_like_pipx(value: Option<&str>) -> bool {
    value
        .map(|value| normalize_str_path(value).contains("/pipx/venvs/"))
        .unwrap_or(false)
}

fn detect_install_source(
    source_env: Option<&str>,
    exe_path: Option<&std::path::Path>,
    python_prefix: Option<&str>,
    python_executable: Option<&str>,
) -> Option<InstallSource> {
    if let Some(source) = source_env {
        match source.to_ascii_lowercase().as_str() {
            "npm" => return Some(InstallSource::Npm),
            "python" | "pypi" | "pip" => {
                if looks_like_uv_tool(python_prefix) || looks_like_uv_tool(python_executable) {
                    return Some(InstallSource::PythonUvTool);
                }
                if looks_like_pipx(python_prefix) || looks_like_pipx(python_executable) {
                    return Some(InstallSource::PythonPipx);
                }
                return Some(InstallSource::PythonPip);
            }
            "pipx" => return Some(InstallSource::PythonPipx),
            "uv" | "uv-tool" | "uv_tool" => return Some(InstallSource::PythonUvTool),
            "desktop" | "desktop-app" | "dmg" | "tauri" => return Some(InstallSource::DesktopApp),
            _ => {}
        }
    }

    let path = exe_path.map(normalize_path)?;
    if path.contains(".app/contents/macos/") {
        return Some(InstallSource::DesktopApp);
    }
    if path.contains("/node_modules/") && path.contains("memorph-bin") {
        return Some(InstallSource::Npm);
    }
    if path.contains("/site-packages/") && path.contains("memorph_bin") {
        return Some(InstallSource::PythonPip);
    }
    None
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    fn parse(value: &str) -> Vec<u64> {
        value
            .trim()
            .trim_start_matches('v')
            .split('+')
            .next()
            .unwrap_or(value)
            .split(['.', '-'])
            .filter_map(|part| {
                let digits: String = part.chars().take_while(|ch| ch.is_ascii_digit()).collect();
                if digits.is_empty() {
                    None
                } else {
                    digits.parse::<u64>().ok()
                }
            })
            .collect()
    }

    let left_parts = parse(left);
    let right_parts = parse(right);
    let max_len = left_parts.len().max(right_parts.len());
    for index in 0..max_len {
        let left_part = *left_parts.get(index).unwrap_or(&0);
        let right_part = *right_parts.get(index).unwrap_or(&0);
        match left_part.cmp(&right_part) {
            Ordering::Equal => continue,
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

async fn fetch_latest_version(source: InstallSource) -> anyhow::Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(format!("memorph/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    match source {
        InstallSource::Npm => {
            let payload = client
                .get("https://registry.npmjs.org/memorph/latest")
                .send()
                .await?
                .error_for_status()?
                .json::<NpmLatestPayload>()
                .await?;
            Ok(payload.version)
        }
        InstallSource::PythonPip | InstallSource::PythonPipx | InstallSource::PythonUvTool => {
            let payload = client
                .get("https://pypi.org/pypi/memorph/json")
                .send()
                .await?
                .error_for_status()?
                .json::<PypiProjectPayload>()
                .await?;
            Ok(payload.info.version)
        }
        InstallSource::DesktopApp => {
            let payload = client
                .get("https://api.github.com/repos/ip2a/memorph/releases/latest")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await?
                .error_for_status()?
                .json::<GitHubReleasePayload>()
                .await?;
            Ok(payload.tag_name)
        }
    }
}

async fn update_check_payload() -> anyhow::Result<UpdateCheckPayload> {
    let source = detect_install_source(
        std::env::var("MEMORPH_INSTALL_SOURCE").ok().as_deref(),
        std::env::current_exe().ok().as_deref(),
        std::env::var("MEMORPH_PYTHON_PREFIX").ok().as_deref(),
        std::env::var("MEMORPH_PYTHON_EXECUTABLE").ok().as_deref(),
    )
    .ok_or_else(|| {
        anyhow!(
            "Could not detect how memorph was installed.\n\
             Try one of these commands manually:\n\
             - npm install -g memorph@latest\n\
             - python -m pip install --upgrade memorph\n\
             - pipx upgrade memorph\n\
             - uv tool upgrade memorph\n\
             - download the latest desktop app from GitHub Releases"
        )
    })?;

    let latest_version = fetch_latest_version(source)
        .await
        .with_context(|| format!("Failed to fetch latest version from {}", source.label()))?;
    let current_version = env!("CARGO_PKG_VERSION");

    Ok(UpdateCheckPayload {
        current_version,
        latest_version: latest_version.clone(),
        install_source: source.slug().to_string(),
        install_source_label: source.label().to_string(),
        has_update: compare_versions(&latest_version, current_version) == Ordering::Greater,
        update_command: source.update_command(std::env::var("MEMORPH_PYTHON_EXECUTABLE").ok()),
        release_url: source.release_url().to_string(),
    })
}

fn settings_payload() -> anyhow::Result<SettingsPayload> {
    let prefs = config::web_preferences()?;
    let server = config::server_preferences()?;
    Ok(SettingsPayload {
        sessions_per_provider: prefs.sessions_per_provider,
        language: prefs.language,
        show_opencode_subagents: config::provider_preference_from_prefs(
            &prefs,
            "opencode",
            "show_subagents",
        )
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false),
        sort_providers_by_session_count: prefs.sort_providers_by_session_count,
        default_backup_dir: prefs.default_backup_dir.clone(),
        logging: prefs.logging.clone(),
        home_buttons: prefs.home_buttons.clone(),
        agent_order: config::ordered_provider_ids(&prefs),
        primary_agents: config::primary_provider_ids(&prefs),
        server,
    })
}

fn config_file_payload() -> anyhow::Result<ConfigFilePayload> {
    let path = config::config_path()?;
    let content = if path.exists() {
        std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?
    } else {
        serde_json::to_string_pretty(&config::MemorphConfig::default())?
    };
    Ok(ConfigFilePayload {
        path: path.display().to_string(),
        format: "json",
        content,
    })
}

fn settings_paths_payload() -> anyhow::Result<SettingsPathsPayload> {
    let prefs = config::web_preferences()?;
    let memorph_dir = config::memorph_dir()?;
    let log_dir = memorph_dir.join("logs");
    let log_file_name = "memorph.log";
    let backup_dir_input = prefs.default_backup_dir.clone();
    let backup_dir_base = backup_dir_base_path(None);
    let backup_dir_resolved = resolve_backup_output_dir(&backup_dir_input, None);

    Ok(SettingsPathsPayload {
        backup_dir_input,
        backup_dir_resolved: display_settings_path(&backup_dir_resolved),
        backup_dir_base: display_settings_path(&backup_dir_base),
        log_dir: display_settings_path(&log_dir),
        log_file_name,
        log_file_path: display_settings_path(&log_dir.join(log_file_name)),
    })
}

async fn get_meta() -> impl IntoResponse {
    match (
        settings_payload(),
        config::selected_workspace(),
        config::known_workspaces(),
        settings_paths_payload(),
        config_file_payload(),
    ) {
        (
            Ok(settings),
            Ok(selected_workspace),
            Ok(workspaces),
            Ok(settings_paths),
            Ok(config_file),
        ) => ApiResponse::success(MetaPayload {
            version: env!("CARGO_PKG_VERSION"),
            selected_workspace,
            workspaces,
            capabilities: CapabilitiesPayload {
                system_folder_picker: FOLDER_PICKER.get().is_some(),
            },
            settings,
            settings_paths,
            config_file,
        })
        .into_response(),
        (Err(e), _, _, _, _)
        | (_, Err(e), _, _, _)
        | (_, _, Err(e), _, _)
        | (_, _, _, Err(e), _)
        | (_, _, _, _, Err(e)) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn check_for_update() -> impl IntoResponse {
    match update_check_payload().await {
        Ok(payload) => ApiResponse::success(payload).into_response(),
        Err(error) => api_error(StatusCode::BAD_GATEWAY, error).into_response(),
    }
}

#[derive(Deserialize)]
struct CatalogQuery {
    workspace: Option<String>,
}

#[derive(Deserialize)]
struct UpdateCatalogBody {
    #[serde(default)]
    sort_order: config::ProviderDisplayOrder,
    #[serde(default)]
    hidden_state: config::ProviderDisplayHidden,
    workspace: Option<String>,
}

fn directory_listing(requested_path: Option<&str>) -> anyhow::Result<DirectoryListingPayload> {
    let requested = requested_path
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let candidate = match requested {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() {
                return Err(anyhow!("Directory path must be absolute."));
            }
            path
        }
        None => config::selected_workspace()?
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_dir())
            .or_else(dirs::home_dir)
            .unwrap_or(std::env::current_dir()?),
    };

    let path = candidate
        .canonicalize()
        .with_context(|| format!("Cannot access directory: {}", candidate.display()))?;
    if !path.is_dir() {
        return Err(anyhow!("Path is not a directory: {}", path.display()));
    }

    let mut directories = std::fs::read_dir(&path)
        .with_context(|| format!("Cannot read directory: {}", path.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let entry_path = entry.path();
            entry_path.is_dir().then(|| DirectoryEntryPayload {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry_path.to_string_lossy().into_owned(),
            })
        })
        .collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(DirectoryListingPayload {
        path: path.to_string_lossy().into_owned(),
        parent: path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned()),
        directories,
    })
}

async fn list_directories(Query(query): Query<DirectoryQuery>) -> impl IntoResponse {
    match directory_listing(query.path.as_deref()) {
        Ok(listing) => ApiResponse::success(listing).into_response(),
        Err(error) => api_error(StatusCode::BAD_REQUEST, error).into_response(),
    }
}

async fn select_folder(Json(body): Json<SelectFolderBody>) -> impl IntoResponse {
    let Some(picker) = FOLDER_PICKER.get() else {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            "Folder picker is only available in the desktop app.",
        )
        .into_response();
    };

    match picker(body.start_path) {
        Ok(path) => ApiResponse::success(SelectFolderPayload { path }).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn select_file(Json(body): Json<SelectFileBody>) -> impl IntoResponse {
    let Some(picker) = FILE_PICKER.get() else {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            "File picker is only available in the desktop app.",
        )
        .into_response();
    };

    match picker(body.start_path) {
        Ok(path) => ApiResponse::success(SelectFilePayload { path }).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn open_external(Json(body): Json<OpenExternalBody>) -> impl IntoResponse {
    let url = body.url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return api_error(StatusCode::BAD_REQUEST, "Only http(s) URLs are supported.")
            .into_response();
    }

    match open::that_detached(url) {
        Ok(()) => ApiResponse::success(OpenExternalPayload { opened: true }).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn get_settings() -> impl IntoResponse {
    match settings_payload() {
        Ok(settings) => ApiResponse::success(settings).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn update_settings(Json(body): Json<SettingsBody>) -> impl IntoResponse {
    let update_result = config::update_web_preferences(
        Some(body.sessions_per_provider),
        Some(body.language),
        Some(body.show_opencode_subagents),
        body.sort_providers_by_session_count,
        Some(body.default_backup_dir),
        Some(body.logging),
    )
    .and_then(|_| config::update_home_button_config(body.home_buttons))
    .and_then(|_| {
        if let Some(server) = body.server {
            config::update_server_preferences(server)
        } else {
            Ok(())
        }
    })
    .and_then(|_| {
        config::update_agent_display_preferences(
            config::ProviderDisplayOrder {
                global: body.agent_order,
                workspace: Vec::new(),
            },
            config::ProviderDisplayHidden {
                global: Vec::new(),
                workspace: Vec::new(),
            },
        )
    });

    match update_result.and_then(|_| settings_payload()) {
        Ok(settings) => ApiResponse::success(settings).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    all: Option<bool>,
    provider: Option<String>,
    dir: Option<String>,
    workspace: Option<String>,
    details: Option<bool>,
    limit: Option<usize>,
    offset: Option<usize>,
    sort: Option<core::SessionListSort>,
    hook_filter: Option<core::SessionHookFilter>,
}

#[derive(Deserialize)]
struct ProviderActivityQuery {
    workspace: Option<String>,
    hours: Option<i64>,
    all: Option<bool>,
    all_time: Option<bool>,
}

#[derive(Deserialize)]
struct SessionDetailQuery {
    event_limit: Option<usize>,
    event_offset: Option<usize>,
}

async fn list_workspaces() -> impl IntoResponse {
    match config::known_workspaces() {
        Ok(workspaces) => ApiResponse::success(workspaces).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct WorkspacesWithSessionsQuery {
    q: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
}

async fn list_workspaces_with_sessions(
    Query(query): Query<WorkspacesWithSessionsQuery>,
) -> impl IntoResponse {
    let options = crate::core::manager::WorkspaceWithSessionsOptions {
        search: query
            .q
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        page: query.page.unwrap_or(1),
        page_size: query.page_size.unwrap_or(5),
    };

    match run_manager_blocking(move || crate::core::manager::workspaces_with_sessions(&options))
        .await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct WorkspaceHistoryBody {
    workspace: String,
}

async fn remove_workspace_history(Json(body): Json<WorkspaceHistoryBody>) -> impl IntoResponse {
    match config::remove_workspace_history(&body.workspace) {
        Ok(workspaces) => ApiResponse::success(workspaces).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct WorkspaceQuery {
    workspace: String,
}

async fn get_workspace_providers(Query(q): Query<WorkspaceQuery>) -> impl IntoResponse {
    match config::workspace_providers(&q.workspace) {
        Ok(providers) => ApiResponse::success(providers).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct WorkspaceProvidersBody {
    workspace: String,
    providers: Vec<String>,
}

async fn update_workspace_providers(Json(body): Json<WorkspaceProvidersBody>) -> impl IntoResponse {
    match config::set_workspace_providers(&body.workspace, body.providers) {
        Ok(()) => match config::workspace_providers(&body.workspace) {
            Ok(providers) => ApiResponse::success(providers).into_response(),
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct RenameBody {
    title: String,
}

#[derive(Deserialize)]
struct ProviderSettingUpdateBody {
    value: Option<Value>,
}

#[derive(Deserialize)]
struct ProviderSettingRunBody {
    workspace: Option<String>,
}

#[derive(Deserialize)]
struct ExportBody {
    provider: String,
    session_id: String,
    output_prefix: Option<String>,
    #[serde(default = "default_format")]
    format: String,
    #[serde(default)]
    output_dir: Option<String>,
}

fn default_format() -> String {
    "both".to_string()
}

async fn export_session(Json(body): Json<ExportBody>) -> impl IntoResponse {
    let params = core::ExportParams {
        provider: body.provider,
        session_id: body.session_id,
        output_prefix: body.output_prefix,
        format: body.format,
        output_dir: body.output_dir,
    };
    match core::export_session(&params, ActivityActor::Api) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ImportBody {
    provider: String,
    file_or_id: String,
    to_dir: Option<String>,
}

async fn import_session(Json(body): Json<ImportBody>) -> impl IntoResponse {
    let params = core::ImportParams {
        provider: body.provider,
        file_or_id: body.file_or_id,
        to_dir: body.to_dir,
    };
    match core::import_session(&params, ActivityActor::Api) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SwitchBody {
    from: String,
    to: String,
    session_id: Option<String>,
    to_dir: Option<String>,
    target_title: Option<String>,
    #[serde(default)]
    move_original: bool,
}

async fn switch_session(Json(body): Json<SwitchBody>) -> impl IntoResponse {
    let params = core::SwitchParams {
        from: body.from,
        to: body.to,
        session_id: body.session_id,
        to_dir: body.to_dir,
        target_title: body.target_title,
        move_original: body.move_original,
    };
    match core::switch_session(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct FindQuery {
    dir: Option<String>,
    session: Option<String>,
    provider: Option<String>,
}

async fn find_sessions(Query(q): Query<FindQuery>) -> impl IntoResponse {
    if q.dir.is_none() && q.session.is_none() && q.provider.is_none() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "At least one filter required: dir, session, or provider",
        )
        .into_response();
    }
    let providers = q
        .provider
        .map(|p| p.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let params = core::FindParams {
        dir: q.dir,
        session: q.session,
        providers,
    };
    match core::find_sessions(&params) {
        Ok(groups) => ApiResponse::success(groups).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn list_sync_groups() -> impl IntoResponse {
    match session_sync::list_groups() {
        Ok(items) => ApiResponse::success(items).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SyncCreateBody {
    provider: String,
    session_id: String,
    #[serde(default)]
    targets: Vec<String>,
    to_dir: Option<String>,
    title: Option<String>,
}

async fn create_sync_group(Json(body): Json<SyncCreateBody>) -> impl IntoResponse {
    let params = session_sync::SyncCreateParams {
        provider: body.provider,
        session_id: body.session_id,
        targets: body.targets,
        to_dir: body.to_dir,
        title: body.title,
    };
    match session_sync::create_group(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SyncBindBody {
    group_id: String,
    provider: String,
    session_id: Option<String>,
    to_dir: Option<String>,
}

async fn bind_sync_group(Json(body): Json<SyncBindBody>) -> impl IntoResponse {
    let params = session_sync::AddHoldingParams {
        group_id: body.group_id,
        provider: body.provider,
        session_id: body.session_id,
        to_dir: body.to_dir,
    };
    match session_sync::add_holding(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn unbind_sync_group(
    Path((group_id, holding_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match session_sync::remove_holding(&group_id, &holding_id) {
        Ok(()) => ApiResponse::success("unbound").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SyncStatusQuery {
    group_id: Option<String>,
}

async fn sync_status(Query(q): Query<SyncStatusQuery>) -> impl IntoResponse {
    match q.group_id {
        Some(id) => match session_sync::load_group(&id) {
            Ok(mut group) => {
                let _ = session_sync::refresh_active_times(&mut group);
                ApiResponse::success(sync_group_payload(group)).into_response()
            }
            Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
        },
        None => match session_sync::list_groups() {
            Ok(mut groups) => {
                for group in &mut groups {
                    let _ = session_sync::refresh_active_times(group);
                }
                let payload: Vec<SyncGroupPayload> =
                    groups.into_iter().map(sync_group_payload).collect();
                ApiResponse::success(payload).into_response()
            }
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
    }
}

fn sync_group_payload(group: session_sync::SyncGroup) -> SyncGroupPayload {
    SyncGroupPayload {
        id: group.id,
        title: group.title,
        source_provider: group.source_provider,
        created_at: group.created_at,
        updated_at: group.updated_at,
        holdings: group
            .holdings
            .into_iter()
            .map(sync_holding_payload)
            .collect(),
    }
}

fn sync_holding_payload(holding: session_sync::Holding) -> SyncHoldingPayload {
    let hook_augmentation = hooks::augmentation::augment_session(
        &holding.provider,
        &holding.session_id,
        holding.target_dir.as_deref(),
    );
    SyncHoldingPayload {
        id: holding.id,
        provider: holding.provider,
        session_id: holding.session_id,
        target_dir: holding.target_dir,
        created_at: holding.created_at,
        last_active_at: holding.last_active_at,
        last_sync_at: holding.last_sync_at,
        last_sync_from: holding.last_sync_from,
        last_error: holding.last_error,
        hook_runtime_summary: hook_augmentation.runtime_summary,
        hook_diagnosis: hook_augmentation.diagnosis,
        hook_runtime_sessions: hook_augmentation.runtime_sessions,
    }
}

fn resolve_sync_source(
    group: &mut session_sync::SyncGroup,
    source_holding_id: Option<String>,
) -> anyhow::Result<String> {
    if let Some(source_id) = source_holding_id {
        if !group.holdings.iter().any(|holding| holding.id == source_id) {
            anyhow::bail!("Source holding not found: {}", source_id);
        }
        return Ok(source_id);
    }

    session_sync::refresh_active_times(group)?;
    group
        .holdings
        .iter()
        .filter(|holding| holding.last_active_at.is_some())
        .max_by_key(|holding| holding.last_active_at.unwrap_or(0))
        .map(|holding| holding.id.clone())
        .with_context(|| "No holding with active time found")
}

fn hook_status_blocks_sync(status: &hooks::model::RuntimeSessionStatus) -> bool {
    matches!(
        status,
        hooks::model::RuntimeSessionStatus::Running
            | hooks::model::RuntimeSessionStatus::WaitingPermission
            | hooks::model::RuntimeSessionStatus::WaitingUser
    )
}

fn blocked_sync_targets_from_snapshot(
    group: &session_sync::SyncGroup,
    source_holding_id: &str,
    snapshot: &[hooks::model::RuntimeSession],
) -> Vec<String> {
    group
        .holdings
        .iter()
        .filter(|holding| holding.id != source_holding_id)
        .filter_map(|holding| {
            let augmentation = hooks::augmentation::augment_session_from_snapshot(
                snapshot,
                &holding.provider,
                &holding.session_id,
                holding.target_dir.as_deref(),
            );
            let summary = augmentation.runtime_summary?;
            if !hook_status_blocks_sync(&summary.status) {
                return None;
            }
            Some(format!(
                "{}:{} is {:?}",
                holding.provider, holding.session_id, summary.status
            ))
        })
        .collect()
}

#[derive(Deserialize)]
struct SyncGroupBody {
    group_id: String,
    source_holding_id: Option<String>,
}

async fn sync_session_groups(Json(body): Json<SyncGroupBody>) -> impl IntoResponse {
    let mut group = match session_sync::load_group(&body.group_id) {
        Ok(group) => group,
        Err(e) => return api_error(StatusCode::NOT_FOUND, e).into_response(),
    };
    let source_id = match resolve_sync_source(&mut group, body.source_holding_id) {
        Ok(source_id) => source_id,
        Err(e) => return api_error(StatusCode::BAD_REQUEST, e).into_response(),
    };
    let blocked = blocked_sync_targets_from_snapshot(
        &group,
        &source_id,
        &hooks::server::runtime_sessions_snapshot(),
    );
    if !blocked.is_empty() {
        return api_error(
            StatusCode::CONFLICT,
            format!(
                "Session sync blocked because target sessions are active: {}",
                blocked.join("; ")
            ),
        )
        .into_response();
    }

    let result = session_sync::push_sync(&body.group_id, &source_id, ActivityActor::Api);
    match result {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SyncRemoveQuery {
    delete_provider_sessions: Option<bool>,
}

async fn remove_sync_group(
    Path(group_id): Path<String>,
    Query(q): Query<SyncRemoveQuery>,
) -> impl IntoResponse {
    match session_sync::delete_group(&group_id, q.delete_provider_sessions.unwrap_or(false)) {
        Ok(()) => ApiResponse::success("removed").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SyncRenameBody {
    title: String,
}

async fn rename_sync_group(
    Path(group_id): Path<String>,
    Json(body): Json<SyncRenameBody>,
) -> impl IntoResponse {
    match session_sync::rename_group(&group_id, &body.title) {
        Ok(()) => ApiResponse::success("renamed").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ManagerPreviewBody {
    #[serde(default)]
    providers: Vec<String>,
    older_than_days: Option<u32>,
    older_than_ms: Option<i64>,
    larger_than_mb: Option<u32>,
    larger_than_bytes: Option<u64>,
    smaller_than_bytes: Option<u64>,
    workspace: Option<String>,
    sort: Option<String>,
    limit: Option<usize>,
}

fn manager_filter_from_body(
    body: ManagerPreviewBody,
    workspace: Option<String>,
    limit: Option<usize>,
) -> crate::core::manager::ManagerFilter {
    crate::core::manager::ManagerFilter {
        providers: body.providers,
        older_than_days: body.older_than_days,
        older_than_ms: body.older_than_ms,
        larger_than_mb: body.larger_than_mb,
        larger_than_bytes: body.larger_than_bytes,
        smaller_than_bytes: body.smaller_than_bytes,
        workspace,
        sort: body.sort,
        limit,
    }
}

async fn run_manager_blocking<T, F>(task: F) -> Result<T, anyhow::Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, anyhow::Error> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| anyhow!("manager task failed: {}", e))?
}

async fn manager_preview(Json(body): Json<ManagerPreviewBody>) -> impl IntoResponse {
    let workspace = body.workspace.clone();
    let limit = body.limit;
    let filter = manager_filter_from_body(body, workspace, limit);
    match run_manager_blocking(move || crate::core::manager::preview(&filter)).await {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Debug, Serialize)]
struct ManagerQuickPreviewResult {
    items: Vec<crate::core::manager::ManagerItem>,
    total_count: usize,
    total_size_bytes: u64,
    selected_agent_count: usize,
}

#[derive(Deserialize)]
struct ManagerQuickQuery {
    /// Comma-separated provider ids. Empty/missing → fall back to all installed providers.
    #[serde(default)]
    providers: String,
}

const MANAGER_QUICK_LIMIT: usize = 15;

/// Resolve provider ids for a quick endpoint: explicit `?providers=` wins, otherwise
/// fall back to every currently-installed provider.
async fn resolve_quick_provider_ids(query: &str) -> Result<Vec<String>, anyhow::Error> {
    let trimmed: Vec<String> = query
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if !trimmed.is_empty() {
        return Ok(trimmed);
    }
    let catalog = build_provider_catalog_light(None).await?;
    Ok(catalog
        .providers
        .into_iter()
        .filter(|p| p.install_state.is_installed)
        .map(|p| p.provider_id)
        .collect())
}

fn quick_filter(provider_ids: Vec<String>) -> crate::core::manager::ManagerFilter {
    crate::core::manager::ManagerFilter {
        providers: provider_ids,
        older_than_days: None,
        older_than_ms: None,
        larger_than_mb: None,
        larger_than_bytes: None,
        smaller_than_bytes: None,
        workspace: None,
        sort: Some("recent".to_string()),
        limit: Some(MANAGER_QUICK_LIMIT),
    }
}

async fn manager_quick_preview(
    axum::extract::Query(query): axum::extract::Query<ManagerQuickQuery>,
) -> impl IntoResponse {
    let provider_ids = match resolve_quick_provider_ids(&query.providers).await {
        Ok(ids) => ids,
        Err(e) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    if provider_ids.is_empty() {
        return ApiResponse::success(ManagerQuickPreviewResult {
            items: Vec::new(),
            total_count: 0,
            total_size_bytes: 0,
            selected_agent_count: 0,
        })
        .into_response();
    }

    let selected_agent_count = provider_ids.len();
    let filter = quick_filter(provider_ids);

    match run_manager_blocking(move || crate::core::manager::preview(&filter)).await {
        Ok(preview) => ApiResponse::success(ManagerQuickPreviewResult {
            selected_agent_count,
            total_count: preview.total_count,
            total_size_bytes: preview.total_size_bytes,
            items: preview.items,
        })
        .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn manager_quick_workspaces(
    axum::extract::Query(query): axum::extract::Query<ManagerQuickQuery>,
) -> impl IntoResponse {
    let provider_ids = match resolve_quick_provider_ids(&query.providers).await {
        Ok(ids) => ids,
        Err(e) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    if provider_ids.is_empty() {
        return ApiResponse::success(crate::core::manager::ManagerWorkspacesResult {
            items: Vec::new(),
            total_count: 0,
            total_size_bytes: 0,
        })
        .into_response();
    }

    let filter = quick_filter(provider_ids);
    match run_manager_blocking(move || crate::core::manager::workspaces(&filter)).await {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn manager_stats(Json(body): Json<ManagerPreviewBody>) -> impl IntoResponse {
    let workspace = body.workspace.clone();
    let filter = manager_filter_from_body(body, workspace, None);
    match run_manager_blocking(move || crate::core::manager::stats(&filter)).await {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ManagerItemsBody {
    items: Vec<crate::core::manager::ManagerItem>,
    output_dir: Option<String>,
}

async fn manager_clean(Json(body): Json<ManagerItemsBody>) -> impl IntoResponse {
    let result = crate::core::manager::clean(&body.items, ActivityActor::Api);
    logging::info(
        "manager_clean",
        format!(
            "success={} failed={} freed_bytes={}",
            result.success, result.failed, result.freed_bytes
        ),
    );
    ApiResponse::success(result).into_response()
}

async fn manager_backup(Json(body): Json<ManagerItemsBody>) -> impl IntoResponse {
    let output_dir = body.output_dir.unwrap_or_else(|| "./backups".to_string());
    let resolved_output_dir = resolve_backup_output_dir(&output_dir, None);
    let result =
        crate::core::manager::backup(&body.items, &resolved_output_dir, ActivityActor::Api);
    logging::info(
        "manager_backup",
        format!(
            "success={} failed={} output_dir={}",
            result.success,
            result.failed,
            resolved_output_dir.display()
        ),
    );
    ApiResponse::success(result).into_response()
}

async fn manager_workspaces(Json(body): Json<ManagerPreviewBody>) -> impl IntoResponse {
    let limit = body.limit;
    let filter = manager_filter_from_body(body, None, limit);
    match run_manager_blocking(move || crate::core::manager::workspaces(&filter)).await {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ManagerWorkspaceBody {
    provider_id: String,
    workspace: String,
    output_dir: Option<String>,
}

async fn manager_clean_workspace(Json(body): Json<ManagerWorkspaceBody>) -> impl IntoResponse {
    let result = crate::core::manager::clean_workspace(
        &body.provider_id,
        &body.workspace,
        ActivityActor::Api,
    );
    logging::info(
        "manager_clean_workspace",
        format!(
            "provider={} workspace={} success={} failed={} freed_bytes={}",
            body.provider_id, body.workspace, result.success, result.failed, result.freed_bytes
        ),
    );
    ApiResponse::success(result).into_response()
}

async fn manager_backup_workspace(Json(body): Json<ManagerWorkspaceBody>) -> impl IntoResponse {
    let output_dir = body.output_dir.unwrap_or_else(|| "./backups".to_string());
    let resolved_output_dir = resolve_backup_output_dir(&output_dir, Some(&body.workspace));
    let result = crate::core::manager::backup_workspace(
        &body.provider_id,
        &body.workspace,
        &resolved_output_dir,
        ActivityActor::Api,
    );
    logging::info(
        "manager_backup_workspace",
        format!(
            "provider={} workspace={} success={} failed={} output_dir={}",
            body.provider_id,
            body.workspace,
            result.success,
            result.failed,
            resolved_output_dir.display()
        ),
    );
    ApiResponse::success(result).into_response()
}

#[cfg(test)]
mod tests;
