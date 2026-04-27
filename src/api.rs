use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{config, core};

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
    (
        status,
        Json(ApiResponse::<()> {
            ok: false,
            data: None,
            error: Some(msg.to_string()),
        }),
    )
}

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/sessions", get(list_sessions))
        .route("/api/v1/sessions/{provider}/{session_id}", get(get_session))
        .route(
            "/api/v1/sessions/{provider}/{session_id}",
            delete(delete_session),
        )
        .route(
            "/api/v1/sessions/{provider}/{session_id}",
            patch(rename_session),
        )
        .route("/api/v1/export", post(export_session))
        .route("/api/v1/import", post(import_session))
        .route("/api/v1/switch", post(switch_session))
        .route("/api/v1/find", get(find_sessions))
        .route("/api/v1/workspaces", get(list_workspaces))
}

#[derive(Deserialize)]
struct ListQuery {
    all: Option<bool>,
    provider: Option<String>,
    dir: Option<String>,
    workspace: Option<String>,
}

async fn list_sessions(Query(q): Query<ListQuery>) -> impl IntoResponse {
    let providers = q
        .provider
        .map(|p| p.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let cwd = q.workspace.or(q.dir).or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });
    let params = core::SessionListParams {
        all: q.all.unwrap_or(false),
        providers,
        cwd,
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

async fn get_session(Path((provider, session_id)): Path<(String, String)>) -> impl IntoResponse {
    match core::get_session(&provider, &session_id) {
        Ok(session) => ApiResponse::success(session).into_response(),
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

async fn rename_session(
    Path((provider, session_id)): Path<(String, String)>,
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    match core::rename_session(&provider, &session_id, &body.title) {
        Ok(()) => ApiResponse::success("renamed").into_response(),
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
