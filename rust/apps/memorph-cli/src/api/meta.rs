use super::*;

pub(super) async fn get_meta() -> impl IntoResponse {
    match (
        settings_payload(),
        config::selected_workspace(),
        config::known_workspaces(),
        settings_paths_payload(),
        config_file_payload(),
    ) {
        (
            Ok(settings),
            Ok(selected_workspace),
            Ok(workspaces),
            Ok(settings_paths),
            Ok(config_file),
        ) => ApiResponse::success(MetaPayload {
            version: env!("CARGO_PKG_VERSION"),
            selected_workspace,
            workspaces,
            capabilities: CapabilitiesPayload {
                system_folder_picker: FOLDER_PICKER.get().is_some(),
            },
            settings,
            settings_paths,
            config_file,
        })
        .into_response(),
        (Err(e), _, _, _, _)
        | (_, Err(e), _, _, _)
        | (_, _, Err(e), _, _)
        | (_, _, _, Err(e), _)
        | (_, _, _, _, Err(e)) => api_error(StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub(super) async fn check_for_update() -> impl IntoResponse {
    match update_check_payload().await {
        Ok(payload) => ApiResponse::success(payload).into_response(),
        Err(error) => api_error(StatusCode::BAD_GATEWAY, error).into_response(),
    }
}
