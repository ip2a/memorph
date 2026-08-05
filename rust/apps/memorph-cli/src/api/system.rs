use super::*;

/// Lightweight "make the environment usable" endpoint. Triggered by the home
/// refresh button so that a deleted/corrupted `~/.memorph` recovers with one
/// click instead of forcing a manual workspace switch.
///
/// Repairs in order: config dir + file, sqlite schema, default workspace if
/// still unset (falls back to the most recently used workspace, not cwd,
/// because this endpoint runs in the server process whose cwd is arbitrary).
pub(super) async fn ensure_ready() -> impl IntoResponse {
    match core::readiness::repair_foundation() {
        Ok(result) => ApiResponse::success(EnsureReadyPayload {
            selected_workspace: result.selected_workspace,
            repaired: result.repaired,
        })
        .into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) fn directory_listing(
    requested_path: Option<&str>,
) -> anyhow::Result<DirectoryListingPayload> {
    let requested = requested_path
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let candidate = match requested {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            if !path.is_absolute() {
                return Err(anyhow!("Directory path must be absolute."));
            }
            path
        }
        None => config::selected_workspace()?
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_dir())
            .or_else(dirs::home_dir)
            .unwrap_or(std::env::current_dir()?),
    };

    let path = candidate
        .canonicalize()
        .with_context(|| format!("Cannot access directory: {}", candidate.display()))?;
    if !path.is_dir() {
        return Err(anyhow!("Path is not a directory: {}", path.display()));
    }

    let mut directories = std::fs::read_dir(&path)
        .with_context(|| format!("Cannot read directory: {}", path.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let entry_path = entry.path();
            entry_path.is_dir().then(|| DirectoryEntryPayload {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry_path.to_string_lossy().into_owned(),
            })
        })
        .collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(DirectoryListingPayload {
        path: path.to_string_lossy().into_owned(),
        parent: path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned()),
        directories,
    })
}

pub(super) async fn list_directories(Query(query): Query<DirectoryQuery>) -> impl IntoResponse {
    match directory_listing(query.path.as_deref()) {
        Ok(listing) => ApiResponse::success(listing).into_response(),
        Err(error) => api_error(StatusCode::BAD_REQUEST, error).into_response(),
    }
}

pub(super) async fn select_folder(Json(body): Json<SelectFolderBody>) -> impl IntoResponse {
    let Some(picker) = FOLDER_PICKER.get() else {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            "Folder picker is only available in the desktop app.",
        )
        .into_response();
    };

    match picker(body.start_path) {
        Ok(path) => ApiResponse::success(SelectFolderPayload { path }).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn select_file(Json(body): Json<SelectFileBody>) -> impl IntoResponse {
    let Some(picker) = FILE_PICKER.get() else {
        return api_error(
            StatusCode::NOT_IMPLEMENTED,
            "File picker is only available in the desktop app.",
        )
        .into_response();
    };

    match picker(body.start_path) {
        Ok(path) => ApiResponse::success(SelectFilePayload { path }).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn open_external(Json(body): Json<OpenExternalBody>) -> impl IntoResponse {
    let url = body.url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return api_error(StatusCode::BAD_REQUEST, "Only http(s) URLs are supported.")
            .into_response();
    }

    match open::that_detached(url) {
        Ok(()) => ApiResponse::success(OpenExternalPayload { opened: true }).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

pub(super) async fn get_settings() -> impl IntoResponse {
    match settings_payload() {
        Ok(settings) => ApiResponse::success(settings).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn update_settings(Json(body): Json<SettingsBody>) -> impl IntoResponse {
    let update_result = config::update_web_preferences(
        Some(body.sessions_per_provider),
        Some(body.language),
        Some(body.show_opencode_subagents),
        body.sort_providers_by_session_count,
        Some(body.default_backup_dir),
        Some(body.logging),
        Some(body.skills_catalog_page_size),
        body.home_session_layout,
    )
    .and_then(|_| config::update_home_button_config(body.home_buttons))
    .and_then(|_| {
        if let Some(server) = body.server {
            config::update_server_preferences(server)
        } else {
            Ok(())
        }
    })
    .and_then(|_| {
        config::update_agent_display_preferences(
            config::ProviderDisplayOrder {
                global: body.agent_order,
                workspace: Vec::new(),
            },
            config::ProviderDisplayHidden {
                global: Vec::new(),
                workspace: Vec::new(),
            },
        )
    });

    match update_result.and_then(|_| settings_payload()) {
        Ok(settings) => ApiResponse::success(settings).into_response(),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
