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
        .as_ref()
        .map(|p| p.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let cwd = q.workspace.clone().or(q.dir.clone()).or_else(|| {
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
    /// Kept for backward compatibility; prefer `feed_state`.
    ///
    /// True when no cached sessions were available for at least one provider
    /// and a background workspace scan is in flight.
    degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    feed_state: Option<core::workspace_session_feed::FeedState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_states: Option<Vec<core::workspace_session_feed::ProviderFeedState>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refreshed_at: Option<i64>,
}

pub(super) async fn list_session_page(Query(q): Query<ListQuery>) -> impl IntoResponse {
    let offset = q.offset.unwrap_or(0);

    // Use the new workspace-scoped adaptive feed when the caller explicitly
    // scopes the request to a workspace. This gives the home view per-provider
    // limits, background refresh state, and fallback to full provider scans when
    // no lightweight scan is available. Queries without an explicit workspace
    // stay on the legacy projection path.
    if let Some(workspace) = q
        .workspace
        .as_deref()
        .or(q.dir.as_deref())
        .filter(|value| !value.trim().is_empty())
    {
        let per_provider_limit = q.limit.unwrap_or(20).clamp(1, 100);
        let feed_params = core::workspace_session_feed::WorkspaceSessionFeedParams {
            workspace_dir: std::path::PathBuf::from(workspace),
            providers: q
                .provider
                .as_ref()
                .map(|p| p.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default(),
            per_provider_limit,
            refresh: q.refresh.unwrap_or(false),
            fields: q.fields.unwrap_or_default(),
            sort: q.sort.unwrap_or_default(),
        };
        match memorph::runtime::run_blocking(move || {
            core::workspace_session_feed::workspace_session_feed(&feed_params)
        })
        .await
        {
            Ok(feed) => {
                let _ = config::remember_workspace(std::path::Path::new(workspace));
                let degraded = matches!(
                    feed.feed_state.kind,
                    core::workspace_session_feed::FeedStateKind::ColdScanning
                );
                ApiResponse::success(SessionPagePayload {
                    groups: feed.groups,
                    offset,
                    limit: per_provider_limit,
                    has_more: feed.has_more,
                    degraded,
                    feed_state: Some(feed.feed_state),
                    provider_states: Some(feed.provider_states),
                    refreshed_at: feed.refreshed_at,
                })
                .into_response()
            }
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        }
    } else {
        let limit = q.limit.unwrap_or(25).clamp(1, 100);
        let params = session_list_params(q, Some(limit.saturating_add(1)));
        match memorph::runtime::run_blocking(move || core::projection::list_sessions(&params)).await
        {
            Ok(groups) => {
                let has_more = groups.iter().any(|group| group.sessions.len() > limit);
                let mut groups = groups;
                for group in &mut groups {
                    group.sessions.truncate(limit);
                }
                ApiResponse::success(SessionPagePayload {
                    groups,
                    offset,
                    limit,
                    has_more,
                    degraded: false,
                    feed_state: None,
                    provider_states: None,
                    refreshed_at: None,
                })
                .into_response()
            }
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct SessionFeedRevisionQuery {
    workspace: String,
}

#[derive(Serialize)]
struct SessionFeedRevisionPayload {
    workspace: String,
    revision: i64,
    updated_at: i64,
    busy: bool,
}

/// Lightweight workspace feed revision poll. Reads the persisted revision and
/// scan-state only — it never starts a scan, so clients can poll it cheaply
/// while a workspace is open and refetch the full session list only when the
/// revision moves.
pub(super) async fn get_session_feed_revision(
    Query(query): Query<SessionFeedRevisionQuery>,
) -> impl IntoResponse {
    match memorph::runtime::run_blocking(move || {
        let workspace_key =
            core::workspace_session_feed::workspace_feed_key(std::path::Path::new(&query.workspace));
        let conn = memorph::storage::local_store::open_database()?;
        let revision = memorph::storage::workspace_feed_revision::read(
            &conn,
            workspace_key.as_deref().unwrap_or(""),
        )?;
        let busy = match workspace_key.as_deref() {
            Some(key) => memorph::storage::workspace_scan_state::WorkspaceScanStateStore::new(&conn)
                .is_workspace_busy(key)?,
            None => false,
        };
        Ok::<_, anyhow::Error>(SessionFeedRevisionPayload {
            workspace: query.workspace,
            revision: revision.revision,
            updated_at: revision.updated_at_ms,
            busy,
        })
    })
    .await
    {
        Ok(payload) => ApiResponse::success(payload).into_response(),
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
        let (status, _) =
            classify_session_read_error(anyhow!("Session is not indexed: claude/abc"));
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
