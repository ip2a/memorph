use super::*;

pub(super) async fn list_sync_groups() -> impl IntoResponse {
    match session_sync::list_groups() {
        Ok(items) => ApiResponse::success(items).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct SyncCreateBody {
    provider: String,
    session_id: String,
    #[serde(default)]
    targets: Vec<String>,
    to_dir: Option<String>,
    title: Option<String>,
}

pub(super) async fn create_sync_group(Json(body): Json<SyncCreateBody>) -> impl IntoResponse {
    let params = session_sync::SyncCreateParams {
        provider: body.provider,
        session_id: body.session_id,
        targets: body.targets,
        to_dir: body.to_dir,
        title: body.title,
    };
    match session_sync::create_group(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct SyncBindBody {
    group_id: String,
    provider: String,
    session_id: Option<String>,
    to_dir: Option<String>,
}

pub(super) async fn bind_sync_group(Json(body): Json<SyncBindBody>) -> impl IntoResponse {
    let params = session_sync::AddHoldingParams {
        group_id: body.group_id,
        provider: body.provider,
        session_id: body.session_id,
        to_dir: body.to_dir,
    };
    match session_sync::add_holding(&params) {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn unbind_sync_group(
    Path((group_id, holding_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match session_sync::remove_holding(&group_id, &holding_id) {
        Ok(()) => ApiResponse::success("unbound").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct SyncStatusQuery {
    group_id: Option<String>,
}

pub(super) async fn sync_status(Query(q): Query<SyncStatusQuery>) -> impl IntoResponse {
    match q.group_id {
        Some(id) => match session_sync::load_group(&id) {
            Ok(mut group) => {
                let _ = session_sync::refresh_active_times(&mut group);
                ApiResponse::success(sync_group_payload(group)).into_response()
            }
            Err(e) => api_error(StatusCode::NOT_FOUND, e).into_response(),
        },
        None => match session_sync::list_groups() {
            Ok(mut groups) => {
                for group in &mut groups {
                    let _ = session_sync::refresh_active_times(group);
                }
                let payload: Vec<SyncGroupPayload> =
                    groups.into_iter().map(sync_group_payload).collect();
                ApiResponse::success(payload).into_response()
            }
            Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        },
    }
}

fn sync_group_payload(group: session_sync::SyncGroup) -> SyncGroupPayload {
    SyncGroupPayload {
        id: group.id,
        title: group.title,
        source_provider: group.source_provider,
        created_at: group.created_at,
        updated_at: group.updated_at,
        holdings: group
            .holdings
            .into_iter()
            .map(sync_holding_payload)
            .collect(),
    }
}

pub(super) fn sync_holding_payload(holding: session_sync::Holding) -> SyncHoldingPayload {
    let hook_augmentation = hooks::augmentation::augment_session(
        &holding.provider,
        &holding.session_id,
        holding.target_dir.as_deref(),
    );
    SyncHoldingPayload {
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

fn resolve_sync_source(
    group: &mut session_sync::SyncGroup,
    source_holding_id: Option<String>,
) -> anyhow::Result<String> {
    if let Some(source_id) = source_holding_id {
        if !group.holdings.iter().any(|holding| holding.id == source_id) {
            anyhow::bail!("Source holding not found: {}", source_id);
        }
        return Ok(source_id);
    }

    session_sync::refresh_active_times(group)?;
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

pub(super) fn blocked_sync_targets_from_snapshot(
    group: &session_sync::SyncGroup,
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
pub(super) struct SyncGroupBody {
    group_id: String,
    source_holding_id: Option<String>,
}

pub(super) async fn sync_session_groups(Json(body): Json<SyncGroupBody>) -> impl IntoResponse {
    let mut group = match session_sync::load_group(&body.group_id) {
        Ok(group) => group,
        Err(e) => return api_error(StatusCode::NOT_FOUND, e).into_response(),
    };
    let source_id = match resolve_sync_source(&mut group, body.source_holding_id) {
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

    let result = session_sync::push_sync(&body.group_id, &source_id, ActivityActor::Api);
    match result {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct SyncRemoveQuery {
    delete_provider_sessions: Option<bool>,
}

pub(super) async fn remove_sync_group(
    Path(group_id): Path<String>,
    Query(q): Query<SyncRemoveQuery>,
) -> impl IntoResponse {
    match session_sync::delete_group(&group_id, q.delete_provider_sessions.unwrap_or(false)) {
        Ok(()) => ApiResponse::success("removed").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct SyncRenameBody {
    title: String,
}

pub(super) async fn rename_sync_group(
    Path(group_id): Path<String>,
    Json(body): Json<SyncRenameBody>,
) -> impl IntoResponse {
    match session_sync::rename_group(&group_id, &body.title) {
        Ok(()) => ApiResponse::success("renamed").into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
