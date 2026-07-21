use super::*;

pub(super) fn delete_codex_session(session_id: &str) -> Result<()> {
    let session_path = find_session_file(session_id)
        .with_context(|| format!("Codex session not found: {session_id}"))?;
    std::fs::remove_file(&session_path)
        .with_context(|| format!("Failed to remove session file: {}", session_path.display()))?;

    let index_path = get_codex_dir().join("session_index.jsonl");
    if index_path.exists() {
        let content = std::fs::read_to_string(&index_path)?;
        let mut new_lines = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                if value.get("id").and_then(Value::as_str) == Some(session_id) {
                    continue;
                }
            }
            new_lines.push(line.to_string());
        }
        std::fs::write(&index_path, new_lines.join("\n") + "\n")?;
    }

    fail_codex_mutation_after_file_write(ProviderSourceMutation::Delete)?;

    let db_path = get_codex_dir().join(CODEX_SQLITE_FILE_BASENAME);
    if db_path.exists() {
        let mut conn = Connection::open(&db_path)?;
        let tx = conn.transaction()?;
        delete_codex_sqlite_rows(&tx, session_id)?;
        tx.commit()?;
    }
    Ok(())
}

pub(super) fn rename_codex_session(session_id: &str, new_title: &str) -> Result<()> {
    let index_path = get_codex_dir().join("session_index.jsonl");
    if !index_path.exists() {
        anyhow::bail!("Codex session index not found");
    }

    let content = std::fs::read_to_string(&index_path)?;
    let mut new_lines = Vec::new();
    let mut found = false;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_str(line)?;
        if value.get("id").and_then(Value::as_str) == Some(session_id) {
            if let Value::Object(ref mut map) = value {
                map.insert(
                    "thread_name".to_string(),
                    Value::String(new_title.to_string()),
                );
                found = true;
            }
            new_lines.push(serde_json::to_string(&value)?);
        } else {
            new_lines.push(line.to_string());
        }
    }
    if !found {
        anyhow::bail!("Codex session not found in index: {session_id}");
    }

    std::fs::write(&index_path, new_lines.join("\n") + "\n")?;
    if let Some(session_path) = find_session_file(session_id) {
        update_rollout_session_meta_title(&session_path, new_title)?;
    }

    fail_codex_mutation_after_file_write(ProviderSourceMutation::Rename)?;

    let db_path = get_codex_dir().join(CODEX_SQLITE_FILE_BASENAME);
    if db_path.exists() {
        let conn = Connection::open(&db_path)?;
        if has_table(&conn, "threads")? && has_columns(&conn, "threads", &["id", "title"])? {
            conn.execute(
                "UPDATE threads SET title = ?1 WHERE id = ?2",
                [new_title, session_id],
            )?;
        }
    }
    Ok(())
}

pub(super) fn delete_codex_sqlite_rows(conn: &Connection, session_id: &str) -> Result<()> {
    delete_related_rows(conn, "thread_dynamic_tools", "thread_id = ?1", session_id)?;
    delete_related_rows(conn, "thread_goals", "thread_id = ?1", session_id)?;
    delete_related_rows(
        conn,
        "thread_spawn_edges",
        "parent_thread_id = ?1 OR child_thread_id = ?1",
        session_id,
    )?;
    delete_related_rows(conn, "stage1_outputs", "thread_id = ?1", session_id)?;
    if has_table(conn, "agent_job_items")?
        && has_columns(conn, "agent_job_items", &["assigned_thread_id"])?
    {
        conn.execute(
            "UPDATE agent_job_items
             SET assigned_thread_id = NULL
             WHERE assigned_thread_id = ?1",
            [session_id],
        )?;
    }
    if has_table(conn, "threads")? {
        conn.execute("DELETE FROM threads WHERE id = ?1", [session_id])?;
    }
    Ok(())
}

pub fn sync_workspace_sessions(
    workspace: Option<&str>,
    codex_home: Option<&Path>,
    keep_backups: usize,
    actor: ActivityActor,
) -> Result<CodexWorkspaceRepairReport> {
    let codex_dir = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(get_codex_dir);
    let backup_root = crate::config::memorph_dir()?
        .join("artifacts")
        .join("backups")
        .join("codex-sync");
    let mut activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "workspace": workspace,
        "codex_home": utils::user_visible_path(&codex_dir.to_string_lossy()),
        "keep_backups": keep_backups,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(PROVIDER_ID.to_string()),
        provider_session_id: None,
        workspace_dir: workspace.map(str::to_string),
        operation_kind: ActivityOperationKind::Sync,
        actor,
        summary: "Synchronizing Codex workspace sessions".to_string(),
        details: input_details.clone(),
    })?;
    let result = sync_workspace_sessions_in_codex_home(
        &mut activity_conn,
        &activity_id,
        &backup_root,
        &codex_dir,
        workspace,
        keep_backups,
    );
    match result {
        Ok(report) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    workspace_dir: Some(report.workspace_dir.clone()),
                    ..ActivityCompletion::success(
                        "Synchronized Codex workspace sessions",
                        serde_json::json!({"report": &report}),
                    )
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to synchronize Codex workspace sessions",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

pub fn repair_workspace_sessions(
    workspace: Option<&str>,
    actor: ActivityActor,
) -> Result<CodexWorkspaceRepairReport> {
    sync_workspace_sessions(workspace, None, DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT, actor)
}

pub(super) fn sync_workspace_sessions_in_codex_home(
    activity_conn: &mut Connection,
    operation_id: &str,
    backup_root: &Path,
    codex_dir: &Path,
    workspace: Option<&str>,
    keep_backups: usize,
) -> Result<CodexWorkspaceRepairReport> {
    if keep_backups < 1 {
        anyhow::bail!("keep_backups must be at least 1");
    }

    let workspace_root = crate::config::resolve_workspace(workspace)?;
    let workspace_key = crate::provider::default_normalized_workspace_key(workspace_root.to_str())
        .with_context(|| {
            format!(
                "Failed to normalize workspace path: {}",
                workspace_root.display()
            )
        })?;
    let current_model_provider = read_codex_model_provider(codex_dir);
    let mut report = CodexWorkspaceRepairReport {
        workspace_dir: utils::user_visible_path(&workspace_key),
        current_model_provider: current_model_provider.clone(),
        scanned_rollouts: 0,
        workspace_session_count: 0,
        hidden_session_count: 0,
        repaired_session_count: 0,
        reindexed_session_count: 0,
        retitled_session_count: 0,
        backup_dir: None,
        backup_artifact_id: None,
        backup_id: None,
        sqlite_rows_updated: 0,
        sqlite_provider_rows_updated: 0,
        sqlite_user_event_rows_updated: 0,
        sqlite_cwd_rows_updated: 0,
        pruned_backup_count: 0,
        skipped_rollout_files: Vec::new(),
        touched_sessions: Vec::new(),
    };

    let index_path = codex_dir.join("session_index.jsonl");
    let mut indexed_session_entries = load_session_index_entries(&index_path)?;
    let sqlite_lookup = build_sqlite_thread_metadata_lookup(codex_dir)?;
    let session_states = session_state::load_state_store().unwrap_or_default();
    let mut candidates = Vec::new();

    for dir_name in CODEX_SYNC_SESSION_DIRS {
        let root = codex_dir.join(dir_name);
        if !root.exists() {
            continue;
        }

        for entry in WalkDir::new(&root)
            .max_depth(5)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }

            report.scanned_rollouts += 1;
            let Some(session) = (match read_codex_rollout_summary(path) {
                Ok(session) => session,
                Err(error) if is_rollout_file_busy_error(&error) => {
                    report
                        .skipped_rollout_files
                        .push(utils::user_visible_path(&path.to_string_lossy()));
                    continue;
                }
                Err(error) => return Err(error),
            }) else {
                continue;
            };

            if !crate::provider::default_workspace_matches(
                session.workspace_dir.as_deref(),
                Some(&workspace_key),
            ) {
                continue;
            }

            report.workspace_session_count += 1;
            if session.model_provider.as_deref() != Some(current_model_provider.as_str()) {
                report.hidden_session_count += 1;
            }

            candidates.push(CodexWorkspaceSyncCandidate {
                rollout_path: path.to_path_buf(),
                session,
            });
        }
    }

    if candidates.is_empty() && !codex_dir.join(CODEX_GLOBAL_STATE_FILE_BASENAME).exists() {
        return Ok(report);
    }

    let provider_session_ids = candidates
        .iter()
        .map(|candidate| candidate.session.session_id.clone())
        .collect::<Vec<_>>();
    let backup_dir = create_codex_sync_backup(
        backup_root,
        operation_id,
        codex_dir,
        &workspace_key,
        &current_model_provider,
        &candidates
            .iter()
            .map(|candidate| candidate.rollout_path.clone())
            .collect::<Vec<_>>(),
    )?;
    let backup = register_codex_sync_backup(
        activity_conn,
        operation_id,
        codex_dir,
        &backup_dir,
        &workspace_key,
        &current_model_provider,
        &provider_session_ids,
    )
    .with_context(|| {
        format!(
            "Failed to register Codex pre-write backup: {}",
            backup_dir.display()
        )
    })?;
    report.backup_dir = Some(utils::user_visible_path(&backup_dir.to_string_lossy()));
    report.backup_artifact_id = Some(backup.artifact.id);
    report.backup_id = Some(backup.id);

    let sync_result: Result<()> = (|| {
        let mut synced_sessions = Vec::new();

        for candidate in candidates {
            let mut session = candidate.session;
            let provider_mismatch =
                session.model_provider.as_deref() != Some(current_model_provider.as_str());
            let mut updated_model_provider = false;

            if provider_mismatch {
                match rewrite_rollout_model_provider(
                    &candidate.rollout_path,
                    &current_model_provider,
                ) {
                    Ok(()) => {
                        session.model_provider = Some(current_model_provider.clone());
                        updated_model_provider = true;
                        report.repaired_session_count += 1;
                    }
                    Err(error) if is_rollout_file_busy_error(&error) => {
                        report.skipped_rollout_files.push(utils::user_visible_path(
                            &candidate.rollout_path.to_string_lossy(),
                        ));
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }

            let mut added_to_index = false;
            let mut updated_index_title = false;
            let existing_index_title = indexed_session_entries
                .get(&session.session_id)
                .map(String::as_str);

            if existing_index_title.is_none() {
                let index_title = resolve_codex_reindex_title(
                    &session,
                    sqlite_lookup.get(&session.session_id),
                    &session_states,
                );
                append_session_index_entry(
                    &index_path,
                    &session.session_id,
                    &index_title,
                    session.updated_at.as_deref(),
                )?;
                session.title = Some(index_title.clone());
                indexed_session_entries.insert(session.session_id.clone(), index_title);
                added_to_index = true;
                report.reindexed_session_count += 1;
            } else if existing_index_title == Some(&session.session_id) {
                let better_title = resolve_codex_reindex_title(
                    &session,
                    sqlite_lookup.get(&session.session_id),
                    &session_states,
                );
                if !better_title.is_empty() && better_title != session.session_id {
                    update_session_index_entry(&index_path, &session.session_id, &better_title)?;
                    indexed_session_entries
                        .insert(session.session_id.clone(), better_title.clone());
                    session.title = Some(better_title);
                    updated_index_title = true;
                    report.retitled_session_count += 1;
                }
            }

            if updated_model_provider || added_to_index || updated_index_title {
                report.touched_sessions.push(CodexWorkspaceRepairItem {
                    session_id: session.session_id.clone(),
                    title: session.title.clone(),
                    rollout_path: utils::user_visible_path(
                        &candidate.rollout_path.to_string_lossy(),
                    ),
                    workspace_dir: session
                        .workspace_dir
                        .as_deref()
                        .map(utils::user_visible_path),
                    previous_model_provider: session.original_model_provider.clone(),
                    current_model_provider: current_model_provider.clone(),
                    updated_model_provider,
                    added_to_index,
                    updated_index_title,
                });
            }

            synced_sessions.push(session);
        }

        let sqlite_stats =
            sync_workspace_sqlite_metadata(codex_dir, &current_model_provider, &synced_sessions)?;
        report.sqlite_rows_updated = sqlite_stats.rows_updated;
        report.sqlite_provider_rows_updated = sqlite_stats.provider_rows_updated;
        report.sqlite_user_event_rows_updated = sqlite_stats.user_event_rows_updated;
        report.sqlite_cwd_rows_updated = sqlite_stats.cwd_rows_updated;

        update_codex_global_state_file_if_exists(codex_dir, &workspace_root)?;
        Ok(())
    })();

    if let Err(error) = sync_result {
        restore_codex_sync_backup(codex_dir, &backup_dir).with_context(|| {
            format!(
                "Failed to restore Codex sync backup after error: {}",
                backup_dir.display()
            )
        })?;
        return Err(error);
    }

    report.pruned_backup_count = prune_codex_sync_backups(
        activity_conn,
        backup_root,
        codex_dir,
        operation_id,
        keep_backups,
    )?;
    Ok(report)
}

pub(super) fn sync_workspace_sqlite_metadata(
    codex_dir: &Path,
    target_provider: &str,
    sessions: &[CodexRolloutSummary],
) -> Result<CodexWorkspaceSqliteStats> {
    if sessions.is_empty() {
        return Ok(CodexWorkspaceSqliteStats::default());
    }

    let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
    if !sqlite_path.exists() {
        return Ok(CodexWorkspaceSqliteStats::default());
    }

    let conn = Connection::open(&sqlite_path)
        .with_context(|| format!("Failed to open Codex SQLite: {}", sqlite_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute("BEGIN IMMEDIATE", [])
        .with_context(|| "Failed to lock Codex SQLite for workspace sync")?;

    let sync_result: Result<CodexWorkspaceSqliteStats> = (|| {
        if !has_table(&conn, "threads")? {
            return Ok(CodexWorkspaceSqliteStats::default());
        }

        let has_provider_column = has_columns(&conn, "threads", &["model_provider"])?;
        let has_user_event_column = has_columns(&conn, "threads", &["has_user_event"])?;
        let has_cwd_column = has_columns(&conn, "threads", &["cwd"])?;

        let mut stats = CodexWorkspaceSqliteStats::default();
        let mut seen_ids = HashSet::new();

        let mut provider_stmt = if has_provider_column {
            Some(conn.prepare(
                "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND COALESCE(model_provider, '') <> ?1",
            )?)
        } else {
            None
        };
        let mut user_event_stmt = if has_user_event_column {
            Some(conn.prepare(
                "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
            )?)
        } else {
            None
        };
        let mut cwd_stmt =
            if has_cwd_column {
                Some(conn.prepare(
                    "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                )?)
            } else {
                None
            };

        for session in sessions {
            if !seen_ids.insert(session.session_id.clone()) {
                continue;
            }

            if let Some(stmt) = provider_stmt.as_mut() {
                stats.provider_rows_updated +=
                    stmt.execute(rusqlite::params![target_provider, &session.session_id])?;
            }

            if session.has_user_event {
                if let Some(stmt) = user_event_stmt.as_mut() {
                    stats.user_event_rows_updated +=
                        stmt.execute(rusqlite::params![&session.session_id])?;
                }
            }

            if let Some(workspace_dir) = session.workspace_dir.as_deref() {
                if !workspace_dir.trim().is_empty() {
                    if let Some(stmt) = cwd_stmt.as_mut() {
                        stats.cwd_rows_updated +=
                            stmt.execute(rusqlite::params![workspace_dir, &session.session_id])?;
                    }
                }
            }
        }

        stats.rows_updated =
            stats.provider_rows_updated + stats.user_event_rows_updated + stats.cwd_rows_updated;
        Ok(stats)
    })();

    match sync_result {
        Ok(stats) => {
            conn.execute("COMMIT", [])?;
            Ok(stats)
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(error)
        }
    }
}

pub(super) fn create_codex_sync_backup(
    backup_root: &Path,
    operation_id: &str,
    codex_dir: &Path,
    workspace_dir: &str,
    target_provider: &str,
    rollout_paths: &[PathBuf],
) -> Result<PathBuf> {
    std::fs::create_dir_all(backup_root).with_context(|| {
        format!(
            "Failed to create Codex sync backup root: {}",
            backup_root.display()
        )
    })?;
    let backup_dir = backup_root.join(operation_id);
    std::fs::create_dir(&backup_dir).with_context(|| {
        format!(
            "Failed to create Codex sync backup directory: {}",
            backup_dir.display()
        )
    })?;
    let rollouts_dir = backup_dir.join("rollouts");
    let db_dir = backup_dir.join("db");
    std::fs::create_dir_all(&rollouts_dir)?;
    std::fs::create_dir_all(&db_dir)?;

    let session_index_path = codex_dir.join("session_index.jsonl");
    let session_index_present =
        copy_if_present(&session_index_path, &backup_dir.join("session_index.jsonl"))?;

    let mut session_files = Vec::new();
    for rollout_path in rollout_paths {
        let relative = rollout_path.strip_prefix(codex_dir).with_context(|| {
            format!(
                "Failed to compute Codex rollout backup path: {}",
                rollout_path.display()
            )
        })?;
        let backup_path = rollouts_dir.join(relative);
        if let Some(parent) = backup_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(rollout_path, &backup_path).with_context(|| {
            format!(
                "Failed to back up Codex rollout file: {}",
                rollout_path.display()
            )
        })?;
        session_files.push(relative.to_string_lossy().to_string());
    }

    let mut db_files = Vec::new();
    for file_name in [
        CODEX_SQLITE_FILE_BASENAME,
        "state_5.sqlite-shm",
        "state_5.sqlite-wal",
    ] {
        let source = codex_dir.join(file_name);
        let destination = db_dir.join(file_name);
        if copy_if_present(&source, &destination)? {
            db_files.push(file_name.to_string());
        }
    }

    let mut global_state_files = Vec::new();
    for file_name in [
        CODEX_GLOBAL_STATE_FILE_BASENAME,
        CODEX_GLOBAL_STATE_BACKUP_FILE_BASENAME,
    ] {
        let source = codex_dir.join(file_name);
        let destination = backup_dir.join(file_name);
        if copy_if_present(&source, &destination)? {
            global_state_files.push(file_name.to_string());
        }
    }

    let metadata = CodexSyncBackupMetadata {
        version: 1,
        namespace: CODEX_SYNC_BACKUP_NAMESPACE.to_string(),
        operation_id: operation_id.to_string(),
        codex_home: codex_dir.to_string_lossy().to_string(),
        workspace_dir: workspace_dir.to_string(),
        target_provider: target_provider.to_string(),
        created_at: Utc::now().to_rfc3339(),
        session_index_present,
        session_files,
        db_files,
        global_state_files,
    };
    std::fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)?,
    )?;

    Ok(backup_dir)
}

pub(super) fn register_codex_sync_backup(
    conn: &mut Connection,
    operation_id: &str,
    codex_dir: &Path,
    backup_dir: &Path,
    workspace_dir: &str,
    target_provider: &str,
    provider_session_ids: &[String],
) -> Result<BackupRecord> {
    ArtifactStore::new(conn).register_backup(NewBackupRecord {
        operation_id: Some(operation_id.to_string()),
        provider_id: Some(PROVIDER_ID.to_string()),
        provider_session_id: None,
        session_id: None,
        source_path: Some(codex_dir.to_path_buf()),
        backup_path: backup_dir.to_path_buf(),
        restore_hint: Some(
            "Restore this backup with memorph's Codex sync restore flow before reopening Codex."
                .to_string(),
        ),
        mime_type: Some("application/vnd.memorph.codex-sync-backup".to_string()),
        format: Some("codex-sync-backup-v1".to_string()),
        artifact_metadata: serde_json::json!({
            "role": "codex_prewrite_sync_backup",
            "workspace_dir": workspace_dir,
            "target_provider": target_provider,
            "provider_session_ids": provider_session_ids,
        }),
        backup_metadata: serde_json::json!({
            "restore_mode": "codex_sync_restore",
            "metadata_file": "metadata.json",
        }),
    })
}

pub(super) fn restore_codex_sync_backup(codex_dir: &Path, backup_dir: &Path) -> Result<()> {
    let metadata_path = backup_dir.join("metadata.json");
    let metadata: CodexSyncBackupMetadata =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).with_context(|| {
            format!(
                "Failed to read Codex sync backup metadata: {}",
                metadata_path.display()
            )
        })?)?;

    if metadata.codex_home != codex_dir.to_string_lossy() {
        anyhow::bail!(
            "Codex sync backup belongs to another home: {}",
            metadata.codex_home
        );
    }

    let session_index_path = codex_dir.join("session_index.jsonl");
    if metadata.session_index_present {
        std::fs::copy(backup_dir.join("session_index.jsonl"), &session_index_path).with_context(
            || {
                format!(
                    "Failed to restore Codex session index from backup: {}",
                    session_index_path.display()
                )
            },
        )?;
    } else if session_index_path.exists() {
        std::fs::remove_file(&session_index_path).with_context(|| {
            format!(
                "Failed to remove newly created Codex session index: {}",
                session_index_path.display()
            )
        })?;
    }

    for relative in &metadata.session_files {
        let source = backup_dir.join("rollouts").join(relative);
        let target = codex_dir.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &target).with_context(|| {
            format!(
                "Failed to restore Codex rollout file from backup: {}",
                target.display()
            )
        })?;
    }

    let known_db_files = [
        CODEX_SQLITE_FILE_BASENAME,
        "state_5.sqlite-shm",
        "state_5.sqlite-wal",
    ];
    for file_name in known_db_files {
        let target = codex_dir.join(file_name);
        if metadata.db_files.iter().any(|entry| entry == file_name) {
            std::fs::copy(backup_dir.join("db").join(file_name), &target).with_context(|| {
                format!(
                    "Failed to restore Codex SQLite backup: {}",
                    target.display()
                )
            })?;
        } else if target.exists() {
            std::fs::remove_file(&target).with_context(|| {
                format!(
                    "Failed to remove SQLite sidecar created during sync: {}",
                    target.display()
                )
            })?;
        }
    }

    for file_name in &metadata.global_state_files {
        let source = backup_dir.join(file_name);
        let target = codex_dir.join(file_name);
        std::fs::copy(&source, &target).with_context(|| {
            format!(
                "Failed to restore Codex global state backup: {}",
                target.display()
            )
        })?;
    }

    Ok(())
}

pub(super) fn prune_codex_sync_backups(
    conn: &mut Connection,
    backup_root: &Path,
    codex_dir: &Path,
    current_operation_id: &str,
    keep_backups: usize,
) -> Result<usize> {
    if !backup_root.exists() {
        return Ok(0);
    }

    let mut managed_dirs = std::fs::read_dir(backup_root)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter_map(|path| {
            let metadata = load_managed_codex_sync_backup(&path)?;
            (metadata.codex_home == codex_dir.to_string_lossy()).then_some((path, metadata))
        })
        .collect::<Vec<_>>();
    managed_dirs.sort_by(|left, right| {
        if left.1.operation_id == current_operation_id {
            return std::cmp::Ordering::Less;
        }
        if right.1.operation_id == current_operation_id {
            return std::cmp::Ordering::Greater;
        }
        right
            .1
            .created_at
            .cmp(&left.1.created_at)
            .then_with(|| right.0.cmp(&left.0))
    });

    let mut deleted = 0;
    for (stale, _) in managed_dirs.into_iter().skip(keep_backups) {
        let backup_record = ArtifactStore::new(conn).find_backup_by_artifact_path(&stale)?;
        std::fs::remove_dir_all(&stale).with_context(|| {
            format!(
                "Failed to remove stale Codex sync backup: {}",
                stale.display()
            )
        })?;
        if let Some(backup_record) = backup_record {
            ArtifactStore::new(conn).delete_backup_metadata(&backup_record.id)?;
        }
        deleted += 1;
    }

    Ok(deleted)
}

pub(super) fn load_managed_codex_sync_backup(path: &Path) -> Option<CodexSyncBackupMetadata> {
    if !path.is_dir() {
        return None;
    }
    let metadata_path = path.join("metadata.json");
    let content = std::fs::read_to_string(metadata_path).ok()?;
    let metadata = serde_json::from_str::<CodexSyncBackupMetadata>(&content).ok()?;
    (metadata.version == 1 && metadata.namespace == CODEX_SYNC_BACKUP_NAMESPACE).then_some(metadata)
}

pub(super) fn copy_if_present(source: &Path, destination: &Path) -> Result<bool> {
    if !source.exists() {
        return Ok(false);
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, destination).with_context(|| {
        format!(
            "Failed to copy backup file: {} -> {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(true)
}

pub(super) fn is_rollout_file_busy_error(error: &anyhow::Error) -> bool {
    let message = format!("{:#}", error).to_lowercase();
    message.contains("resource busy")
        || message.contains("being used by another process")
        || message.contains("currently in use")
        || message.contains("permission denied")
}
