use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::{
    agent_management, config, core, logging, provider_features, provider_settings, shared,
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
            "/api/v1/compression/restore",
            post(restore_compression_archive),
        )
        .route(
            "/api/v1/compression/expand",
            post(expand_compression_session),
        )
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
struct MetaPayload {
    version: &'static str,
    providers: Vec<ProviderInfo>,
    selected_workspace: Option<String>,
    workspaces: Vec<config::WorkspaceEntry>,
    settings: SettingsPayload,
}

#[derive(Debug, Serialize)]
struct SessionDetailPayload {
    view: core::SessionDetailView,
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
struct CratesIoPayload {
    #[serde(rename = "crate")]
    crate_info: CratesIoCrateInfo,
}

#[derive(Debug, Deserialize)]
struct CratesIoCrateInfo {
    max_version: String,
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
    Cargo,
    DesktopApp,
}

impl InstallSource {
    fn slug(self) -> &'static str {
        match self {
            InstallSource::Npm => "npm",
            InstallSource::PythonPip => "pip",
            InstallSource::PythonPipx => "pipx",
            InstallSource::PythonUvTool => "uv-tool",
            InstallSource::Cargo => "cargo",
            InstallSource::DesktopApp => "desktop-app",
        }
    }

    fn label(self) -> &'static str {
        match self {
            InstallSource::Npm => "npm",
            InstallSource::PythonPip => "PyPI/pip",
            InstallSource::PythonPipx => "PyPI/pipx",
            InstallSource::PythonUvTool => "PyPI/uv tool",
            InstallSource::Cargo => "Cargo",
            InstallSource::DesktopApp => "GitHub Desktop App",
        }
    }

    fn release_url(self) -> &'static str {
        match self {
            InstallSource::Npm => "https://www.npmjs.com/package/memorph",
            InstallSource::PythonPip
            | InstallSource::PythonPipx
            | InstallSource::PythonUvTool => "https://pypi.org/project/memorph/",
            InstallSource::Cargo => "https://crates.io/crates/memorph",
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
            InstallSource::Cargo => "cargo install memorph --force".to_string(),
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
            "cargo" | "crates" | "crates.io" => return Some(InstallSource::Cargo),
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
    if path.contains("/.cargo/bin/") {
        return Some(InstallSource::Cargo);
    }
    if path.contains("/target/debug/") || path.contains("/target/release/") {
        return Some(InstallSource::Cargo);
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
        InstallSource::Cargo => {
            let payload = client
                .get("https://crates.io/api/v1/crates/memorph")
                .send()
                .await?
                .error_for_status()?
                .json::<CratesIoPayload>()
                .await?;
            Ok(payload.crate_info.max_version)
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
             - cargo install memorph --force"
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

async fn get_meta() -> impl IntoResponse {
    match (
        settings_payload(),
        config::selected_workspace(),
        config::known_workspaces(),
    ) {
        (Ok(settings), Ok(selected_workspace), Ok(workspaces)) => {
            ApiResponse::success(MetaPayload {
                version: env!("CARGO_PKG_VERSION"),
                providers: provider_info_list(),
                selected_workspace,
                workspaces,
                settings,
            })
            .into_response()
        }
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
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
        return api_error(StatusCode::BAD_REQUEST, "Only http(s) URLs are supported.").into_response();
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

async fn get_session(Path((provider, session_id)): Path<(String, String)>) -> impl IntoResponse {
    match core::get_session_detail_view(&provider, &session_id) {
        Ok(view) => {
            if let Some(project_dir) = view.workspace_dir.as_deref() {
                let _ = config::remember_workspace(std::path::Path::new(project_dir));
            }
            ApiResponse::success(SessionDetailPayload { view }).into_response()
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

#[derive(Deserialize)]
struct RestoreCompressionArchiveBody {
    archive_ref: String,
    output_prefix: Option<String>,
    #[serde(default = "default_format")]
    format: String,
}

#[derive(Deserialize)]
struct ExpandCompressionSessionBody {
    file: String,
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
}

async fn switch_session(Json(body): Json<SwitchBody>) -> impl IntoResponse {
    let params = core::SwitchParams {
        from: body.from,
        to: body.to,
        session_id: body.session_id,
        to_dir: body.to_dir,
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
                ApiResponse::success(group).into_response()
            }
            Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
        },
        None => match shared::list_groups() {
            Ok(mut groups) => {
                for group in &mut groups {
                    let _ = shared::refresh_active_times(group);
                }
                ApiResponse::success(groups).into_response()
            }
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
    }
}

#[derive(Deserialize)]
struct ShareSyncBody {
    group_id: String,
    source_holding_id: Option<String>,
}

async fn sync_shared_sessions(Json(body): Json<ShareSyncBody>) -> impl IntoResponse {
    let result = if let Some(source_id) = body.source_holding_id {
        shared::push_sync(&body.group_id, &source_id)
    } else {
        shared::sync_to_latest(&body.group_id)
    };
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
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::util::ServiceExt;

    async fn read_json(app: Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, value)
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
        assert!(settings
            .iter()
            .any(|setting| setting["id"] == "repair_workspace_sessions"));
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
