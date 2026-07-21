use super::*;

#[derive(Deserialize)]
pub(super) struct RenameBody {
    title: String,
}

pub(super) async fn get_stats_dashboard(
    Query(query): Query<crate::stats_dashboard::StatsDashboardQuery>,
) -> impl IntoResponse {
    match manager::run_manager_blocking(move || crate::stats_dashboard::dashboard(&query)).await {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn list_sessions(Query(q): Query<ListQuery>) -> impl IntoResponse {
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

pub(super) async fn refresh_session_staleness() -> impl IntoResponse {
    match core::refresh_projected_session_staleness(ActivityActor::Api) {
        Ok(report) => ApiResponse::success(SessionStalenessRefreshPayload {
            checked_sources: report.checked_sources,
            fresh_snapshots: report.fresh_snapshots,
            stale_snapshots: report.stale_snapshots,
            missing_sources: report.missing_sources,
            unknown_sources: report.unknown_sources,
        })
        .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn bootstrap_session_projections(
    Json(request): Json<SessionProjectionBootstrapRequest>,
) -> impl IntoResponse {
    match core::bootstrap_session_projections(request.provider.as_deref(), ActivityActor::Api) {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn reproject_stale_sessions(
    Json(request): Json<SessionReprojectStaleRequest>,
) -> impl IntoResponse {
    match core::reproject_stale_sessions(request.provider.as_deref(), ActivityActor::Api) {
        Ok(report) => ApiResponse::success(SessionReprojectionPayload {
            candidate_snapshots: report.candidate_snapshots,
            reprojected_snapshots: report.reprojected_snapshots,
            missing_sources: report.missing_sources,
            unsupported_providers: report.unsupported_providers,
            failed_snapshots: report.failed_snapshots,
            failures: report.failures,
        })
        .into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn get_session(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<SessionDetailQuery>,
) -> impl IntoResponse {
    let events_offset = q.event_offset.unwrap_or(0);
    let events_limit = q.event_limit;
    match core::sessions::get_session_detail_view_page(
        &provider,
        &session_id,
        events_offset,
        events_limit,
    ) {
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

pub(super) async fn get_session_stats(
    Path((provider, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        core::sessions::compute_session_stats(&provider, &session_id)
    })
    .await;

    match result {
        Ok(Ok(stats)) => ApiResponse::success(stats).into_response(),
        Ok(Err(e)) => api_error(StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to compute session stats: {}", e),
        )
        .into_response(),
    }
}

pub(super) async fn get_provider_activity(
    Path(provider): Path<String>,
    Query(q): Query<ProviderActivityQuery>,
) -> impl IntoResponse {
    let hours = q
        .hours
        .unwrap_or(core::sessions::PROVIDER_ACTIVITY_DEFAULT_HOURS);
    let workspace = q.workspace;
    let all_workspaces = q.all.unwrap_or(false);
    let all_time = q.all_time.unwrap_or(false);
    let result = tokio::task::spawn_blocking(move || {
        core::sessions::compute_provider_activity_timeline(
            &provider,
            workspace.as_deref(),
            hours,
            all_workspaces,
            all_time,
        )
    })
    .await;

    match result {
        Ok(Ok(timeline)) => ApiResponse::success(timeline).into_response(),
        Ok(Err(e)) => api_error(StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to compute provider activity: {}", e),
        )
        .into_response(),
    }
}

pub(super) async fn get_session_activity(
    Path((provider, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        core::sessions::compute_session_activity_timeline(&provider, &session_id)
    })
    .await;

    match result {
        Ok(Ok(timeline)) => ApiResponse::success(timeline).into_response(),
        Ok(Err(e)) => api_error(StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to compute session activity: {}", e),
        )
        .into_response(),
    }
}

pub(super) async fn delete_session(
    Path((provider, session_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match core::delete_session(&provider, &session_id, ActivityActor::Api) {
        Ok(()) => ApiResponse::success("deleted").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn rename_session(
    Path((provider, session_id)): Path<(String, String)>,
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    match core::rename_session(&provider, &session_id, &body.title, ActivityActor::Api) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn update_session_local_state(
    Path((provider, session_id)): Path<(String, String)>,
    Json(body): Json<crate::storage::session_state::SessionLocalStateUpdate>,
) -> impl IntoResponse {
    match core::update_session_local_state(&provider, &session_id, &body, ActivityActor::Api) {
        Ok(state) => ApiResponse::success(state).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
