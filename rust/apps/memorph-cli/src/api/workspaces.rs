use super::*;

pub(super) async fn list_workspaces() -> impl IntoResponse {
    match config::known_workspaces() {
        Ok(workspaces) => ApiResponse::success(workspaces).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct WorkspacesWithSessionsQuery {
    q: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
}

pub(super) async fn list_workspaces_with_sessions(
    Query(query): Query<WorkspacesWithSessionsQuery>,
) -> impl IntoResponse {
    let options = memorph::core::manager::WorkspaceWithSessionsOptions {
        search: query
            .q
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        page: query.page.unwrap_or(1),
        page_size: query.page_size.unwrap_or(5),
    };

    match memorph::runtime::run_blocking(move || {
        memorph::core::manager::workspaces_with_sessions(&options)
    })
    .await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct WorkspaceHistoryBody {
    workspace: String,
}

pub(super) async fn remove_workspace_history(
    Json(body): Json<WorkspaceHistoryBody>,
) -> impl IntoResponse {
    match config::remove_workspace_history(&body.workspace) {
        Ok(workspaces) => ApiResponse::success(workspaces).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct WorkspaceQuery {
    workspace: String,
}

pub(super) async fn get_workspace_providers(Query(q): Query<WorkspaceQuery>) -> impl IntoResponse {
    match config::workspace_providers(&q.workspace) {
        Ok(providers) => ApiResponse::success(providers).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct WorkspaceProvidersBody {
    workspace: String,
    providers: Vec<String>,
}

pub(super) async fn update_workspace_providers(
    Json(body): Json<WorkspaceProvidersBody>,
) -> impl IntoResponse {
    match config::set_workspace_providers(&body.workspace, body.providers) {
        Ok(()) => match config::workspace_providers(&body.workspace) {
            Ok(providers) => ApiResponse::success(providers).into_response(),
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
