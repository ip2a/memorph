use super::*;

#[derive(Deserialize)]
pub(super) struct RenameBody {
    title: String,
}

pub(super) async fn get_stats_dashboard(
    Query(query): Query<memorph::stats_dashboard::StatsDashboardQuery>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || memorph::stats_dashboard::dashboard(&query)).await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

fn session_list_params(q: ListQuery, limit: Option<usize>) -> core::SessionListParams {
    let providers = q
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

    core::SessionListParams {
        all: q.all.unwrap_or(false),
        providers,
        cwd,
        fields: q.fields.unwrap_or_default(),
        limit,
        offset: q.offset,
        sort: q.sort.unwrap_or_default(),
    }
}

#[derive(Serialize)]
struct SessionPagePayload {
    groups: Vec<core::SessionGroup>,
    offset: usize,
    limit: usize,
    has_more: bool,
    /// True when the SQLite store had no indexed sessions for the requested
    /// workspace and a background workspace indexing pass was triggered.
    /// Clients should re-poll shortly; this response is intentionally empty
    /// rather than blocking on a scan (read paths never implicitly block on
    /// full provider scans).
    degraded: bool,
}

pub(super) async fn list_session_page(Query(q): Query<ListQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let offset = q.offset.unwrap_or(0);
    let cwd_for_warmup = q.workspace.clone().or(q.dir.clone());
    let params = session_list_params(q, Some(limit.saturating_add(1)));
    match memorph::runtime::run_blocking(move || core::projection::list_sessions(&params)).await {
        Ok(groups) => {
            let has_more = groups.iter().any(|group| group.sessions.len() > limit);
            let total: usize = groups.iter().map(|g| g.sessions.len()).sum();
            // Cold path: empty result for a workspace scope means the SQLite
            // projection has not indexed this workspace yet. Trigger a
            // background warmup for that workspace only (per-provider, bounded
            // by fingerprint dedup) and flag the response so clients re-poll.
            // The read itself never blocks on a scan.
            let degraded = total == 0
                && cwd_for_warmup
                    .as_deref()
                    .map(|d| !d.trim().is_empty())
                    .unwrap_or(false);
            if degraded {
                if let Some(cwd) = cwd_for_warmup.clone() {
                    core::projection::spawn_workspace_index_background(
                        std::path::PathBuf::from(cwd),
                        memorph::storage::activity_store::ActivityActor::Api,
                    );
                }
            }
            let mut groups = groups;
            for group in &mut groups {
                group.sessions.truncate(limit);
            }
            ApiResponse::success(SessionPagePayload {
                groups,
                offset,
                limit,
                has_more,
                degraded,
            })
            .into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn refresh_session_staleness() -> impl IntoResponse {
    match memorph::runtime::run_blocking(|| {
        core::projection::refresh_projected_session_staleness(ActivityActor::Api)
    })
    .await
    {
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
    match memorph::runtime::run_blocking(move || {
        core::projection::bootstrap_session_projections(
            request.provider.as_deref(),
            ActivityActor::Api,
        )
    })
    .await
    {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn reproject_stale_sessions(
    Json(request): Json<SessionReprojectStaleRequest>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || {
        core::projection::reproject_stale_sessions(request.provider.as_deref(), ActivityActor::Api)
    })
    .await
    {
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

pub(super) async fn index_workspace_sessions(
    Json(request): Json<SessionWorkspaceIndexRequest>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || {
        core::projection::index_workspace_sessions(
            &request.provider,
            std::path::Path::new(&request.workspace_dir),
            ActivityActor::Api,
        )
    })
    .await
    {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}


/// Map a session-read anyhow error to an HTTP status per rule 10:
/// distinguish unindexed / source-missing / unknown-provider / import-failed
/// rather than collapsing every failure to 404.
fn classify_session_read_error(error: anyhow::Error) -> (StatusCode, String) {
    let message = format!("{error:#}");
    let lower = message.to_ascii_lowercase();
    if lower.contains("unknown provider") {
        (StatusCode::NOT_FOUND, message)
    } else if lower.contains("not indexed") {
        // Session is known to the provider but has not been projected yet.
        // 404 with an explicit reason; clients can retry after indexing.
        (StatusCode::NOT_FOUND, message)
    } else if lower.contains("source is missing") || lower.contains("no source locator") {
        // Source file was removed after the session was indexed.
        (StatusCode::GONE, message)
    } else if lower.contains("does not support") {
        (StatusCode::NOT_IMPLEMENTED, message)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

pub(super) async fn get_session(
    Path((provider, session_id)): Path<(String, String)>,
    Query(q): Query<SessionDetailQuery>,
) -> impl IntoResponse {
    let events_offset = q.event_offset.unwrap_or(0);
    let events_limit = q.event_limit;
    let event_search = q
        .event_search
        .map(|query| query.trim().to_string())
        .filter(|query| !query.is_empty());
    let event_order = core::SessionEventOrder::parse(q.event_order.as_deref());
    let requested_search = event_search.clone();
    let requested_order = match event_order {
        core::SessionEventOrder::Desc => Some("desc".to_string()),
        core::SessionEventOrder::Asc => None,
    };
    match memorph::runtime::run_blocking(move || {
        core::sessions::get_session_detail_view_page_result(
            &provider,
            &session_id,
            events_offset,
            events_limit,
            event_search.as_deref(),
            event_order,
        )
    })
    .await
    {
        Ok(result) => {
            let view = result.view;
            if let Some(project_dir) = view.workspace_dir.as_deref() {
                let _ = config::remember_workspace(std::path::Path::new(project_dir));
            }
            let returned_event_count = view.events.len();
            let has_more_events = if let Some(matched_count) = result.matched_event_count {
                events_offset + returned_event_count < matched_count
            } else {
                events_offset + returned_event_count < view.event_count
            };
            ApiResponse::success(SessionDetailPayload {
                view,
                events_offset,
                events_limit,
                returned_event_count,
                has_more_events,
                event_search: requested_search,
                event_order: requested_order,
                matched_event_count: result.matched_event_count,
                returned_event_indices: result.returned_event_indices,
            })
            .into_response()
        }
        Err(e) => {
            let (status, message) = classify_session_read_error(e);
            api_error(status, message).into_response()
        }
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
    match memorph::runtime::run_blocking(move || {
        core::session_mutation::delete_session(&provider, &session_id, ActivityActor::Api)
    })
    .await
    {
        Ok(()) => ApiResponse::success("deleted").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn rename_session(
    Path((provider, session_id)): Path<(String, String)>,
    Json(body): Json<RenameBody>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || {
        core::session_mutation::rename_session(
            &provider,
            &session_id,
            &body.title,
            ActivityActor::Api,
        )
    })
    .await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn update_session_local_state(
    Path((provider, session_id)): Path<(String, String)>,
    Json(body): Json<memorph::storage::session_state::SessionLocalStateUpdate>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || {
        core::session_mutation::update_session_local_state(
            &provider,
            &session_id,
            &body,
            ActivityActor::Api,
        )
    })
    .await
    {
        Ok(state) => ApiResponse::success(state).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn classifies_unindexed_as_404() {
        let (status, _) = classify_session_read_error(anyhow!("Session is not indexed: claude/abc"));
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn classifies_source_missing_as_gone() {
        let (status, _) =
            classify_session_read_error(anyhow!("Session source is missing: /tmp/x.jsonl"));
        assert_eq!(status, StatusCode::GONE);
    }

    #[test]
    fn classifies_unknown_provider_as_404() {
        let (status, _) = classify_session_read_error(anyhow!("Unknown provider: nope"));
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn classifies_unsupported_as_not_implemented() {
        let (status, _) = classify_session_read_error(anyhow!(
            "Provider does not support session detail reads: foo"
        ));
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }
}
