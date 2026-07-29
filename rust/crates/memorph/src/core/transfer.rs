use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportParams {
    pub provider: String,
    pub session_id: String,
    pub output_prefix: Option<String>,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub files: Vec<String>,
}

pub fn export_session(params: &ExportParams, actor: ActivityActor) -> Result<ExportResult> {
    let mut activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "provider_session_id": params.session_id,
        "format": params.format,
        "output_prefix": params.output_prefix,
        "output_dir": params.output_dir,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(params.provider.clone()),
        provider_session_id: Some(params.session_id.clone()),
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Export,
        actor,
        summary: "Exporting session".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let imported = sessions::get_canonical_session(&params.provider, &params.session_id)?;
        let prefix = params
            .output_prefix
            .as_deref()
            .unwrap_or(&params.session_id);
        let output_dir = params.output_dir.as_deref().map(std::path::Path::new);
        let export = session_management::write_session_export_files(
            &imported.session,
            prefix,
            &params.format,
            output_dir,
        )?;
        let artifacts = register_session_export_artifacts(
            &mut activity_conn,
            &activity_id,
            &params.provider,
            &params.session_id,
            &params.format,
            &export,
        )?;
        Ok((export, artifacts))
    })();
    match result {
        Ok((export, artifacts)) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Exported session",
                    serde_json::json!({
                        "provider_session_id": params.session_id,
                        "format": params.format,
                        "files": export.files,
                        "artifact_ids": artifacts.iter().map(|artifact| artifact.id.clone()).collect::<Vec<_>>(),
                    }),
                ),
            )?;
            Ok(export)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed("Failed to export session", input_details, &message),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportParams {
    pub provider: String,
    pub file_or_id: String,
    pub to_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub provider_name: String,
    pub new_session_id: String,
    pub resume_command: Option<String>,
}

pub fn import_session(params: &ImportParams, actor: ActivityActor) -> Result<ImportResult> {
    let activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "source_ref": params.file_or_id,
        "target_provider_id": params.provider,
        "target_dir": params.to_dir,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(params.provider.clone()),
        provider_session_id: None,
        workspace_dir: params.to_dir.clone(),
        operation_kind: ActivityOperationKind::Import,
        actor,
        summary: "Importing session".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let session = if params.file_or_id.ends_with(".morph")
            || params.file_or_id.ends_with(".json")
            || params.file_or_id.ends_with(".md")
            || params.file_or_id.ends_with(".html")
        {
            session_management::read_session_export_file(&params.file_or_id)?
        } else {
            sessions::get_canonical_session(&params.provider, &params.file_or_id)?
        };

        let target_prov = providers::find_provider(&params.provider)
            .with_context(|| format!("Target provider not available: {}", params.provider))?;
        let target_capabilities = target_prov.capabilities();
        if !target_capabilities.export {
            anyhow::bail!(
                "Provider does not support writing sessions: {}",
                params.provider
            );
        }
        let target_dir = target_prov.resolve_workspace_dir(params.to_dir.as_deref())?;
        let (session, _) =
            session_management::prepare_session_for_target_provider(&session, &params.provider)?;
        let exported = target_prov.export_session(&session, &target_dir)?;

        Ok((
            ImportResult {
                provider_name: target_prov.name().to_string(),
                new_session_id: exported.session_id,
                resume_command: exported.resume_command,
            },
            target_dir,
        ))
    })();
    match result {
        Ok((imported, target_dir)) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status: ActivityStatus::Success,
                    provider_id: Some(params.provider.clone()),
                    provider_session_id: Some(imported.new_session_id.clone()),
                    workspace_dir: Some(target_dir.to_string_lossy().to_string()),
                    summary: "Imported session".to_string(),
                    details: serde_json::json!({
                        "source_ref": params.file_or_id,
                        "target_provider_id": params.provider,
                        "new_session_id": imported.new_session_id,
                        "target_dir": target_dir,
                        "resume_command": imported.resume_command,
                    }),
                    error: None,
                },
            )?;
            // Same index-refresh rationale as switch_session: the UI opens the
            // imported session immediately, so project it now rather than wait
            // for the background sync.
            index_target_provider_sessions(&params.provider);
            Ok(imported)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed("Failed to import session", input_details, &message),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchParams {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_title: Option<String>,
    #[serde(default)]
    pub move_original: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchResult {
    pub from_name: String,
    pub to_name: String,
    pub source_session_id: String,
    pub target_session_id: String,
    pub resume_command: Option<String>,
    #[serde(default)]
    pub removed_original: bool,
}

/// Immediately project the target provider into the SQLite session index after
/// `switch_session` or `import_session` writes its file.
///
/// Both the session list (`list_sessions`) and the detail view read session
/// identities straight from the SQLite index, which the 60s background sync
/// loop (`spawn_background_sync_loop`) only fills in after a delay. Without
/// this synchronous pass the UI opens the freshly written session and hits
/// "Session is not indexed" for up to a minute. Best-effort: a failure only
/// logs, because the export already succeeded and the background sync will
/// still catch up.
pub(super) fn refresh_target_provider_sessions(provider_id: &str) -> Result<()> {
    let provider_id = providers::canonical_provider_id(provider_id);
    let mut conn = local_store::open_database()?;
    projection::bootstrap_session_projections_in_connection(&mut conn, Some(provider_id.as_str()))
        .map(|_| ())
}

pub(super) fn index_target_provider_sessions(provider_id: &str) {
    let provider_id = providers::canonical_provider_id(provider_id);
    if let Err(error) = refresh_target_provider_sessions(&provider_id) {
        crate::logging::error(
            "target_provider_index_refresh",
            format!(
                "Failed to project {provider_id} sessions into the index after writing a session: {error:#}"
            ),
        );
    }
}

pub fn switch_session(params: &SwitchParams) -> Result<SwitchResult> {
    let cwd = std::env::current_dir()?;

    let source_prov = providers::find_provider(&params.from)
        .with_context(|| format!("Unknown source provider: {}", params.from))?;
    let source_capabilities = source_prov.capabilities();
    if !source_capabilities.scan || !source_capabilities.import {
        anyhow::bail!(
            "Source provider does not support reading sessions: {}",
            params.from
        );
    }
    let cwd_str = cwd.to_string_lossy().to_string();

    let session_meta = if let Some(id) = &params.session_id {
        source_prov
            .get_session_meta(id)?
            .with_context(|| format!("Session not found: {}", id))?
    } else {
        let cache = crate::cache::global_cache();
        let sessions = cache.get_or_refresh(&params.from, || source_prov.scan_sessions())?;
        let mut candidates: Vec<_> = sessions
            .into_iter()
            .filter(|s| source_prov.workspace_matches(s.project_dir.as_deref(), Some(&cwd_str)))
            .collect();
        candidates.sort_by_key(|s| std::cmp::Reverse(s.last_active_at));
        candidates.into_iter().next().with_context(|| {
            format!(
                "No {} session found in current workspace: {}\nUse --session-id to specify one, or run from the project directory.",
                source_prov.name(),
                cwd_str
            )
        })?
    };

    let source_session_id = session_meta.session_id.clone();
    let imported = sessions::load_canonical_session_from_meta(
        source_prov.as_ref(),
        &params.from,
        session_meta,
    )?;

    let target_prov = providers::find_provider(&params.to)
        .with_context(|| format!("Unknown target provider: {}", params.to))?;
    let target_capabilities = target_prov.capabilities();
    if !target_capabilities.export {
        anyhow::bail!(
            "Target provider does not support writing sessions: {}",
            params.to
        );
    }
    let target_dir = target_prov.resolve_workspace_dir(params.to_dir.as_deref())?;
    let (mut session, _) = session_management::prepare_session_for_export(
        &imported.session,
        &params.from,
        &params.to,
    )?;
    if let Some(raw_title) = params.target_title.as_ref() {
        let trimmed = raw_title.trim();
        if !trimmed.is_empty() {
            session.identity.title = Some(trimmed.to_string());
        }
    }
    let exported = target_prov.export_session(&session, &target_dir)?;

    let mut removed_original = false;
    if params.move_original {
        if !source_capabilities.delete {
            anyhow::bail!(
                "Source provider does not support deleting sessions: {}",
                params.from
            );
        }
        source_prov.delete_session(&source_session_id)?;
        removed_original = true;
    }

    // Make the freshly exported session visible to list/detail views without
    // waiting on the 60s background sync, which otherwise leaves a
    // "Session is not indexed" window for the UI.
    index_target_provider_sessions(&params.to);

    Ok(SwitchResult {
        from_name: source_prov.name().to_string(),
        to_name: target_prov.name().to_string(),
        source_session_id,
        target_session_id: exported.session_id,
        resume_command: exported.resume_command,
        removed_original,
    })
}
