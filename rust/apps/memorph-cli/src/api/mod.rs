use anyhow::{anyhow, Context as _};
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

use memorph::{
    agent_management, cache, config, core, hooks, logging, provider_settings,
    providers::catalog::{build_catalog, sort_catalog, CatalogInput, ProviderCatalog},
    storage::activity_store::{
        ActivityActor, ActivityOperationKind, ActivityQuery, ActivityStatus,
    },
    sync as session_sync,
};

mod compression;
mod management;
mod manager;
mod meta;
mod providers;
mod router;
mod sessions;
mod sync;
mod system;
mod transfer;
mod workspaces;
pub use router::router;

fn default_format() -> String {
    "both".to_string()
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_order: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    matched_event_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    returned_event_indices: Vec<usize>,
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

#[derive(Debug, Deserialize)]
struct SessionWorkspaceIndexRequest {
    provider: String,
    workspace_dir: String,
}

#[derive(Debug, Serialize)]
struct SessionReprojectionPayload {
    candidate_snapshots: usize,
    reprojected_snapshots: usize,
    missing_sources: usize,
    unsupported_providers: usize,
    failed_snapshots: usize,
    failures: Vec<core::projection::SessionReprojectionFailure>,
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
    let visible = memorph::utils::user_visible_path(&path.to_string_lossy());
    let Ok(home_dir) = config::effective_home_dir() else {
        return visible;
    };
    let home_visible = memorph::utils::user_visible_path(&home_dir.to_string_lossy());
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
pub(super) struct SettingsBody {
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
pub(super) struct SelectFolderBody {
    start_path: Option<String>,
}

#[derive(Serialize)]
pub(super) struct SelectFolderPayload {
    path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct EnsureReadyPayload {
    /// Workspace the client should land in after ensure. May be None when no
    /// cwd and no history exist (fresh install with no terminal context).
    selected_workspace: Option<String>,
    /// Whether we had to repair the environment (create dir/config/db,
    /// or prime a default workspace). Lets the UI explain a brief reindex.
    repaired: bool,
}

#[derive(Deserialize)]
pub(super) struct DirectoryQuery {
    path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct DirectoryEntryPayload {
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
pub(super) struct SelectFileBody {
    start_path: Option<String>,
}

#[derive(Serialize)]
pub(super) struct SelectFilePayload {
    path: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct OpenExternalBody {
    url: String,
}

#[derive(Serialize)]
pub(super) struct OpenExternalPayload {
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
    memorph::providers::all_provider_ids()
        .iter()
        .filter_map(|id| memorph::providers::find_provider(id))
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
                memorph::agent_environment::detect_provider_environment_fast(&id),
            )
        }));
    }
    let mut env_by_provider: std::collections::HashMap<
        String,
        memorph::agent_environment::AgentEnvironmentStatus,
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
                memorph::agent_environment::AgentEnvironmentStatus {
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
        let conn = memorph::storage::local_store::open_database()?;
        memorph::storage::snapshot_store::SnapshotStore::new(&conn).list_session_snapshots()
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
    snapshots: &[memorph::storage::snapshot_store::ProjectedSessionSnapshotRow],
) -> cache::ProviderActiveCatalog {
    let providers = ordered_ids
        .iter()
        .map(|id| {
            let provider = memorph::providers::find_provider(id);
            let sessions: Vec<_> = snapshots
                .iter()
                .filter(|session| session.provider_id == *id)
                .collect();
            let workspace_sessions: Vec<_> = sessions
                .iter()
                .copied()
                .filter(|session| {
                    workspace.is_none_or(|workspace| {
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
                active_time: memorph::providers::catalog::ActiveTime {
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

#[derive(Deserialize)]
struct ListQuery {
    all: Option<bool>,
    provider: Option<String>,
    dir: Option<String>,
    workspace: Option<String>,
    fields: Option<core::SessionListFields>,
    limit: Option<usize>,
    offset: Option<usize>,
    sort: Option<core::SessionListSort>,
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
    event_search: Option<String>,
    event_order: Option<String>,
}

#[cfg(test)]
mod tests;
