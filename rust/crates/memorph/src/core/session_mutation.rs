use super::*;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Bump the feed revision for a session's workspace after a mutation that
/// changes its display. No-op for sessions without a workspace (they are not
/// part of any workspace feed). Logs instead of failing — a stale revision is
/// preferable to failing a user mutation.
fn bump_session_workspace_revision(conn: &rusqlite::Connection, provider_id: &str, session_id: &str) {
    super::projection::bump_feed_revision_for_session(conn, provider_id, session_id);
}

pub fn delete_session(provider_id: &str, session_id: &str, actor: ActivityActor) -> Result<()> {
    delete_sessions(provider_id, &[session_id], actor)
        .into_iter()
        .next()
        .unwrap_or_else(|| Err(anyhow::anyhow!("No delete result for session {session_id}")))
}

pub fn delete_sessions(
    provider_id: &str,
    session_ids: &[&str],
    actor: ActivityActor,
) -> Vec<Result<()>> {
    let mut activity_conn = match local_store::open_database() {
        Ok(conn) => conn,
        Err(error) => {
            let message = format!("Failed to open activity store before delete: {error:#}");
            return session_ids
                .iter()
                .map(|_| Err(anyhow::anyhow!(message.clone())))
                .collect();
        }
    };
    let backup_root = match crate::config::memorph_dir() {
        Ok(path) => path.join("artifacts").join("backups"),
        Err(error) => {
            let message =
                format!("Failed to resolve provider backup root before delete: {error:#}");
            return session_ids
                .iter()
                .map(|_| Err(anyhow::anyhow!(message.clone())))
                .collect();
        }
    };
    let mut activities = Vec::with_capacity(session_ids.len());
    for session_id in session_ids {
        match ActivityStore::new(&activity_conn).start(NewActivity {
            provider_id: Some(provider_id.to_string()),
            provider_session_id: Some((*session_id).to_string()),
            workspace_dir: None,
            operation_kind: ActivityOperationKind::Delete,
            actor,
            summary: "Deleting session".to_string(),
            details: serde_json::json!({"provider_session_id": session_id}),
        }) {
            Ok(activity_id) => activities.push(activity_id),
            Err(error) => {
                let message = format!("Failed to start delete activity: {error:#}");
                for (started_session_id, activity_id) in session_ids.iter().zip(activities.iter()) {
                    let _ = ActivityStore::new(&activity_conn).finish(
                        activity_id,
                        ActivityCompletion::failed(
                            "Delete cancelled before provider write",
                            serde_json::json!({
                                "provider_session_id": started_session_id,
                            }),
                            &message,
                        ),
                    );
                }
                return session_ids
                    .iter()
                    .map(|_| Err(anyhow::anyhow!(message.clone())))
                    .collect();
            }
        }
    }

    // Capture each session's workspace before the hard delete removes its row,
    // so the feed revision can be bumped for the affected workspace afterwards.
    let workspace_keys: Vec<Option<String>> = session_ids
        .iter()
        .map(|session_id| {
            super::projection::workspace_key_for_session(&activity_conn, provider_id, session_id)
        })
        .collect();

    let results = session_management::delete_sessions(
        provider_id,
        session_ids,
        &activities,
        &backup_root,
        &mut activity_conn,
    );
    results
        .into_iter()
        .zip(activities)
        .zip(session_ids)
        .zip(workspace_keys)
        .map(|(((result, activity_id), session_id), workspace_key)| match result {
            Ok(()) => {
                ActivityStore::new(&activity_conn).finish(
                    &activity_id,
                    ActivityCompletion::success(
                        "Deleted session",
                        serde_json::json!({"provider_session_id": session_id}),
                    ),
                )?;
                if let Some(workspace_key) = workspace_key {
                    let _ = crate::storage::workspace_feed_revision::bump(
                        &activity_conn,
                        &workspace_key,
                        now_ms(),
                    );
                }
                Ok(())
            }
            Err(error) => {
                let message = format!("{error:#}");
                ActivityStore::new(&activity_conn).finish(
                    &activity_id,
                    ActivityCompletion::failed(
                        "Failed to delete session",
                        serde_json::json!({"provider_session_id": session_id}),
                        &message,
                    ),
                )?;
                Err(error)
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameResult {
    pub provider_name: String,
    pub session_id: String,
    pub display_title: String,
    pub native_updated: bool,
    pub warning: Option<String>,
}

pub fn rename_session(
    provider_id: &str,
    session_id: &str,
    new_title: &str,
    actor: ActivityActor,
) -> Result<RenameResult> {
    let mut activity_conn = local_store::open_database()?;
    let backup_root = crate::config::memorph_dir()?
        .join("artifacts")
        .join("backups");
    let details = serde_json::json!({
        "provider_session_id": session_id,
        "new_title": new_title,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(provider_id.to_string()),
        provider_session_id: Some(session_id.to_string()),
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Rename,
        actor,
        summary: "Renaming session".to_string(),
        details: details.clone(),
    })?;
    match session_management::rename_session(
        provider_id,
        session_id,
        new_title,
        &activity_id,
        &backup_root,
        &mut activity_conn,
    ) {
        Ok(renamed) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Renamed session",
                    serde_json::json!({
                        "provider_session_id": session_id,
                        "display_title": renamed.display_title,
                        "native_updated": renamed.native_updated,
                        "warning": renamed.warning,
                    }),
                ),
            )?;
            bump_session_workspace_revision(&activity_conn, provider_id, session_id);
            Ok(renamed)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed("Failed to rename session", details, &message),
            )?;
            Err(error)
        }
    }
}

pub fn update_session_local_state(
    provider_id: &str,
    session_id: &str,
    update: &session_state::SessionLocalStateUpdate,
    actor: ActivityActor,
) -> Result<session_state::ResolvedLocalSessionState> {
    let operation_kind = local_state_activity_kind(update);
    let activity_conn = local_store::open_database()?;
    let input_details = serde_json::to_value(update)?;
    let workspace_dir = update
        .workspace_override
        .as_ref()
        .map(|workspace| workspace.workspace_dir.clone());
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(provider_id.to_string()),
        provider_session_id: Some(session_id.to_string()),
        workspace_dir,
        operation_kind,
        actor,
        summary: "Updating session local state".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let prov = providers::find_provider(provider_id)
            .with_context(|| format!("Unknown provider: {}", provider_id))?;
        let projected_identity = crate::storage::snapshot_store::SnapshotStore::new(&activity_conn)
            .find_session_identity(provider_id, session_id)?;
        if projected_identity.is_none() {
            anyhow::bail!("Projected session not found: {}", session_id);
        }

        let mut normalized_update = update.clone();
        if let Some(workspace_override) = normalized_update.workspace_override.as_mut() {
            let workspace = workspace_override.workspace_dir.trim();
            if workspace.is_empty() {
                anyhow::bail!("Workspace path cannot be empty");
            }
            workspace_override.workspace_dir = prov
                .normalized_workspace_key(Some(workspace))
                .with_context(|| format!("Failed to normalize workspace: {}", workspace))?;
        }

        session_state::update_session_state(provider_id, session_id, &normalized_update)
    })();
    match result {
        Ok(state) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Updated session local state",
                    serde_json::to_value(&state)?,
                ),
            )?;
            bump_session_workspace_revision(&activity_conn, provider_id, session_id);
            Ok(state)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to update session local state",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

pub(super) fn local_state_activity_kind(
    update: &session_state::SessionLocalStateUpdate,
) -> ActivityOperationKind {
    let only_hidden = update.hidden.is_some()
        && update.pinned.is_none()
        && update.display_title.is_none()
        && update.notes.is_none()
        && update.tags.is_none()
        && update.preferred_targets.is_none()
        && update.compressed_archive_refs.is_none()
        && update.workspace_override.is_none();
    if only_hidden {
        return ActivityOperationKind::Hide;
    }
    let only_pinned = update.pinned.is_some()
        && update.hidden.is_none()
        && update.display_title.is_none()
        && update.notes.is_none()
        && update.tags.is_none()
        && update.preferred_targets.is_none()
        && update.compressed_archive_refs.is_none()
        && update.workspace_override.is_none();
    if only_pinned {
        return ActivityOperationKind::Pin;
    }
    let only_workspace_override = update.workspace_override.is_some()
        && update.hidden.is_none()
        && update.pinned.is_none()
        && update.display_title.is_none()
        && update.notes.is_none()
        && update.tags.is_none()
        && update.preferred_targets.is_none()
        && update.compressed_archive_refs.is_none();
    if only_workspace_override {
        let workspace = update.workspace_override.as_ref().unwrap();
        let only_workspace_hidden = workspace.hidden.is_some()
            && workspace.pinned.is_none()
            && workspace.preferred_targets.is_none();
        if only_workspace_hidden {
            return ActivityOperationKind::Hide;
        }
        let only_workspace_pinned = workspace.pinned.is_some()
            && workspace.hidden.is_none()
            && workspace.preferred_targets.is_none();
        if only_workspace_pinned {
            return ActivityOperationKind::Pin;
        }
    }
    ActivityOperationKind::LocalStateUpdate
}
