use super::*;

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/meta", get(get_meta))
        .route("/api/v1/update-check", get(check_for_update))
        .route("/api/v1/agents", get(providers::list_agent_management))
        .route(
            "/api/v1/agents/summary",
            get(providers::list_agent_management_summary),
        )
        .route(
            "/api/v1/agents/{provider}",
            get(providers::get_agent_management_provider),
        )
        .route(
            "/api/v1/agents/{provider}/detect",
            post(providers::detect_agent_management_provider),
        )
        .route("/api/v1/providers", get(providers::list_providers))
        .route(
            "/api/v1/providers/catalog",
            get(providers::get_provider_catalog).put(providers::update_provider_catalog),
        )
        .route(
            "/api/v1/providers/catalog/active",
            get(providers::get_provider_catalog_active),
        )
        .route(
            "/api/v1/providers/{provider}/activity",
            get(sessions::get_provider_activity),
        )
        .route(
            "/api/v1/providers/{provider}/settings",
            get(providers::list_provider_settings),
        )
        .route(
            "/api/v1/providers/{provider}/settings/{setting_id}",
            get(providers::get_provider_setting)
                .put(providers::update_provider_setting)
                .post(providers::run_provider_setting),
        )
        .route("/api/v1/settings", get(get_settings).put(update_settings))
        .route("/api/v1/system/select-folder", post(select_folder))
        .route("/api/v1/filesystem/directories", get(list_directories))
        .route("/api/v1/system/select-file", post(select_file))
        .route("/api/v1/system/open-external", post(open_external))
        .route(
            "/api/v1/management/activity",
            get(management::list_management_activity),
        )
        .route("/api/v1/backups", get(management::list_backups))
        .route("/api/v1/backups/{backup_id}", get(management::get_backup))
        .route(
            "/api/v1/backups/{backup_id}/restore",
            post(management::restore_backup),
        )
        .route(
            "/api/v1/database/backups",
            post(management::create_database_backup),
        )
        .route(
            "/api/v1/database/backups/verify",
            post(management::verify_database_backup),
        )
        .route(
            "/api/v1/artifacts/inspection",
            get(management::inspect_artifacts),
        )
        .route(
            "/api/v1/artifacts/cleanup",
            post(management::cleanup_artifacts),
        )
        .route("/api/v1/sessions", get(sessions::list_sessions))
        .route(
            "/api/v1/stats/dashboard",
            get(sessions::get_stats_dashboard),
        )
        .route(
            "/api/v1/sessions/bootstrap",
            post(sessions::bootstrap_session_projections),
        )
        .route(
            "/api/v1/sessions/refresh-stale",
            post(sessions::refresh_session_staleness),
        )
        .route(
            "/api/v1/sessions/reproject-stale",
            post(sessions::reproject_stale_sessions),
        )
        .route(
            "/api/v1/sessions/{provider}/{session_id}",
            get(sessions::get_session),
        )
        .route(
            "/api/v1/sessions/{provider}/{session_id}/stats",
            get(sessions::get_session_stats),
        )
        .route(
            "/api/v1/sessions/{provider}/{session_id}/activity",
            get(sessions::get_session_activity),
        )
        .route(
            "/api/v1/sessions/{provider}/{session_id}",
            delete(sessions::delete_session)
                .patch(sessions::rename_session)
                .put(sessions::update_session_local_state),
        )
        .route("/api/v1/export", post(export_session))
        .route(
            "/api/v1/compression/archives",
            get(list_compression_archives),
        )
        .route("/api/v1/compression/archive", get(get_compression_archive))
        .route(
            "/api/v1/compression/providers",
            get(list_compression_providers),
        )
        .route(
            "/api/v1/compression/tool-spec",
            get(get_compression_tool_spec),
        )
        .route(
            "/api/v1/compression/instructions",
            post(get_compression_retrieval_instructions),
        )
        .route(
            "/api/v1/compression/restore",
            post(restore_compression_archive),
        )
        .route(
            "/api/v1/compression/restore-native",
            post(restore_native_compression),
        )
        .route(
            "/api/v1/compression/retrieve",
            post(retrieve_compression_archive),
        )
        .route(
            "/api/v1/compression/expand",
            post(expand_compression_session),
        )
        .route("/api/v1/compression/plan", post(plan_active_compression))
        .route("/api/v1/compression/apply", post(apply_active_compression))
        .route("/api/v1/import", post(import_session))
        .route("/api/v1/switch", post(switch_session))
        .route("/api/v1/find", get(find_sessions))
        .route("/api/v1/workspaces", get(list_workspaces))
        .route(
            "/api/v1/workspaces/with-sessions",
            get(list_workspaces_with_sessions),
        )
        .route(
            "/api/v1/workspaces/history",
            delete(remove_workspace_history),
        )
        .route(
            "/api/v1/workspaces/providers",
            get(get_workspace_providers).put(update_workspace_providers),
        )
        .route(
            "/api/v1/sync",
            get(list_sync_groups).post(create_sync_group),
        )
        .route("/api/v1/sync/status", get(sync_status))
        .route("/api/v1/sync/sync", post(sync_session_groups))
        .route("/api/v1/sync/bind", post(bind_sync_group))
        .route(
            "/api/v1/sync/holdings/{group_id}/{holding_id}",
            delete(unbind_sync_group),
        )
        .route(
            "/api/v1/sync/{group_id}",
            delete(remove_sync_group).patch(rename_sync_group),
        )
        .route("/api/v1/manager/preview", post(manager_preview))
        .route("/api/v1/manager/quick-preview", get(manager_quick_preview))
        .route(
            "/api/v1/manager/quick-workspaces",
            get(manager_quick_workspaces),
        )
        .route("/api/v1/manager/workspaces", post(manager_workspaces))
        .route("/api/v1/manager/stats", post(manager_stats))
        .route("/api/v1/manager/clean", post(manager_clean))
        .route(
            "/api/v1/manager/clean-workspace",
            post(manager_clean_workspace),
        )
        .route("/api/v1/manager/backup", post(manager_backup))
        .route(
            "/api/v1/manager/backup-workspace",
            post(manager_backup_workspace),
        )
        .merge(hooks::server::router())
        .merge(crate::skills::server::router())
}
