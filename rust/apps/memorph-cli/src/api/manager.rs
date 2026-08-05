use super::*;

pub(super) async fn manager_preview(
    Json(filter): Json<memorph::core::manager::ManagerFilter>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || memorph::core::manager::preview(&filter)).await {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Debug, Serialize)]
struct ManagerQuickPreviewResult {
    items: Vec<memorph::core::manager::ManagerItem>,
    total_count: usize,
    total_size_bytes: u64,
    selected_agent_count: usize,
}

#[derive(Deserialize)]
pub(super) struct ManagerQuickQuery {
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

fn quick_filter(provider_ids: Vec<String>) -> memorph::core::manager::ManagerFilter {
    memorph::core::manager::ManagerFilter {
        providers: provider_ids,
        older_than_days: None,
        older_than_ms: None,
        larger_than_mb: None,
        larger_than_bytes: None,
        smaller_than_bytes: None,
        workspace: None,
        search: None,
        sort: Some("recent".to_string()),
        offset: None,
        limit: Some(MANAGER_QUICK_LIMIT),
    }
}

pub(super) async fn manager_quick_preview(
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

    match memorph::runtime::run_blocking(move || memorph::core::manager::preview(&filter)).await {
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

pub(super) async fn manager_quick_workspaces(
    axum::extract::Query(query): axum::extract::Query<ManagerQuickQuery>,
) -> impl IntoResponse {
    let provider_ids = match resolve_quick_provider_ids(&query.providers).await {
        Ok(ids) => ids,
        Err(e) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };

    if provider_ids.is_empty() {
        return ApiResponse::success(memorph::core::manager::ManagerWorkspacesResult {
            items: Vec::new(),
            total_count: 0,
            total_size_bytes: 0,
        })
        .into_response();
    }

    let filter = quick_filter(provider_ids);
    match memorph::runtime::run_blocking(move || memorph::core::manager::workspaces(&filter)).await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn manager_stats(
    Json(filter): Json<memorph::core::manager::ManagerFilter>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || memorph::core::manager::stats(&filter)).await {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct ManagerItemsBody {
    items: Vec<memorph::core::manager::ManagerItem>,
    output_dir: Option<String>,
}

pub(super) async fn manager_clean(Json(body): Json<ManagerItemsBody>) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || {
        Ok(memorph::core::manager::clean(
            &body.items,
            ActivityActor::Api,
        ))
    })
    .await
    {
        Ok(result) => {
            logging::info(
                "manager_clean",
                format!(
                    "success={} failed={} freed_bytes={}",
                    result.success, result.failed, result.freed_bytes
                ),
            );
            ApiResponse::success(result).into_response()
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn manager_backup(Json(body): Json<ManagerItemsBody>) -> impl IntoResponse {
    let output_dir = body.output_dir.unwrap_or_else(|| "./backups".to_string());
    let resolved_output_dir = resolve_backup_output_dir(&output_dir, None);
    let logged_output_dir = resolved_output_dir.clone();
    match memorph::runtime::run_blocking(move || {
        Ok(memorph::core::manager::backup(
            &body.items,
            &resolved_output_dir,
            ActivityActor::Api,
        ))
    })
    .await
    {
        Ok(result) => {
            logging::info(
                "manager_backup",
                format!(
                    "success={} failed={} output_dir={}",
                    result.success,
                    result.failed,
                    logged_output_dir.display()
                ),
            );
            ApiResponse::success(result).into_response()
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn manager_workspaces(
    Json(filter): Json<memorph::core::manager::ManagerFilter>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || memorph::core::manager::workspaces(&filter)).await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct ManagerWorkspaceBody {
    provider_id: String,
    workspace: String,
    output_dir: Option<String>,
}

pub(super) async fn manager_clean_workspace(
    Json(body): Json<ManagerWorkspaceBody>,
) -> impl IntoResponse {
    let provider_id = body.provider_id.clone();
    let workspace = body.workspace.clone();
    match memorph::runtime::run_blocking(move || {
        Ok(memorph::core::manager::clean_workspace(
            &body.provider_id,
            &body.workspace,
            ActivityActor::Api,
        ))
    })
    .await
    {
        Ok(result) => {
            logging::info(
                "manager_clean_workspace",
                format!(
                    "provider={} workspace={} success={} failed={} freed_bytes={}",
                    provider_id, workspace, result.success, result.failed, result.freed_bytes
                ),
            );
            ApiResponse::success(result).into_response()
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn manager_backup_workspace(
    Json(body): Json<ManagerWorkspaceBody>,
) -> impl IntoResponse {
    let output_dir = body.output_dir.unwrap_or_else(|| "./backups".to_string());
    let resolved_output_dir = resolve_backup_output_dir(&output_dir, Some(&body.workspace));
    let provider_id = body.provider_id.clone();
    let workspace = body.workspace.clone();
    let logged_output_dir = resolved_output_dir.clone();
    match memorph::runtime::run_blocking(move || {
        Ok(memorph::core::manager::backup_workspace(
            &body.provider_id,
            &body.workspace,
            &resolved_output_dir,
            ActivityActor::Api,
        ))
    })
    .await
    {
        Ok(result) => {
            logging::info(
                "manager_backup_workspace",
                format!(
                    "provider={} workspace={} success={} failed={} output_dir={}",
                    provider_id,
                    workspace,
                    result.success,
                    result.failed,
                    logged_output_dir.display()
                ),
            );
            ApiResponse::success(result).into_response()
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}
