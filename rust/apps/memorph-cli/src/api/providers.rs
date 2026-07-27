use super::*;

#[derive(Deserialize)]
pub(super) struct ProviderSettingUpdateBody {
    value: Option<Value>,
}

#[derive(Deserialize)]
pub(super) struct ProviderSettingRunBody {
    workspace: Option<String>,
}

pub(super) async fn list_agent_management() -> impl IntoResponse {
    match memorph::runtime::run_blocking(agent_management::list_agent_management_entries).await {
        Ok(providers) => ApiResponse::success(AgentManagementPayload { providers }).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn list_agent_management_summary() -> impl IntoResponse {
    match memorph::runtime::run_blocking(agent_management::list_agent_management_summaries).await {
        Ok(providers) => {
            ApiResponse::success(AgentManagementSummaryPayload { providers }).into_response()
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn get_agent_management_provider(
    Path(provider): Path<String>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || agent_management::get_agent_management_entry(&provider)).await {
        Ok(provider) => ApiResponse::success(provider).into_response(),
        Err(error) => api_error(StatusCode::NOT_FOUND, error).into_response(),
    }
}

pub(super) async fn detect_agent_management_provider(
    Path(provider): Path<String>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || agent_management::detect_agent_management_entry(&provider)).await {
        Ok(provider) => {
            invalidate_catalog_cache();
            ApiResponse::success(provider).into_response()
        }
        Err(error) => api_error(StatusCode::NOT_FOUND, error).into_response(),
    }
}

pub(super) async fn list_providers() -> impl IntoResponse {
    ApiResponse::success(provider_info_list()).into_response()
}

pub(super) async fn list_provider_hooks(Path(provider): Path<String>) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || hooks::discovery::list(&provider)).await {
        Ok(hooks) => ApiResponse::success(hooks).into_response(),
        Err(error) => api_error(StatusCode::BAD_REQUEST, error).into_response(),
    }
}

pub(super) async fn get_provider_catalog(Query(q): Query<CatalogQuery>) -> impl IntoResponse {
    match build_provider_catalog_light(q.workspace.as_deref()).await {
        Ok(catalog) => ApiResponse::success(catalog).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn get_provider_catalog_active(
    Query(q): Query<CatalogQuery>,
) -> impl IntoResponse {
    match build_provider_catalog_active(q.workspace.as_deref()).await {
        Ok(catalog) => ApiResponse::success(catalog).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn update_provider_catalog(
    Json(body): Json<UpdateCatalogBody>,
) -> impl IntoResponse {
    let result = if let Some(workspace) = body.workspace.as_deref() {
        config::update_workspace_catalog_preferences(
            workspace,
            body.sort_order.workspace,
            body.hidden_state.workspace,
        )
    } else {
        config::update_agent_display_preferences(
            config::ProviderDisplayOrder {
                global: body.sort_order.global,
                workspace: body.sort_order.workspace,
            },
            config::ProviderDisplayHidden {
                global: body.hidden_state.global,
                workspace: body.hidden_state.workspace,
            },
        )
    };

    match result {
        Ok(()) => {
            invalidate_catalog_cache();
            match build_provider_catalog_light(body.workspace.as_deref()).await {
                Ok(catalog) => ApiResponse::success(catalog).into_response(),
                Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
            }
        }
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn list_provider_settings(Path(provider): Path<String>) -> impl IntoResponse {
    match provider_settings::list_provider_settings(&provider) {
        Ok(settings) => ApiResponse::success(ProviderSettingsPayload {
            provider_id: provider,
            settings,
        })
        .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn get_provider_setting(
    Path((provider, setting_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match provider_settings::get_provider_setting(&provider, &setting_id) {
        Ok(setting) => ApiResponse::success(setting).into_response(),
        Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
    }
}

pub(super) async fn update_provider_setting(
    Path((provider, setting_id)): Path<(String, String)>,
    Json(body): Json<ProviderSettingUpdateBody>,
) -> impl IntoResponse {
    match provider_settings::update_provider_setting(&provider, &setting_id, body.value) {
        Ok(setting) => ApiResponse::success(setting).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn run_provider_setting(
    Path((provider, setting_id)): Path<(String, String)>,
    Json(body): Json<ProviderSettingRunBody>,
) -> impl IntoResponse {
    match provider_settings::run_provider_setting(
        &provider,
        &setting_id,
        provider_settings::ProviderSettingContext {
            workspace: body.workspace,
            actor: ActivityActor::Api,
        },
    ) {
        Ok(output) => ApiResponse::success(output).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
