use super::*;

#[derive(Deserialize)]
pub(super) struct ManagementActivityQuery {
    session_id: Option<String>,
    provider: Option<String>,
    workspace: Option<String>,
    operation: Option<String>,
    status: Option<String>,
    actor: Option<String>,
    started_after_ms: Option<i64>,
    started_before_ms: Option<i64>,
    limit: Option<usize>,
}

fn parse_management_activity_filter<T>(name: &str, value: Option<&str>) -> Result<Option<T>, String>
where
    T: FromStr,
    T::Err: Display,
{
    value
        .map(|value| {
            value
                .parse::<T>()
                .map_err(|error| format!("Invalid {name}: {error}"))
        })
        .transpose()
}

pub(super) async fn list_management_activity(
    Query(query): Query<ManagementActivityQuery>,
) -> impl IntoResponse {
    let operation_kind = match parse_management_activity_filter::<ActivityOperationKind>(
        "operation",
        query.operation.as_deref(),
    ) {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, error).into_response(),
    };
    let status =
        match parse_management_activity_filter::<ActivityStatus>("status", query.status.as_deref())
        {
            Ok(value) => value,
            Err(error) => return api_error(StatusCode::BAD_REQUEST, error).into_response(),
        };
    let actor =
        match parse_management_activity_filter::<ActivityActor>("actor", query.actor.as_deref()) {
            Ok(value) => value,
            Err(error) => return api_error(StatusCode::BAD_REQUEST, error).into_response(),
        };
    let query = ActivityQuery {
        session_id: query.session_id,
        provider_id: query.provider,
        workspace_dir: query.workspace,
        operation_kind,
        status,
        actor,
        started_after_ms: query.started_after_ms,
        started_before_ms: query.started_before_ms,
        limit: query.limit,
    };
    match core::management::list_management_activity(&query) {
        Ok(activities) => ApiResponse::success(activities).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct BackupQueryParams {
    operation_id: Option<String>,
    provider: Option<String>,
    provider_session_id: Option<String>,
    restore_status: Option<String>,
    limit: Option<usize>,
}

pub(super) async fn list_backups(Query(query): Query<BackupQueryParams>) -> impl IntoResponse {
    let restore_status =
        match parse_management_activity_filter("restore_status", query.restore_status.as_deref()) {
            Ok(value) => value,
            Err(error) => return api_error(StatusCode::BAD_REQUEST, error).into_response(),
        };
    match core::session_management::list_registered_backups(
        crate::storage::artifact_store::BackupQuery {
            operation_id: query.operation_id,
            provider_id: query.provider,
            provider_session_id: query.provider_session_id,
            restore_status,
            limit: query.limit,
        },
    ) {
        Ok(backups) => ApiResponse::success(backups).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn get_backup(Path(backup_id): Path<String>) -> impl IntoResponse {
    match core::session_management::get_registered_backup(&backup_id) {
        Ok(Some(backup)) => ApiResponse::success(backup).into_response(),
        Ok(None) => api_error(
            StatusCode::NOT_FOUND,
            format!("Unknown backup: {backup_id}"),
        )
        .into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn restore_backup(Path(backup_id): Path<String>) -> impl IntoResponse {
    match core::session_management::restore_registered_backup(&backup_id, ActivityActor::Api) {
        Ok(restore) => ApiResponse::success(restore).into_response(),
        Err(error) if error.to_string().contains("Unknown backup:") => {
            api_error(StatusCode::NOT_FOUND, error).into_response()
        }
        Err(error) => api_error(StatusCode::CONFLICT, error).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct CreateDatabaseBackupRequest {
    output_dir: Option<String>,
}

pub(super) async fn create_database_backup(
    Json(request): Json<CreateDatabaseBackupRequest>,
) -> impl IntoResponse {
    match core::database_management::backup_database(
        request.output_dir.as_deref().map(std::path::Path::new),
        ActivityActor::Api,
    ) {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct VerifyDatabaseBackupRequest {
    bundle: String,
}

pub(super) async fn verify_database_backup(
    Json(request): Json<VerifyDatabaseBackupRequest>,
) -> impl IntoResponse {
    if request.bundle.trim().is_empty() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "Database backup bundle is required",
        )
        .into_response();
    }
    match core::database_management::verify_database_backup(std::path::Path::new(&request.bundle)) {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(error) => api_error(StatusCode::BAD_REQUEST, error).into_response(),
    }
}

pub(super) async fn inspect_artifacts() -> impl IntoResponse {
    match core::management::inspect_artifacts() {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct ArtifactCleanupRequest {
    #[serde(default = "default_artifact_retention_hours")]
    retention_hours: u64,
    #[serde(default)]
    apply: bool,
}

fn default_artifact_retention_hours() -> u64 {
    168
}

pub(super) async fn cleanup_artifacts(
    Json(request): Json<ArtifactCleanupRequest>,
) -> impl IntoResponse {
    if request.retention_hours == 0 {
        return api_error(
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("Artifact retention must be at least one hour"),
        )
        .into_response();
    }
    match core::management::cleanup_artifacts(
        request.retention_hours,
        request.apply,
        ActivityActor::Api,
    ) {
        Ok(report) => ApiResponse::success(report).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}
