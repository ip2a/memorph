use anyhow::{Context, anyhow};
use axum::{
    Json, Router,
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::{
    agent_management, config, core, hooks, logging, provider_features, provider_settings, shared,
};

type FolderPicker =
    dyn Fn(Option<String>) -> anyhow::Result<Option<String>> + Send + Sync + 'static;

static FOLDER_PICKER: OnceLock<Arc<FolderPicker>> = OnceLock::new();

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

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/meta", get(get_meta))
        .route("/api/v1/update-check", get(check_for_update))
        .route("/api/v1/agents", get(list_agent_management))
        .route(
            "/api/v1/agents/{provider}",
            get(get_agent_management_provider),
        )
        .route(
            "/api/v1/agents/{provider}/detect",
            post(detect_agent_management_provider),
        )
        .route("/api/v1/providers", get(list_providers))
        .route(
            "/api/v1/providers/{provider}/features",
            get(list_legacy_provider_features),
        )
        .route(
            "/api/v1/providers/{provider}/settings",
            get(list_provider_settings),
        )
        .route(
            "/api/v1/providers/{provider}/controls",
            get(list_legacy_provider_controls),
        )
        .route(
            "/api/v1/providers/{provider}/features/{feature_id}",
            get(get_legacy_provider_feature)
                .put(update_legacy_provider_feature)
                .post(run_legacy_provider_feature),
        )
        .route(
            "/api/v1/providers/{provider}/settings/{setting_id}",
            get(get_provider_setting)
                .put(update_provider_setting)
                .post(run_provider_setting),
        )
        .route(
            "/api/v1/providers/{provider}/controls/{control_id}",
            get(get_legacy_provider_control)
                .put(update_legacy_provider_control)
                .post(run_legacy_provider_control),
        )
        .route("/api/v1/settings", get(get_settings).put(update_settings))
        .route("/api/v1/system/select-folder", post(select_folder))
        .route("/api/v1/system/open-external", post(open_external))
        .route("/api/v1/sessions", get(list_sessions))
        .route("/api/v1/sessions/{provider}/{session_id}", get(get_session))
        .route(
            "/api/v1/sessions/{provider}/{session_id}",
            delete(delete_session)
                .patch(rename_session)
                .put(update_session_local_state),
        )
        .route("/api/v1/export", post(export_session))
        .route(
            "/api/v1/compression/archives",
            get(list_compression_archives),
        )
        .route(
            "/api/v1/compression/providers",
            get(list_compression_providers),
        )
        .route(
            "/api/v1/compression/tool-spec",
            get(get_compression_tool_spec),
        )
        .route(
            "/api/v1/compression/instructions",
            post(get_compression_retrieval_instructions),
        )
        .route(
            "/api/v1/compression/restore",
            post(restore_compression_archive),
        )
        .route(
            "/api/v1/compression/retrieve",
            post(retrieve_compression_archive),
        )
        .route(
            "/api/v1/compression/expand",
            post(expand_compression_session),
        )
        .route("/api/v1/compression/plan", post(plan_active_compression))
        .route("/api/v1/compression/apply", post(apply_active_compression))
        .route("/api/v1/import", post(import_session))
        .route("/api/v1/switch", post(switch_session))
        .route("/api/v1/find", get(find_sessions))
        .route("/api/v1/workspaces", get(list_workspaces))
        .route(
            "/api/v1/workspaces/history",
            delete(remove_workspace_history),
        )
        .route(
            "/api/v1/workspaces/providers",
            get(get_workspace_providers).put(update_workspace_providers),
        )
        .route(
            "/api/v1/share",
            get(list_shared_sessions).post(create_shared_session),
        )
        .route("/api/v1/share/status", get(shared_status))
        .route("/api/v1/share/sync", post(sync_shared_sessions))
        .route("/api/v1/share/bind", post(bind_shared_session))
        .route(
            "/api/v1/share/holdings/{group_id}/{holding_id}",
            delete(unbind_shared_session),
        )
        .route(
            "/api/v1/share/{group_id}",
            delete(remove_shared_session).patch(rename_shared_session),
        )
        .route("/api/v1/manager/preview", post(manager_preview))
        .route("/api/v1/manager/clean", post(manager_clean))
        .route("/api/v1/manager/backup", post(manager_backup))
        .merge(hooks::server::router())
}

pub fn register_folder_picker<F>(picker: F) -> bool
where
    F: Fn(Option<String>) -> anyhow::Result<Option<String>> + Send + Sync + 'static,
{
    FOLDER_PICKER.set(Arc::new(picker)).is_ok()
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
struct LegacyProviderFeaturePayload {
    provider_id: String,
    features: Vec<provider_features::ResolvedProviderFeature>,
}

#[derive(Debug, Serialize)]
struct LegacyProviderControlsPayload {
    provider_id: String,
    controls: Vec<provider_settings::ProviderSettingItem>,
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
}

#[derive(Debug, Serialize)]
struct ConfigFilePayload {
    path: String,
    format: &'static str,
    content: String,
}

#[derive(Debug, Serialize)]
struct MetaPayload {
    version: &'static str,
    providers: Vec<ProviderInfo>,
    selected_workspace: Option<String>,
    workspaces: Vec<config::WorkspaceEntry>,
    settings: SettingsPayload,
    config_file: ConfigFilePayload,
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
struct SharedHoldingPayload {
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
struct SharedGroupPayload {
    id: String,
    title: String,
    #[serde(default)]
    source_provider: Option<String>,
    created_at: i64,
    updated_at: i64,
    holdings: Vec<SharedHoldingPayload>,
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
    primary_agents: Vec<String>,
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
    Ok(SettingsPayload {
        sessions_per_provider: prefs.sessions_per_provider,
        language: prefs.language,
        show_opencode_subagents: prefs.show_opencode_subagents,
        sort_providers_by_session_count: prefs.sort_providers_by_session_count,
        default_backup_dir: prefs.default_backup_dir.clone(),
        logging: prefs.logging.clone(),
        home_buttons: prefs.home_buttons.clone(),
        agent_order: config::ordered_provider_ids(&prefs),
        primary_agents: config::primary_provider_ids(&prefs),
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

async fn get_meta() -> impl IntoResponse {
    match (
        settings_payload(),
        config::selected_workspace(),
        config::known_workspaces(),
        config_file_payload(),
    ) {
        (Ok(settings), Ok(selected_workspace), Ok(workspaces), Ok(config_file)) => {
            ApiResponse::success(MetaPayload {
                version: env!("CARGO_PKG_VERSION"),
                providers: provider_info_list(),
                selected_workspace,
                workspaces,
                settings,
                config_file,
            })
            .into_response()
        }
        (Err(e), _, _, _) | (_, Err(e), _, _) | (_, _, Err(e), _) | (_, _, _, Err(e)) => {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}

async fn check_for_update() -> impl IntoResponse {
    match update_check_payload().await {
        Ok(payload) => ApiResponse::success(payload).into_response(),
        Err(error) => api_error(StatusCode::BAD_GATEWAY, error).into_response(),
    }
}

async fn list_agent_management() -> impl IntoResponse {
    match agent_management::list_agent_management_entries() {
        Ok(providers) => ApiResponse::success(AgentManagementPayload { providers }).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn get_agent_management_provider(Path(provider): Path<String>) -> impl IntoResponse {
    match agent_management::get_agent_management_entry(&provider) {
        Ok(provider) => ApiResponse::success(provider).into_response(),
        Err(error) => api_error(StatusCode::NOT_FOUND, error).into_response(),
    }
}

async fn detect_agent_management_provider(Path(provider): Path<String>) -> impl IntoResponse {
    match agent_management::detect_agent_management_entry(&provider) {
        Ok(provider) => ApiResponse::success(provider).into_response(),
        Err(error) => api_error(StatusCode::NOT_FOUND, error).into_response(),
    }
}

async fn list_providers() -> impl IntoResponse {
    ApiResponse::success(provider_info_list()).into_response()
}

async fn list_legacy_provider_features(Path(provider): Path<String>) -> impl IntoResponse {
    match provider_features::list_provider_features(&provider) {
        Ok(features) => ApiResponse::success(LegacyProviderFeaturePayload {
            provider_id: provider,
            features,
        })
        .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn list_legacy_provider_controls(Path(provider): Path<String>) -> impl IntoResponse {
    match provider_settings::list_provider_settings(&provider) {
        Ok(controls) => ApiResponse::success(LegacyProviderControlsPayload {
            provider_id: provider,
            controls,
        })
        .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn list_provider_settings(Path(provider): Path<String>) -> impl IntoResponse {
    match provider_settings::list_provider_settings(&provider) {
        Ok(settings) => ApiResponse::success(ProviderSettingsPayload {
            provider_id: provider,
            settings,
        })
        .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn get_legacy_provider_feature(
    Path((provider, feature_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match provider_features::get_provider_feature(&provider, &feature_id) {
        Ok(feature) => ApiResponse::success(feature).into_response(),
        Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
    }
}

async fn get_provider_setting(
    Path((provider, setting_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match provider_settings::get_provider_setting(&provider, &setting_id) {
        Ok(setting) => ApiResponse::success(setting).into_response(),
        Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
    }
}

async fn get_legacy_provider_control(
    Path((provider, control_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match provider_settings::get_provider_setting(&provider, &control_id) {
        Ok(control) => ApiResponse::success(control).into_response(),
        Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
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
    .and_then(|_| config::update_agent_display_preferences(body.agent_order, body.primary_agents));

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
struct SessionDetailQuery {
    event_limit: Option<usize>,
    event_offset: Option<usize>,
}

async fn list_sessions(Query(q): Query<ListQuery>) -> impl IntoResponse {
    let providers: Vec<String> = q
        .provider
        .map(|p| p.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let cwd = q.workspace.or(q.dir).or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });

    if let Some(workspace) = cwd.as_deref() {
        let _ = config::remember_workspace(std::path::Path::new(workspace));
    }

    let params = core::SessionListParams {
        all: q.all.unwrap_or(false),
        providers,
        cwd,
        include_message_counts: q.details.unwrap_or(true),
        limit: q.limit,
        offset: q.offset,
        sort: q.sort.unwrap_or_default(),
        hook_filter: q.hook_filter.unwrap_or_default(),
    };
    match core::list_sessions(&params) {
        Ok(groups) => ApiResponse::success(groups).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn list_workspaces() -> impl IntoResponse {
    match config::known_workspaces() {
        Ok(workspaces) => ApiResponse::success(workspaces).into_response(),
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

async fn get_session(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<SessionDetailQuery>,
) -> impl IntoResponse {
    let events_offset = q.event_offset.unwrap_or(0);
    let events_limit = q.event_limit;
    match core::get_session_detail_view_page(&provider, &session_id, events_offset, events_limit) {
        Ok(view) => {
            if let Some(project_dir) = view.workspace_dir.as_deref() {
                let _ = config::remember_workspace(std::path::Path::new(project_dir));
            }
            let returned_event_count = view.events.len();
            let has_more_events = events_offset + returned_event_count < view.event_count;
            let hook_runtime_sessions = view.hook_runtime_sessions.clone();
            ApiResponse::success(SessionDetailPayload {
                view,
                events_offset,
                events_limit,
                returned_event_count,
                has_more_events,
                hook_runtime_sessions,
            })
            .into_response()
        }
        Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
    }
}

async fn delete_session(Path((provider, session_id)): Path<(String, String)>) -> impl IntoResponse {
    match core::delete_session(&provider, &session_id) {
        Ok(()) => ApiResponse::success("deleted").into_response(),
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

async fn rename_session(
    Path((provider, session_id)): Path<(String, String)>,
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    match core::rename_session(&provider, &session_id, &body.title) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn update_session_local_state(
    Path((provider, session_id)): Path<(String, String)>,
    Json(body): Json<crate::storage::session_state::SessionLocalStateUpdate>,
) -> impl IntoResponse {
    match core::update_session_local_state(&provider, &session_id, &body) {
        Ok(state) => ApiResponse::success(state).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn update_legacy_provider_feature(
    Path((provider, feature_id)): Path<(String, String)>,
    Json(body): Json<ProviderSettingUpdateBody>,
) -> impl IntoResponse {
    match provider_features::update_provider_feature(&provider, &feature_id, body.value) {
        Ok(feature) => ApiResponse::success(feature).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn update_legacy_provider_control(
    Path((provider, control_id)): Path<(String, String)>,
    Json(body): Json<ProviderSettingUpdateBody>,
) -> impl IntoResponse {
    match provider_settings::update_provider_setting(&provider, &control_id, body.value) {
        Ok(control) => ApiResponse::success(control).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn update_provider_setting(
    Path((provider, setting_id)): Path<(String, String)>,
    Json(body): Json<ProviderSettingUpdateBody>,
) -> impl IntoResponse {
    match provider_settings::update_provider_setting(&provider, &setting_id, body.value) {
        Ok(setting) => ApiResponse::success(setting).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn run_legacy_provider_feature(
    Path((provider, feature_id)): Path<(String, String)>,
    Json(body): Json<ProviderSettingRunBody>,
) -> impl IntoResponse {
    match provider_features::run_provider_feature(
        &provider,
        &feature_id,
        provider_features::ProviderFeatureContext {
            workspace: body.workspace,
        },
    ) {
        Ok(output) => ApiResponse::success(output).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn run_provider_setting(
    Path((provider, setting_id)): Path<(String, String)>,
    Json(body): Json<ProviderSettingRunBody>,
) -> impl IntoResponse {
    match provider_settings::run_provider_setting(
        &provider,
        &setting_id,
        provider_settings::ProviderSettingContext {
            workspace: body.workspace,
        },
    ) {
        Ok(output) => ApiResponse::success(output).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn run_legacy_provider_control(
    Path((provider, control_id)): Path<(String, String)>,
    Json(body): Json<ProviderSettingRunBody>,
) -> impl IntoResponse {
    match provider_settings::run_provider_setting(
        &provider,
        &control_id,
        provider_settings::ProviderSettingContext {
            workspace: body.workspace,
        },
    ) {
        Ok(output) => ApiResponse::success(output).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ExportBody {
    provider: String,
    session_id: String,
    output_prefix: Option<String>,
    #[serde(default = "default_format")]
    format: String,
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
    };
    match core::export_session(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn list_compression_archives() -> impl IntoResponse {
    match core::list_compression_archives() {
        Ok(items) => ApiResponse::success(items).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn list_compression_providers() -> impl IntoResponse {
    ApiResponse::success(core::list_compression_provider_support())
}

async fn get_compression_tool_spec() -> impl IntoResponse {
    ApiResponse::success(core::compression_retrieval_tool_spec())
}

#[derive(Deserialize)]
struct CompressionRetrievalInstructionsBody {
    archive_ref: String,
}

async fn get_compression_retrieval_instructions(
    Json(body): Json<CompressionRetrievalInstructionsBody>,
) -> impl IntoResponse {
    match core::compression_retrieval_instructions(&body.archive_ref) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct RestoreCompressionArchiveBody {
    archive_ref: String,
    output_prefix: Option<String>,
    #[serde(default = "default_format")]
    format: String,
}

#[derive(Deserialize)]
struct RetrieveCompressionArchiveBody {
    archive_ref: String,
    query: Option<String>,
    max_results: Option<usize>,
}

#[derive(Deserialize)]
struct ExpandCompressionSessionBody {
    file: String,
    output_prefix: Option<String>,
    #[serde(default = "default_format")]
    format: String,
}

#[derive(Deserialize)]
struct ActiveCompressionPlanBody {
    source_provider_id: String,
    target_provider_id: String,
    session_id: Option<String>,
    file: Option<String>,
    #[serde(default)]
    policy: core::active_compression::ActiveCompressionPolicy,
}

#[derive(Deserialize)]
struct ActiveCompressionApplyBody {
    source_provider_id: String,
    target_provider_id: String,
    session_id: Option<String>,
    file: Option<String>,
    #[serde(default)]
    policy: core::active_compression::ActiveCompressionPolicy,
    #[serde(default)]
    candidate_ids: Vec<String>,
    output_prefix: Option<String>,
    #[serde(default = "default_format")]
    format: String,
}

async fn restore_compression_archive(
    Json(body): Json<RestoreCompressionArchiveBody>,
) -> impl IntoResponse {
    let params = core::RestoreCompressionArchiveParams {
        archive_ref: body.archive_ref,
        output_prefix: body.output_prefix,
        format: body.format,
    };
    match core::restore_compression_archive(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn retrieve_compression_archive(
    Json(body): Json<RetrieveCompressionArchiveBody>,
) -> impl IntoResponse {
    let params = core::RetrieveCompressionArchiveParams {
        archive_ref: body.archive_ref,
        query: body.query,
        max_results: body.max_results,
    };
    match core::retrieve_compression_archive(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn expand_compression_session(
    Json(body): Json<ExpandCompressionSessionBody>,
) -> impl IntoResponse {
    let params = core::ExpandCompressionSessionParams {
        file: body.file,
        output_prefix: body.output_prefix,
        format: body.format,
    };
    match core::expand_compression_session(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn plan_active_compression(Json(body): Json<ActiveCompressionPlanBody>) -> impl IntoResponse {
    let params = core::ActiveCompressionDryRunParams {
        source_provider_id: body.source_provider_id,
        target_provider_id: body.target_provider_id,
        session_id: body.session_id,
        file: body.file,
        policy: body.policy,
    };
    match core::active_compression_dry_run(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn apply_active_compression(
    Json(body): Json<ActiveCompressionApplyBody>,
) -> impl IntoResponse {
    let params = core::ActiveCompressionApplyCommandParams {
        source_provider_id: body.source_provider_id,
        target_provider_id: body.target_provider_id,
        session_id: body.session_id,
        file: body.file,
        policy: body.policy,
        candidate_ids: body.candidate_ids,
        output_prefix: body.output_prefix,
        format: body.format,
    };
    match core::active_compression_apply(&params) {
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
    match core::import_session(&params) {
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
    active_compression: Option<core::active_compression::ActiveCompressionPolicy>,
}

async fn switch_session(Json(body): Json<SwitchBody>) -> impl IntoResponse {
    let params = core::SwitchParams {
        from: body.from,
        to: body.to,
        session_id: body.session_id,
        to_dir: body.to_dir,
        target_title: body.target_title,
        move_original: body.move_original,
        active_compression: body.active_compression,
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

async fn list_shared_sessions() -> impl IntoResponse {
    match shared::list_groups() {
        Ok(items) => ApiResponse::success(items).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ShareCreateBody {
    provider: String,
    session_id: String,
    #[serde(default)]
    targets: Vec<String>,
    to_dir: Option<String>,
    title: Option<String>,
}

async fn create_shared_session(Json(body): Json<ShareCreateBody>) -> impl IntoResponse {
    let params = shared::ShareCreateParams {
        provider: body.provider,
        session_id: body.session_id,
        targets: body.targets,
        to_dir: body.to_dir,
        title: body.title,
    };
    match shared::create_group(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ShareBindBody {
    group_id: String,
    provider: String,
    session_id: Option<String>,
    to_dir: Option<String>,
}

async fn bind_shared_session(Json(body): Json<ShareBindBody>) -> impl IntoResponse {
    let params = shared::AddHoldingParams {
        group_id: body.group_id,
        provider: body.provider,
        session_id: body.session_id,
        to_dir: body.to_dir,
    };
    match shared::add_holding(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn unbind_shared_session(
    Path((group_id, holding_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match shared::remove_holding(&group_id, &holding_id) {
        Ok(()) => ApiResponse::success("unbound").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ShareStatusQuery {
    group_id: Option<String>,
}

async fn shared_status(Query(q): Query<ShareStatusQuery>) -> impl IntoResponse {
    match q.group_id {
        Some(id) => match shared::load_group(&id) {
            Ok(mut group) => {
                let _ = shared::refresh_active_times(&mut group);
                ApiResponse::success(shared_group_payload(group)).into_response()
            }
            Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
        },
        None => match shared::list_groups() {
            Ok(mut groups) => {
                for group in &mut groups {
                    let _ = shared::refresh_active_times(group);
                }
                let payload: Vec<SharedGroupPayload> =
                    groups.into_iter().map(shared_group_payload).collect();
                ApiResponse::success(payload).into_response()
            }
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
    }
}

fn shared_group_payload(group: shared::SharedGroup) -> SharedGroupPayload {
    SharedGroupPayload {
        id: group.id,
        title: group.title,
        source_provider: group.source_provider,
        created_at: group.created_at,
        updated_at: group.updated_at,
        holdings: group
            .holdings
            .into_iter()
            .map(shared_holding_payload)
            .collect(),
    }
}

fn shared_holding_payload(holding: shared::Holding) -> SharedHoldingPayload {
    let hook_augmentation = hooks::augmentation::augment_session(
        &holding.provider,
        &holding.session_id,
        holding.target_dir.as_deref(),
    );
    SharedHoldingPayload {
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

fn resolve_share_sync_source(
    group: &mut shared::SharedGroup,
    source_holding_id: Option<String>,
) -> anyhow::Result<String> {
    if let Some(source_id) = source_holding_id {
        if !group.holdings.iter().any(|holding| holding.id == source_id) {
            anyhow::bail!("Source holding not found: {}", source_id);
        }
        return Ok(source_id);
    }

    shared::refresh_active_times(group)?;
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
    group: &shared::SharedGroup,
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
struct ShareSyncBody {
    group_id: String,
    source_holding_id: Option<String>,
}

async fn sync_shared_sessions(Json(body): Json<ShareSyncBody>) -> impl IntoResponse {
    let mut group = match shared::load_group(&body.group_id) {
        Ok(group) => group,
        Err(e) => return api_error(StatusCode::NOT_FOUND, e).into_response(),
    };
    let source_id = match resolve_share_sync_source(&mut group, body.source_holding_id) {
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

    let result = shared::push_sync(&body.group_id, &source_id);
    match result {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ShareRemoveQuery {
    delete_provider_sessions: Option<bool>,
}

async fn remove_shared_session(
    Path(group_id): Path<String>,
    Query(q): Query<ShareRemoveQuery>,
) -> impl IntoResponse {
    match shared::delete_group(&group_id, q.delete_provider_sessions.unwrap_or(false)) {
        Ok(()) => ApiResponse::success("removed").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct ShareRenameBody {
    title: String,
}

async fn rename_shared_session(
    Path(group_id): Path<String>,
    Json(body): Json<ShareRenameBody>,
) -> impl IntoResponse {
    match shared::rename_group(&group_id, &body.title) {
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

async fn manager_preview(Json(body): Json<ManagerPreviewBody>) -> impl IntoResponse {
    let filter = crate::core::manager::ManagerFilter {
        providers: body.providers,
        older_than_days: body.older_than_days,
        older_than_ms: body.older_than_ms,
        larger_than_mb: body.larger_than_mb,
        larger_than_bytes: body.larger_than_bytes,
        smaller_than_bytes: body.smaller_than_bytes,
        workspace: body.workspace,
        sort: body.sort,
        limit: body.limit,
    };
    match crate::core::manager::preview(&filter) {
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
    let result = crate::core::manager::clean(&body.items);
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
    let result = crate::core::manager::backup(&body.items, std::path::Path::new(&output_dir));
    logging::info(
        "manager_backup",
        format!(
            "success={} failed={} output_dir={}",
            result.success, result.failed, output_dir
        ),
    );
    ApiResponse::success(result).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{
        CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
        EventSource, MappingDisposition, ProviderSessionRef, SessionContext, SessionEvent,
        SessionEventKind, SessionIdentity, SessionProvenance,
    };
    use crate::hooks::model::{RuntimeSession, RuntimeSessionId, RuntimeSessionStatus};
    use crate::hooks::protocol::{HookIngestRequest, HookRuntimeEndpoint};
    use crate::storage::session_state::ResolvedLocalSessionState;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use tempfile::Builder;
    use tower::util::ServiceExt;

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    struct ConfigTestHome {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl ConfigTestHome {
        fn new(path: &Path) -> Self {
            let guard = test_guard();
            crate::config::set_test_home_dir(path.to_path_buf());
            Self { _guard: guard }
        }
    }

    impl Drop for ConfigTestHome {
        fn drop(&mut self) {
            crate::config::reset_test_home_dir();
        }
    }

    async fn read_json(app: Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, value)
    }

    fn runtime_session_for_payload(runtime_id: &str) -> RuntimeSession {
        RuntimeSession {
            runtime_id: RuntimeSessionId::new(runtime_id),
            provider: "claude".to_string(),
            provider_session_id: Some("session-1".to_string()),
            run_id: None,
            cwd: None,
            pid: None,
            parent_pid: None,
            pid_start_time: None,
            tty: None,
            terminal_vars: BTreeMap::new(),
            process_ancestry: Vec::new(),
            correlation: None,
            model: None,
            session_title: None,
            transcript_path: None,
            workspace_roots: Vec::new(),
            last_user_prompt: None,
            last_assistant_message: None,
            last_tool_result: None,
            last_error: None,
            stop_reason: None,
            compact_count: 0,
            tool_call_count: 0,
            failed_tool_count: 0,
            permission_request_count: 0,
            question_count: 0,
            status: RuntimeSessionStatus::Running,
            current_tool: None,
            pending_permission: None,
            pending_question: None,
            recent_activity: Vec::new(),
            subagents: BTreeMap::new(),
            last_event_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn shared_holding(id: &str, provider: &str, session_id: &str) -> shared::Holding {
        shared::Holding {
            id: id.to_string(),
            provider: provider.to_string(),
            session_id: session_id.to_string(),
            target_dir: None,
            created_at: 1,
            last_active_at: None,
            last_sync_at: None,
            last_sync_from: None,
            last_error: None,
        }
    }

    #[test]
    fn sync_safety_blocks_active_target_runtime() {
        let group = shared::SharedGroup {
            id: "group-1".to_string(),
            title: "group".to_string(),
            source_provider: Some("codex".to_string()),
            created_at: 1,
            updated_at: 1,
            holdings: vec![
                shared_holding("source", "codex", "source-session"),
                shared_holding("target", "claude", "session-1"),
            ],
        };
        let snapshot = vec![runtime_session_for_payload("runtime-1")];

        let blocked = blocked_sync_targets_from_snapshot(&group, "source", &snapshot);

        assert_eq!(blocked.len(), 1);
        assert!(blocked[0].contains("claude:session-1"));
    }

    #[test]
    fn sync_safety_allows_active_source_runtime() {
        let group = shared::SharedGroup {
            id: "group-1".to_string(),
            title: "group".to_string(),
            source_provider: Some("claude".to_string()),
            created_at: 1,
            updated_at: 1,
            holdings: vec![
                shared_holding("source", "claude", "session-1"),
                shared_holding("target", "codex", "target-session"),
            ],
        };
        let snapshot = vec![runtime_session_for_payload("runtime-1")];

        let blocked = blocked_sync_targets_from_snapshot(&group, "source", &snapshot);

        assert!(blocked.is_empty());
    }

    struct ArchiveFixture {
        archive_ref: String,
        group_dir: std::path::PathBuf,
    }

    impl Drop for ArchiveFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.group_dir);
        }
    }

    fn write_api_retrieve_archive_fixture() -> ArchiveFixture {
        let now = Utc::now();
        let group = format!("api-retrieve-{}", uuid::Uuid::new_v4());
        let archive_dir = config::memorph_dir()
            .unwrap()
            .join("compression_archives")
            .join(&group);
        std::fs::create_dir_all(&archive_dir).unwrap();

        let source = EventSource {
            provider_id: "claude".to_string(),
            original_id: None,
            original_role: Some("user".to_string()),
            phase: None,
        };
        let metadata = EventMetadata {
            source,
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: BTreeMap::new(),
        };
        let archive = core::compression::CompressionArchive {
            version: 1,
            created_at: now,
            canonical_id: group.clone(),
            source_provider_id: "claude".to_string(),
            target_provider_id: "codex".to_string(),
            summary_event_id: "summary-event".to_string(),
            source_event_ids: vec!["needle-event".to_string(), "other-event".to_string()],
            events: vec![
                SessionEvent {
                    id: "needle-event".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: now,
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "needle detail from archived original event".to_string(),
                    }],
                    metadata: metadata.clone(),
                },
                SessionEvent {
                    id: "other-event".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::Assistant,
                    timestamp: now,
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "unrelated archived original event".to_string(),
                    }],
                    metadata,
                },
            ],
        };
        std::fs::write(
            archive_dir.join("archive.json"),
            serde_json::to_string_pretty(&archive).unwrap(),
        )
        .unwrap();

        ArchiveFixture {
            archive_ref: format!("memorph-archive://{}/archive.json", group),
            group_dir: archive_dir,
        }
    }

    #[tokio::test]
    async fn compression_plan_route_returns_candidates_from_file() {
        let now = Utc::now();
        let session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "api-dry-run-file".to_string(),
                source_title: Some("API Dry Run File".to_string()),
            },
            provenance: SessionProvenance {
                imported_at: now,
                imported_by: None,
                primary_source: ProviderSessionRef {
                    provider_id: "claude".to_string(),
                    session_id: "api-dry-run-file".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext::default(),
            events: vec![
                SessionEvent {
                    id: "old-user".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: now,
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "historical context ".repeat(80),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: "claude".to_string(),
                            original_id: Some("old-user".to_string()),
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
                SessionEvent {
                    id: "recent-user".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: now,
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "latest active request".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: "claude".to_string(),
                            original_id: Some("recent-user".to_string()),
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };
        let mut file = Builder::new().suffix(".json").tempfile().unwrap();
        write!(file, "{}", serde_json::to_string(&session).unwrap()).unwrap();

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/compression/plan")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "source_provider_id": "claude",
                    "target_provider_id": "codex",
                    "file": file.path().to_string_lossy(),
                    "policy": {
                        "protect_recent_message_events": 1,
                        "min_candidate_bytes": 16,
                        "min_savings_ratio_percent": 20,
                        "mode": "plan_only"
                    }
                }))
                .unwrap(),
            ))
            .unwrap();

        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["dry_run"], true);
        assert_eq!(value["data"]["candidates"][0]["event_ids"][0], "old-user");
        assert_eq!(
            value["data"]["candidates"][0]["reason"],
            "historical_context"
        );
        assert!(
            value["data"]["candidates"][0]["estimated_bytes_saved"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(value["data"]["candidates"][0]["risk"], "medium");
        assert!(
            value["data"]["skipped"]
                .as_array()
                .unwrap()
                .iter()
                .any(|skipped| skipped["event_id"] == "recent-user"
                    && skipped["reason"] == "protected_recent_message")
        );
    }

    #[test]
    fn session_detail_payload_serializes_hook_runtime_sessions() {
        let payload = SessionDetailPayload {
            view: core::SessionDetailView {
                provider_id: "claude".to_string(),
                provider_name: "claude".to_string(),
                session_id: "session-1".to_string(),
                canonical_id: "canonical-1".to_string(),
                title: Some("Session".to_string()),
                native_title: None,
                display_title: None,
                workspace_dir: Some("/tmp/project".to_string()),
                created_at: None,
                last_active_at: None,
                source_path: None,
                resume_command: None,
                local_state: ResolvedLocalSessionState::default(),
                event_count: 0,
                message_count: 0,
                artifact_count: 0,
                hook_runtime_summary: Some(hooks::augmentation::HookRuntimeSummary {
                    linked_sessions: 1,
                    waiting_sessions: 0,
                    status: hooks::model::RuntimeSessionStatus::Running,
                    current_tool_name: Some("Bash".to_string()),
                    has_pending_permission: false,
                    has_pending_question: false,
                    last_event_at: None,
                    matched_by: Some("provider_session_id".to_string()),
                    confidence: Some(hooks::augmentation::HookLinkConfidence::High),
                }),
                hook_diagnosis: Some(hooks::augmentation::SessionHookDiagnosis {
                    kind: hooks::augmentation::SessionHookDiagnosisKind::Linked,
                    provider_status: hooks::model::HookHealthStatus::InstalledOk,
                    linked_runtime_sessions: 1,
                    provider_runtime_sessions: 1,
                    matched_by: Some("provider_session_id".to_string()),
                    confidence: Some(hooks::augmentation::HookLinkConfidence::High),
                    last_event_at: None,
                    message: "Hook runtime is linked directly to this session.".to_string(),
                    actions: Vec::new(),
                }),
                hook_runtime_sessions: vec![runtime_session_for_payload(
                    "claude:session:session-1",
                )],
                events: Vec::new(),
                artifacts: Vec::new(),
            },
            events_offset: 0,
            events_limit: Some(50),
            returned_event_count: 0,
            has_more_events: false,
            hook_runtime_sessions: vec![runtime_session_for_payload("claude:session:session-1")],
        };

        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value["hook_runtime_sessions"].as_array().unwrap().len(), 1);
        assert_eq!(
            value["view"]["hook_runtime_summary"]["matched_by"],
            "provider_session_id"
        );
        assert_eq!(value["view"]["hook_runtime_summary"]["confidence"], "high");
        assert_eq!(value["view"]["hook_diagnosis"]["kind"], "linked");
        assert_eq!(
            value["hook_runtime_sessions"][0]["provider_session_id"],
            "session-1"
        );
    }

    #[test]
    fn shared_holding_payload_serializes_hook_runtime_sessions() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        crate::hooks::store::set_test_store_root(dir.path().to_path_buf());
        crate::hooks::server::reset_for_tests();
        let endpoint = HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "test-token".to_string(),
            pid: 1,
            started_at: Utc::now(),
        };
        crate::hooks::server::set_runtime_endpoint_for_tests(endpoint.clone());

        let request = HookIngestRequest::new(
            "generic",
            "tool_started",
            serde_json::json!({
                "session_id": "session-1",
                "cwd": "/tmp/project",
                "tool": {"name": "Bash", "input": {"command": "cargo check"}}
            }),
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (status, value) = read_json(
                router(),
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hooks/ingest")
                    .header("content-type", "application/json")
                    .header("x-memorph-hook-token", endpoint.token)
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(value["ok"], true);
        });

        let payload = shared_holding_payload(shared::Holding {
            id: "holding-1".to_string(),
            provider: "generic".to_string(),
            session_id: "session-1".to_string(),
            target_dir: Some("/tmp/project".to_string()),
            created_at: 1,
            last_active_at: None,
            last_sync_at: None,
            last_sync_from: None,
            last_error: None,
        });

        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value["provider"], "generic");
        assert_eq!(value["session_id"], "session-1");
        assert_eq!(value["hook_runtime_summary"]["current_tool_name"], "Bash");
        assert_eq!(
            value["hook_runtime_summary"]["matched_by"],
            "provider_session_id"
        );
        assert_eq!(value["hook_runtime_summary"]["confidence"], "high");
        assert_eq!(value["hook_diagnosis"]["kind"], "linked");
        assert_eq!(value["hook_runtime_sessions"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn compression_apply_route_rejects_ambiguous_source_without_writing_archive() {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/compression/apply")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "source_provider_id": "claude",
                    "target_provider_id": "codex",
                    "session_id": "s1",
                    "file": "session.json",
                    "format": "json"
                }))
                .unwrap(),
            ))
            .unwrap();

        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(value["ok"], false);
        assert!(
            value["error"]
                .as_str()
                .unwrap()
                .contains("Use either session_id or file")
        );
    }

    #[tokio::test]
    async fn compression_retrieve_route_rejects_invalid_archive_ref() {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/compression/retrieve")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "archive_ref": "not-an-archive-ref"
                }))
                .unwrap(),
            ))
            .unwrap();

        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(value["ok"], false);
        assert!(
            value["error"]
                .as_str()
                .unwrap()
                .contains("Unsupported compression archive ref")
        );
    }

    #[tokio::test]
    async fn compression_retrieve_route_returns_query_matches_from_archive() {
        let fixture = write_api_retrieve_archive_fixture();
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/compression/retrieve")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "archive_ref": fixture.archive_ref.clone(),
                    "query": "needle",
                    "max_results": 5
                }))
                .unwrap(),
            ))
            .unwrap();

        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["archive_ref"], fixture.archive_ref);
        assert_eq!(value["data"]["retrieval_mode"], "query_matches");
        assert!(
            value["data"]["recommended_next_action"]
                .as_str()
                .unwrap()
                .contains("partial retrieval")
        );
        assert_eq!(value["data"]["source_event_count"], 2);
        assert_eq!(
            value["data"]["returned_event_ids"],
            serde_json::json!(["needle-event"])
        );
        assert_eq!(value["data"]["returned_event_count"], 1);
        assert_eq!(value["data"]["omitted_event_count"], 1);
        assert_eq!(value["data"]["events"][0]["id"], "needle-event");
        assert_eq!(value["data"]["matches"][0]["event_id"], "needle-event");
        assert!(
            value["data"]["matches"][0]["snippets"][0]
                .as_str()
                .unwrap()
                .contains("needle detail")
        );
    }

    #[tokio::test]
    async fn compression_tool_spec_route_returns_retrieval_contract() {
        let request = Request::builder()
            .uri("/api/v1/compression/tool-spec")
            .body(Body::empty())
            .unwrap();

        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);
        assert_eq!(
            value["data"]["name"],
            "memorph_retrieve_compression_archive"
        );
        assert_eq!(value["data"]["api"]["path"], "/api/v1/compression/retrieve");
        assert_eq!(
            value["data"]["input_schema"]["required"],
            serde_json::json!(["archive_ref"])
        );
        assert!(
            value["data"]["usage_rules"]
                .as_array()
                .unwrap()
                .iter()
                .any(|rule| rule.as_str().unwrap().contains("Prefer query retrieval"))
        );
        assert!(
            value["data"]["usage_rules"]
                .as_array()
                .unwrap()
                .iter()
                .any(|rule| rule.as_str().unwrap().contains("exact phrase matches"))
        );
    }

    #[tokio::test]
    async fn compression_instructions_route_returns_archive_specific_examples() {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/compression/instructions")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "archive_ref": "memorph-archive://group/archive.json.gz"
                }))
                .unwrap(),
            ))
            .unwrap();

        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);
        assert_eq!(
            value["data"]["archive_ref"],
            "memorph-archive://group/archive.json.gz"
        );
        assert!(
            value["data"]["query_first_cli"]
                .as_str()
                .unwrap()
                .contains("--query <terms> --max-results 5")
        );
        assert_eq!(
            value["data"]["api_query_body"]["archive_ref"],
            "memorph-archive://group/archive.json.gz"
        );
        assert!(
            value["data"]["suggested_steps"]
                .as_array()
                .unwrap()
                .iter()
                .any(|step| step.as_str().unwrap().contains("broader term coverage"))
        );
    }

    #[tokio::test]
    async fn compression_instructions_route_rejects_invalid_archive_ref() {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/compression/instructions")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "archive_ref": "not-an-archive-ref"
                }))
                .unwrap(),
            ))
            .unwrap();

        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(value["ok"], false);
        assert!(
            value["error"]
                .as_str()
                .unwrap()
                .contains("Unsupported compression archive ref")
        );
    }

    #[tokio::test]
    async fn settings_route_lists_codex_repair_setting() {
        let request = Request::builder()
            .uri("/api/v1/providers/codex/settings")
            .body(Body::empty())
            .unwrap();
        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);

        let settings = value["data"]["settings"].as_array().unwrap();
        assert!(
            settings
                .iter()
                .any(|setting| setting["id"] == "repair_workspace_sessions")
        );
    }
    #[tokio::test]
    async fn settings_route_lists_codeisland_gap_provider_hook_actions() {
        let request = Request::builder()
            .uri("/api/v1/providers/qoder/settings")
            .body(Body::empty())
            .unwrap();
        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);

        let settings = value["data"]["settings"].as_array().unwrap();
        for action in [
            "install_hook",
            "verify_hook",
            "repair_hook",
            "uninstall_hook",
        ] {
            assert!(
                settings.iter().any(|setting| setting["id"] == action),
                "missing {action}"
            );
        }
    }

    #[tokio::test]
    async fn provider_settings_route_accepts_provider_aliases() {
        let canonical_request = Request::builder()
            .uri("/api/v1/providers/droid/settings")
            .body(Body::empty())
            .unwrap();
        let alias_request = Request::builder()
            .uri("/api/v1/providers/factory/settings")
            .body(Body::empty())
            .unwrap();

        let (canonical_status, canonical_value) = read_json(router(), canonical_request).await;
        let (alias_status, alias_value) = read_json(router(), alias_request).await;

        assert_eq!(canonical_status, StatusCode::OK);
        assert_eq!(alias_status, StatusCode::OK);
        assert_eq!(
            canonical_value["data"]["settings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|setting| setting["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            alias_value["data"]["settings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|setting| setting["id"].as_str().unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn provider_setting_update_syncs_legacy_settings_payload() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ConfigTestHome::new(dir.path());

        let update_request = Request::builder()
            .method("PUT")
            .uri("/api/v1/providers/opencode/settings/show_subagents")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({ "value": true })).unwrap(),
            ))
            .unwrap();
        let (update_status, update_value) = read_json(router(), update_request).await;

        assert_eq!(update_status, StatusCode::OK);
        assert_eq!(update_value["data"]["value"], true);

        let settings_request = Request::builder()
            .uri("/api/v1/settings")
            .body(Body::empty())
            .unwrap();
        let (settings_status, settings_value) = read_json(router(), settings_request).await;

        assert_eq!(settings_status, StatusCode::OK);
        assert_eq!(settings_value["data"]["show_opencode_subagents"], true);
    }

    #[tokio::test]
    async fn legacy_settings_update_syncs_provider_setting_payload() {
        let dir = tempfile::tempdir().unwrap();
        let _home = ConfigTestHome::new(dir.path());

        let update_request = Request::builder()
            .method("PUT")
            .uri("/api/v1/settings")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "sessions_per_provider": 12,
                    "language": "en",
                    "show_opencode_subagents": true,
                    "sort_providers_by_session_count": true,
                    "default_backup_dir": "./backups",
                    "logging": {
                        "max_size_bytes": 5 * 1024 * 1024,
                        "retention_days": null
                    },
                    "home_buttons": {
                        "switch": true,
                        "view": true,
                        "export": true,
                        "share": false,
                        "delete": false
                    },
                    "agent_order": [],
                    "primary_agents": []
                }))
                .unwrap(),
            ))
            .unwrap();
        let (update_status, update_value) = read_json(router(), update_request).await;

        assert_eq!(update_status, StatusCode::OK);
        assert_eq!(update_value["data"]["show_opencode_subagents"], true);

        let setting_request = Request::builder()
            .uri("/api/v1/providers/opencode/settings/show_subagents")
            .body(Body::empty())
            .unwrap();
        let (setting_status, setting_value) = read_json(router(), setting_request).await;

        assert_eq!(setting_status, StatusCode::OK);
        assert_eq!(setting_value["data"]["id"], "show_subagents");
        assert_eq!(setting_value["data"]["value"], true);
    }

    #[tokio::test]
    async fn agents_route_exposes_settings_field() {
        let request = Request::builder()
            .uri("/api/v1/agents")
            .body(Body::empty())
            .unwrap();
        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);

        let providers = value["data"]["providers"].as_array().unwrap();
        let codex = providers
            .iter()
            .find(|provider| provider["provider_id"] == "codex")
            .expect("missing codex agent entry");
        assert!(codex.get("settings").is_some());
        assert!(codex.get("environment").is_some());
        assert!(codex.get("features").is_none());
    }
    #[tokio::test]
    async fn agents_route_exposes_all_hook_profile_providers() {
        let request = Request::builder()
            .uri("/api/v1/agents")
            .body(Body::empty())
            .unwrap();
        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        let providers = value["data"]["providers"].as_array().unwrap();
        for descriptor in crate::hooks::registry::all() {
            let entry = providers
                .iter()
                .find(|provider| provider["provider_id"] == descriptor.provider())
                .unwrap_or_else(|| panic!("missing agent entry for {}", descriptor.provider()));
            assert_eq!(entry["hook"]["provider"], descriptor.provider());
            assert_eq!(entry["hook_profile"]["provider"], descriptor.provider());
            assert_eq!(
                entry["hook_required_events"].as_array().unwrap().len(),
                descriptor.required_events.len()
            );
            assert!(entry["hook_profile"]["events"].as_array().unwrap().len() > 0);
            assert!(
                entry["settings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|setting| setting["id"] == "install_hook")
            );
        }
    }

    #[tokio::test]
    async fn agents_route_keeps_environment_block_and_flat_fields_in_sync() {
        let request = Request::builder()
            .uri("/api/v1/agents")
            .body(Body::empty())
            .unwrap();
        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        let providers = value["data"]["providers"].as_array().unwrap();
        let codex = providers
            .iter()
            .find(|provider| provider["provider_id"] == "codex")
            .expect("missing codex agent entry");

        assert_eq!(codex["environment"]["installed"], codex["installed"]);
        assert_eq!(codex["environment"]["config_path"], codex["config_path"]);
        assert_eq!(
            codex["environment"]["install_method"],
            codex["install_method"]
        );
    }

    #[tokio::test]
    async fn agent_detail_route_returns_single_provider_entry() {
        let request = Request::builder()
            .uri("/api/v1/agents/codex")
            .body(Body::empty())
            .unwrap();
        let (status, value) = read_json(router(), request).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["provider_id"], "codex");
        assert!(value["data"]["settings"].is_array());
        assert!(value["data"]["environment"].is_object());
        assert_eq!(
            value["data"]["environment"]["config_path"],
            value["data"]["config_path"]
        );
    }

    #[tokio::test]
    async fn agent_detect_route_matches_detail_route_for_provider_entry() {
        let detail_request = Request::builder()
            .uri("/api/v1/agents/codex")
            .body(Body::empty())
            .unwrap();
        let detect_request = Request::builder()
            .method("POST")
            .uri("/api/v1/agents/codex/detect")
            .body(Body::empty())
            .unwrap();

        let (detail_status, detail_value) = read_json(router(), detail_request).await;
        let (detect_status, detect_value) = read_json(router(), detect_request).await;

        assert_eq!(detail_status, StatusCode::OK);
        assert_eq!(detect_status, StatusCode::OK);
        assert_eq!(
            detail_value["data"]["provider_id"],
            detect_value["data"]["provider_id"]
        );
        assert_eq!(
            detail_value["data"]["environment"],
            detect_value["data"]["environment"]
        );
    }

    #[tokio::test]
    async fn legacy_features_route_matches_settings_route_for_codex_repair() {
        let setting_request = Request::builder()
            .uri("/api/v1/providers/codex/settings/repair_workspace_sessions")
            .body(Body::empty())
            .unwrap();
        let feature_request = Request::builder()
            .uri("/api/v1/providers/codex/features/repair_workspace_sessions")
            .body(Body::empty())
            .unwrap();

        let (setting_status, setting_value) = read_json(router(), setting_request).await;
        let (feature_status, feature_value) = read_json(router(), feature_request).await;

        assert_eq!(setting_status, StatusCode::OK);
        assert_eq!(feature_status, StatusCode::OK);
        assert_eq!(setting_value["data"]["id"], feature_value["data"]["id"]);
        assert_eq!(setting_value["data"]["kind"], feature_value["data"]["kind"]);
        assert_eq!(
            setting_value["data"]["scope"],
            feature_value["data"]["scope"]
        );
    }

    #[tokio::test]
    async fn legacy_features_list_route_matches_settings_route_for_codex() {
        let setting_request = Request::builder()
            .uri("/api/v1/providers/codex/settings")
            .body(Body::empty())
            .unwrap();
        let feature_request = Request::builder()
            .uri("/api/v1/providers/codex/features")
            .body(Body::empty())
            .unwrap();

        let (setting_status, setting_value) = read_json(router(), setting_request).await;
        let (feature_status, feature_value) = read_json(router(), feature_request).await;

        assert_eq!(setting_status, StatusCode::OK);
        assert_eq!(feature_status, StatusCode::OK);
        assert_eq!(
            setting_value["data"]["settings"][0]["id"],
            feature_value["data"]["features"][0]["id"]
        );
        assert_eq!(
            setting_value["data"]["settings"][0]["kind"],
            feature_value["data"]["features"][0]["kind"]
        );
    }

    #[tokio::test]
    async fn legacy_controls_route_matches_settings_route_for_codex() {
        let settings_request = Request::builder()
            .uri("/api/v1/providers/codex/settings")
            .body(Body::empty())
            .unwrap();
        let controls_request = Request::builder()
            .uri("/api/v1/providers/codex/controls")
            .body(Body::empty())
            .unwrap();

        let (settings_status, settings_value) = read_json(router(), settings_request).await;
        let (controls_status, controls_value) = read_json(router(), controls_request).await;

        assert_eq!(settings_status, StatusCode::OK);
        assert_eq!(controls_status, StatusCode::OK);
        assert_eq!(
            settings_value["data"]["settings"][0]["id"],
            controls_value["data"]["controls"][0]["id"]
        );
        assert_eq!(
            settings_value["data"]["settings"][0]["kind"],
            controls_value["data"]["controls"][0]["kind"]
        );
    }
}
