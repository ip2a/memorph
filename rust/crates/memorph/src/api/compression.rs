use super::*;

pub(super) async fn list_compression_archives_cached(
    workspace: Option<String>,
) -> anyhow::Result<Vec<core::compression::CompressionArchiveSummary>> {
    let workspace_key = workspace.filter(|value| !value.is_empty());

    if let Some(items) = cache::compression_archives_cache().get(workspace_key.as_deref()) {
        return Ok(items);
    }

    let workspace_for_spawn = workspace_key.clone();
    let items = tokio::task::spawn_blocking(move || {
        core::compression_application::list_compression_archives(workspace_for_spawn.as_deref())
    })
    .await
    .map_err(|err| anyhow!("Failed to list compression archives: {err}"))??;

    cache::compression_archives_cache().set(workspace_key.as_deref(), items.clone());

    Ok(items)
}

#[derive(Deserialize)]
pub(super) struct CompressionArchivesQuery {
    workspace: Option<String>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

pub(super) async fn list_compression_archives(
    Query(q): Query<CompressionArchivesQuery>,
) -> impl IntoResponse {
    let workspace = q.workspace.clone().filter(|value| !value.is_empty());
    match list_compression_archives_cached(workspace).await {
        Ok(items) => {
            let total = items.len();
            let offset = q.offset.unwrap_or(0).min(total);
            let remaining = total.saturating_sub(offset);
            let limit = q
                .limit
                .map(|value| value.min(remaining))
                .unwrap_or(remaining);
            let page: Vec<_> = items.into_iter().skip(offset).take(limit).collect();

            let mut response = ApiResponse::success(page).into_response();
            let headers = response.headers_mut();
            let _ = headers.insert("X-Total-Count", total.to_string().parse().unwrap());
            let _ = headers.insert("X-Offset", offset.to_string().parse().unwrap());
            let _ = headers.insert("X-Limit", limit.to_string().parse().unwrap());
            response
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct CompressionArchiveQuery {
    archive_ref: String,
}

pub(super) async fn get_compression_archive(
    Query(q): Query<CompressionArchiveQuery>,
) -> impl IntoResponse {
    match run_blocking(move || {
        core::compression_application::get_compression_archive(&q.archive_ref)
    })
    .await
    {
        Ok(archive) => ApiResponse::success(archive).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn list_compression_providers() -> impl IntoResponse {
    ApiResponse::success(core::compression_application::list_compression_provider_support())
}

pub(super) async fn get_compression_tool_spec() -> impl IntoResponse {
    ApiResponse::success(core::compression_application::compression_retrieval_tool_spec())
}

#[derive(Deserialize)]
pub(super) struct CompressionRetrievalInstructionsBody {
    archive_ref: String,
}

pub(super) async fn get_compression_retrieval_instructions(
    Json(body): Json<CompressionRetrievalInstructionsBody>,
) -> impl IntoResponse {
    match run_blocking(move || {
        core::compression_application::compression_retrieval_instructions(&body.archive_ref)
    })
    .await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct RestoreNativeCompressionBody {
    provider_id: String,
    session_id: String,
    archive_ref: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RestoreCompressionArchiveBody {
    archive_ref: String,
    output_prefix: Option<String>,
    #[serde(default = "default_format")]
    format: String,
}

#[derive(Deserialize)]
pub(super) struct RetrieveCompressionArchiveBody {
    archive_ref: String,
    query: Option<String>,
    max_results: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct ExpandCompressionSessionBody {
    file: String,
    output_prefix: Option<String>,
    #[serde(default = "default_format")]
    format: String,
}

#[derive(Deserialize)]
pub(super) struct ActiveCompressionPlanBody {
    source_provider_id: String,
    target_provider_id: String,
    session_id: Option<String>,
    file: Option<String>,
    #[serde(default)]
    policy: core::active_compression::ActiveCompressionPolicy,
}

#[derive(Deserialize)]
pub(super) struct ActiveCompressionApplyBody {
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

pub(super) async fn restore_native_compression(
    Json(body): Json<RestoreNativeCompressionBody>,
) -> impl IntoResponse {
    let params = core::compression_application::RestoreNativeCompressionParams {
        provider_id: body.provider_id,
        session_id: body.session_id,
        archive_ref: body.archive_ref,
    };
    match run_blocking(move || {
        core::compression_application::restore_native_compression(&params, ActivityActor::Api)
    })
    .await
    {
        Ok(result) => {
            invalidate_compression_archives_cache();
            ApiResponse::success(result).into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn restore_compression_archive(
    Json(body): Json<RestoreCompressionArchiveBody>,
) -> impl IntoResponse {
    let params = core::compression_application::RestoreCompressionArchiveParams {
        archive_ref: body.archive_ref,
        output_prefix: body.output_prefix,
        format: body.format,
    };
    match run_blocking(move || {
        core::compression_application::restore_compression_archive(&params, ActivityActor::Api)
    })
    .await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn retrieve_compression_archive(
    Json(body): Json<RetrieveCompressionArchiveBody>,
) -> impl IntoResponse {
    let params = core::compression_application::RetrieveCompressionArchiveParams {
        archive_ref: body.archive_ref,
        query: body.query,
        max_results: body.max_results,
    };
    match run_blocking(move || core::compression_application::retrieve_compression_archive(&params))
        .await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn expand_compression_session(
    Json(body): Json<ExpandCompressionSessionBody>,
) -> impl IntoResponse {
    let params = core::compression_application::ExpandCompressionSessionParams {
        file: body.file,
        output_prefix: body.output_prefix,
        format: body.format,
    };
    match run_blocking(move || {
        core::compression_application::expand_compression_session(&params, ActivityActor::Api)
    })
    .await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn plan_active_compression(
    Json(body): Json<ActiveCompressionPlanBody>,
) -> impl IntoResponse {
    let params = core::compression_application::ActiveCompressionDryRunParams {
        source_provider_id: body.source_provider_id,
        target_provider_id: body.target_provider_id,
        session_id: body.session_id,
        file: body.file,
        policy: body.policy,
    };
    match run_blocking(move || core::compression_application::active_compression_dry_run(&params))
        .await
    {
        Ok(result) => ApiResponse::success(result).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn apply_active_compression(
    Json(body): Json<ActiveCompressionApplyBody>,
) -> impl IntoResponse {
    let params = core::compression_application::ActiveCompressionApplyCommandParams {
        source_provider_id: body.source_provider_id,
        target_provider_id: body.target_provider_id,
        session_id: body.session_id,
        file: body.file,
        policy: body.policy,
        candidate_ids: body.candidate_ids,
        output_prefix: body.output_prefix,
        format: body.format,
    };
    match run_blocking(move || {
        core::compression_application::active_compression_apply(&params, ActivityActor::Api)
    })
    .await
    {
        Ok(result) => {
            invalidate_compression_archives_cache();
            ApiResponse::success(result).into_response()
        }
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
