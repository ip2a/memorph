use super::management::delete_codex_sqlite_rows;
use super::*;

pub(super) fn create_codex_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    let codex_dir = get_codex_dir();
    let source_path = codex_dir.canonicalize().with_context(|| {
        format!(
            "Failed to resolve Codex data directory: {}",
            codex_dir.display()
        )
    })?;
    let index_path = source_path.join("session_index.jsonl");
    let rollout_path = find_session_file(session_id)
        .map(|path| path.canonicalize())
        .transpose()
        .with_context(|| format!("Failed to resolve Codex rollout for session {session_id}"))?;
    if rollout_path
        .as_deref()
        .is_some_and(|path| !path.starts_with(&source_path))
    {
        anyhow::bail!("Codex rollout is outside the Codex data directory");
    }

    match mutation {
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
            if rollout_path.is_none() =>
        {
            anyhow::bail!("Codex session not found: {session_id}");
        }
        ProviderSourceMutation::Rename => {
            if !index_path.exists() {
                anyhow::bail!("Codex session index not found");
            }
            if !codex_index_contains_session(&index_path, session_id)? {
                anyhow::bail!("Codex session not found in index: {session_id}");
            }
        }
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {}
    }

    let provider_backup_root = backup_root.join(PROVIDER_ID);
    std::fs::create_dir_all(&provider_backup_root).with_context(|| {
        format!(
            "Failed to create Codex backup root: {}",
            provider_backup_root.display()
        )
    })?;
    let backup_path = provider_backup_root.join(operation_id);
    std::fs::create_dir(&backup_path).with_context(|| {
        format!(
            "Failed to create Codex session backup: {}",
            backup_path.display()
        )
    })?;

    let session_index = capture_codex_file(
        Some(index_path.clone()),
        PathBuf::from("session_index.jsonl"),
        &backup_path,
    )?;
    let rollout = capture_codex_file(
        rollout_path.clone(),
        PathBuf::from("rollout").join("session.jsonl"),
        &backup_path,
    )?;

    let db_path = source_path.join(CODEX_SQLITE_FILE_BASENAME);
    let database_present = db_path.exists();
    let sqlite_tables = if database_present {
        std::fs::create_dir(backup_path.join("sqlite"))?;
        let mut conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open Codex SQLite: {}", db_path.display()))?;
        capture_codex_sqlite_backup(
            &mut conn,
            mutation,
            session_id,
            &backup_path.join(CODEX_SESSION_BACKUP_DB_PATH),
        )?
    } else {
        Vec::new()
    };

    let metadata = CodexSessionBackupMetadata {
        version: 1,
        provider_id: PROVIDER_ID.to_string(),
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        codex_home: source_path.clone(),
        db_path,
        database_present,
        session_index,
        rollout,
        sqlite_tables,
    };
    std::fs::write(
        backup_path.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )
    .with_context(|| {
        format!(
            "Failed to write Codex backup metadata: {}",
            backup_path.display()
        )
    })?;

    Ok(ProviderSessionBackup {
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        source_path,
        backup_path,
        restore_hint:
            "Restore this backup with memorph's Codex native session restore flow before reopening Codex."
                .to_string(),
        mime_type: CODEX_SESSION_BACKUP_MIME.to_string(),
        format: CODEX_SESSION_BACKUP_FORMAT.to_string(),
        artifact_metadata: serde_json::json!({
            "role": "codex_prewrite_session_backup",
            "mutation": mutation,
            "sqlite_table_count": metadata.sqlite_tables.len(),
            "session_index_present": metadata.session_index.present,
            "rollout_present": metadata.rollout.present,
        }),
        restore_metadata: serde_json::json!({
            "restore_mode": "codex_session_restore",
            "metadata_file": "metadata.json",
            "mutation": mutation,
        }),
    })
}

pub(super) fn restore_codex_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != CODEX_SESSION_BACKUP_FORMAT {
        anyhow::bail!("Unsupported Codex session backup format: {}", backup.format);
    }
    if backup.mime_type != CODEX_SESSION_BACKUP_MIME {
        anyhow::bail!(
            "Unsupported Codex session backup MIME type: {}",
            backup.mime_type
        );
    }

    let metadata_path = backup.backup_path.join("metadata.json");
    let metadata: CodexSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read Codex backup metadata: {}",
                metadata_path.display()
            )
        })?)?;
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.mutation != backup.mutation
        || metadata.codex_home != backup.source_path
        || metadata.db_path != backup.source_path.join(CODEX_SQLITE_FILE_BASENAME)
    {
        anyhow::bail!(
            "Codex backup metadata does not match the registered restore context: {}",
            backup.backup_path.display()
        );
    }
    validate_codex_file_manifest(&metadata)?;

    if metadata.database_present {
        restore_codex_sqlite_backup(
            &metadata.db_path,
            &backup.backup_path.join(CODEX_SESSION_BACKUP_DB_PATH),
            metadata.mutation,
            &metadata.provider_session_id,
            &metadata.sqlite_tables,
        )?;
    } else if !metadata.sqlite_tables.is_empty() {
        anyhow::bail!("Codex backup contains SQLite rows without a source database");
    }

    restore_codex_file(&backup.backup_path, &metadata.session_index)?;
    restore_codex_file(&backup.backup_path, &metadata.rollout)?;
    Ok(())
}

pub(super) fn codex_index_contains_session(index_path: &Path, session_id: &str) -> Result<bool> {
    let content = std::fs::read_to_string(index_path)?;
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line).with_context(|| {
            format!(
                "Failed to parse Codex session index: {}",
                index_path.display()
            )
        })?;
        if value.get("id").and_then(Value::as_str) == Some(session_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn capture_codex_file(
    source_path: Option<PathBuf>,
    relative_path: PathBuf,
    backup_path: &Path,
) -> Result<CodexFileBackup> {
    let Some(source_path) = source_path else {
        return Ok(CodexFileBackup {
            source_path: None,
            relative_path,
            present: false,
        });
    };
    let metadata = match std::fs::symlink_metadata(&source_path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let Some(metadata) = metadata else {
        return Ok(CodexFileBackup {
            source_path: Some(source_path),
            relative_path,
            present: false,
        });
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Codex backup source is not a regular file: {}",
            source_path.display()
        );
    }
    let destination = backup_path.join(&relative_path);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&source_path, &destination).with_context(|| {
        format!(
            "Failed to copy Codex backup source: {}",
            source_path.display()
        )
    })?;
    Ok(CodexFileBackup {
        source_path: Some(source_path),
        relative_path,
        present: true,
    })
}

pub(super) fn restore_codex_file(backup_path: &Path, file: &CodexFileBackup) -> Result<()> {
    let Some(source_path) = &file.source_path else {
        if file.present {
            anyhow::bail!("Codex backup marks a pathless file as present");
        }
        return Ok(());
    };
    match std::fs::symlink_metadata(source_path) {
        Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
            std::fs::remove_file(source_path)?;
        }
        Ok(_) => {
            anyhow::bail!(
                "Codex restore target is not a file: {}",
                source_path.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if !file.present {
        return Ok(());
    }

    let captured_path = backup_path.join(&file.relative_path);
    if !captured_path.is_file() {
        anyhow::bail!(
            "Codex backup file does not exist: {}",
            captured_path.display()
        );
    }
    if let Some(parent) = source_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&captured_path, source_path)
        .with_context(|| format!("Failed to restore Codex file: {}", source_path.display()))?;
    Ok(())
}

pub(super) fn validate_codex_file_manifest(metadata: &CodexSessionBackupMetadata) -> Result<()> {
    let expected_index = metadata.codex_home.join("session_index.jsonl");
    if metadata.session_index.source_path.as_deref() != Some(expected_index.as_path())
        || metadata.session_index.relative_path != Path::new("session_index.jsonl")
    {
        anyhow::bail!("Codex backup session index manifest is invalid");
    }
    if metadata.rollout.relative_path != Path::new("rollout/session.jsonl") {
        anyhow::bail!("Codex backup rollout manifest is invalid");
    }
    if let Some(rollout_path) = metadata.rollout.source_path.as_deref() {
        if !rollout_path.starts_with(&metadata.codex_home) {
            anyhow::bail!("Codex backup rollout path is outside the Codex data directory");
        }
    } else if metadata.rollout.present {
        anyhow::bail!("Codex backup marks a pathless rollout as present");
    }
    if matches!(
        metadata.mutation,
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
    ) && !metadata.rollout.present
    {
        anyhow::bail!("Codex full-source backup does not contain a rollout");
    }
    Ok(())
}

pub(super) fn capture_codex_sqlite_backup(
    conn: &mut Connection,
    mutation: ProviderSourceMutation,
    session_id: &str,
    backup_db_path: &Path,
) -> Result<Vec<CodexSqliteTableManifest>> {
    if backup_db_path.exists() {
        anyhow::bail!(
            "Codex SQLite backup already exists: {}",
            backup_db_path.display()
        );
    }
    let backup_db_path_str = backup_db_path.to_str().with_context(|| {
        format!(
            "Codex SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    conn.execute("ATTACH DATABASE ?1 AS memorph_backup", [backup_db_path_str])?;

    let capture_result = (|| -> Result<Vec<CodexSqliteTableManifest>> {
        let tx = conn.transaction()?;
        let mut manifests = Vec::new();
        match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
                capture_codex_full_table(
                    &tx,
                    "threads",
                    &["id"],
                    "id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                capture_codex_full_table(
                    &tx,
                    "thread_dynamic_tools",
                    &["thread_id"],
                    "thread_id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                capture_codex_full_table(
                    &tx,
                    "thread_goals",
                    &["thread_id"],
                    "thread_id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                capture_codex_full_table(
                    &tx,
                    "thread_spawn_edges",
                    &["parent_thread_id", "child_thread_id"],
                    "parent_thread_id = ?1 OR child_thread_id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                capture_codex_full_table(
                    &tx,
                    "stage1_outputs",
                    &["thread_id"],
                    "thread_id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                if has_table(&tx, "agent_job_items")?
                    && has_columns(&tx, "agent_job_items", &["assigned_thread_id"])?
                    && !has_columns(
                        &tx,
                        "agent_job_items",
                        &["job_id", "item_id", "assigned_thread_id"],
                    )?
                {
                    anyhow::bail!(
                        "Codex table agent_job_items cannot restore assigned_thread_id without job_id and item_id"
                    );
                }
                capture_codex_selected_table(
                    &tx,
                    "agent_job_items",
                    &["job_id", "item_id", "assigned_thread_id"],
                    "assigned_thread_id = ?1",
                    session_id,
                    CodexSqliteRestoreMode::AssignedThread,
                    &mut manifests,
                )?;
            }
            ProviderSourceMutation::Rename => {
                capture_codex_selected_table(
                    &tx,
                    "threads",
                    &["id", "title"],
                    "id = ?1",
                    session_id,
                    CodexSqliteRestoreMode::ThreadTitle,
                    &mut manifests,
                )?;
            }
        }
        tx.commit()?;
        Ok(manifests)
    })();

    let detach_result = conn.execute_batch("DETACH DATABASE memorph_backup;");
    match capture_result {
        Ok(manifests) => {
            detach_result?;
            Ok(manifests)
        }
        Err(error) => {
            let _ = detach_result;
            let _ = std::fs::remove_file(backup_db_path);
            Err(error)
        }
    }
}

pub(super) fn capture_codex_full_table(
    conn: &Connection,
    table: &str,
    required_columns: &[&str],
    where_clause: &str,
    session_id: &str,
    manifests: &mut Vec<CodexSqliteTableManifest>,
) -> Result<()> {
    if !has_table(conn, table)? {
        return Ok(());
    }
    if !has_columns(conn, table, required_columns)? {
        anyhow::bail!("Codex table {table} is missing required session columns");
    }
    let quoted_table = quote_codex_sqlite_identifier(table);
    conn.execute(
        &format!(
            "CREATE TABLE memorph_backup.{quoted_table} AS
             SELECT * FROM main.{quoted_table} WHERE {where_clause}"
        ),
        [session_id],
    )?;
    manifests.push(codex_sqlite_table_manifest(
        conn,
        table,
        CodexSqliteRestoreMode::FullRows,
    )?);
    Ok(())
}

pub(super) fn capture_codex_selected_table(
    conn: &Connection,
    table: &str,
    columns: &[&str],
    where_clause: &str,
    session_id: &str,
    restore_mode: CodexSqliteRestoreMode,
    manifests: &mut Vec<CodexSqliteTableManifest>,
) -> Result<()> {
    if !has_table(conn, table)? {
        return Ok(());
    }
    if !has_columns(conn, table, columns)? {
        return Ok(());
    }
    let quoted_table = quote_codex_sqlite_identifier(table);
    let column_list = columns
        .iter()
        .map(|column| quote_codex_sqlite_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute(
        &format!(
            "CREATE TABLE memorph_backup.{quoted_table} AS
             SELECT {column_list} FROM main.{quoted_table} WHERE {where_clause}"
        ),
        [session_id],
    )?;
    manifests.push(codex_sqlite_table_manifest(conn, table, restore_mode)?);
    Ok(())
}

pub(super) fn codex_sqlite_table_manifest(
    conn: &Connection,
    table: &str,
    restore_mode: CodexSqliteRestoreMode,
) -> Result<CodexSqliteTableManifest> {
    let quoted_table = quote_codex_sqlite_identifier(table);
    let row_count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM memorph_backup.{quoted_table}"),
        [],
        |row| row.get(0),
    )?;
    Ok(CodexSqliteTableManifest {
        table: table.to_string(),
        columns: codex_table_columns_in_schema(conn, "memorph_backup", table)?,
        row_count: usize::try_from(row_count)
            .context("Codex backup row count does not fit in usize")?,
        restore_mode,
    })
}

pub(super) fn restore_codex_sqlite_backup(
    db_path: &Path,
    backup_db_path: &Path,
    mutation: ProviderSourceMutation,
    session_id: &str,
    manifests: &[CodexSqliteTableManifest],
) -> Result<()> {
    if !db_path.exists() {
        anyhow::bail!(
            "Codex database required by backup no longer exists: {}",
            db_path.display()
        );
    }
    if !backup_db_path.exists() {
        anyhow::bail!(
            "Codex SQLite backup does not exist: {}",
            backup_db_path.display()
        );
    }
    let backup_db_path_str = backup_db_path.to_str().with_context(|| {
        format!(
            "Codex SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    let mut conn = Connection::open(db_path)?;
    conn.execute("ATTACH DATABASE ?1 AS memorph_backup", [backup_db_path_str])?;

    let restore_result = (|| -> Result<()> {
        validate_codex_sqlite_backup(&conn, mutation, manifests)?;
        let tx = conn.transaction()?;
        match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
                delete_codex_sqlite_rows(&tx, session_id)?;
                if let Some(manifest) = manifests
                    .iter()
                    .find(|manifest| manifest.table == "threads")
                {
                    insert_codex_full_backup_table(&tx, manifest)?;
                }
                for manifest in manifests.iter().filter(|manifest| {
                    manifest.restore_mode == CodexSqliteRestoreMode::FullRows
                        && manifest.table != "threads"
                }) {
                    insert_codex_full_backup_table(&tx, manifest)?;
                }
                if let Some(manifest) = manifests.iter().find(|manifest| {
                    manifest.restore_mode == CodexSqliteRestoreMode::AssignedThread
                }) {
                    restore_codex_assigned_threads(&tx, manifest)?;
                }
            }
            ProviderSourceMutation::Rename => {
                if let Some(manifest) = manifests.first() {
                    restore_codex_thread_title(&tx, manifest)?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    })();
    let detach_result = conn.execute_batch("DETACH DATABASE memorph_backup;");
    if let Err(error) = restore_result {
        return Err(error);
    }
    detach_result?;
    Ok(())
}

pub(super) fn validate_codex_sqlite_backup(
    conn: &Connection,
    mutation: ProviderSourceMutation,
    manifests: &[CodexSqliteTableManifest],
) -> Result<()> {
    validate_codex_sqlite_manifest_contract(mutation, manifests)?;

    let mut tables_stmt = conn.prepare(
        "SELECT name
         FROM memorph_backup.sqlite_master
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let backup_tables = tables_stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    let manifest_tables = manifests
        .iter()
        .map(|manifest| manifest.table.clone())
        .collect::<HashSet<_>>();
    if backup_tables != manifest_tables || manifest_tables.len() != manifests.len() {
        anyhow::bail!("Codex SQLite backup table manifest does not match backup database");
    }

    for manifest in manifests {
        let live_columns = table_columns(conn, &manifest.table)?;
        let backup_columns =
            codex_table_columns_in_schema(conn, "memorph_backup", &manifest.table)?;
        if backup_columns != manifest.columns {
            anyhow::bail!(
                "Codex backup table {} schema does not match its manifest",
                manifest.table
            );
        }
        match manifest.restore_mode {
            CodexSqliteRestoreMode::FullRows if live_columns != manifest.columns => {
                anyhow::bail!("Codex table {} schema changed since backup", manifest.table);
            }
            CodexSqliteRestoreMode::AssignedThread | CodexSqliteRestoreMode::ThreadTitle => {
                let live_columns = live_columns.into_iter().collect::<HashSet<_>>();
                if !manifest
                    .columns
                    .iter()
                    .all(|column| live_columns.contains(column))
                {
                    anyhow::bail!("Codex table {} schema changed since backup", manifest.table);
                }
            }
            CodexSqliteRestoreMode::FullRows => {}
        }

        let quoted_table = quote_codex_sqlite_identifier(&manifest.table);
        let row_count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM memorph_backup.{quoted_table}"),
            [],
            |row| row.get(0),
        )?;
        if usize::try_from(row_count).ok() != Some(manifest.row_count) {
            anyhow::bail!(
                "Codex SQLite backup row count mismatch for table {}",
                manifest.table
            );
        }
    }
    Ok(())
}

pub(super) fn validate_codex_sqlite_manifest_contract(
    mutation: ProviderSourceMutation,
    manifests: &[CodexSqliteTableManifest],
) -> Result<()> {
    let mut seen = HashSet::new();
    for manifest in manifests {
        if !seen.insert(manifest.table.as_str()) {
            anyhow::bail!("Codex SQLite backup contains duplicate table manifests");
        }
        let expected = match manifest.table.as_str() {
            "threads"
                if matches!(
                    mutation,
                    ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
                ) =>
            {
                (CodexSqliteRestoreMode::FullRows, None)
            }
            "threads" if mutation == ProviderSourceMutation::Rename => (
                CodexSqliteRestoreMode::ThreadTitle,
                Some(&["id", "title"][..]),
            ),
            "thread_dynamic_tools" | "thread_goals" | "thread_spawn_edges" | "stage1_outputs"
                if matches!(
                    mutation,
                    ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
                ) =>
            {
                (CodexSqliteRestoreMode::FullRows, None)
            }
            "agent_job_items"
                if matches!(
                    mutation,
                    ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
                ) =>
            {
                (
                    CodexSqliteRestoreMode::AssignedThread,
                    Some(&["job_id", "item_id", "assigned_thread_id"][..]),
                )
            }
            _ => anyhow::bail!(
                "Codex SQLite backup contains an unexpected table: {}",
                manifest.table
            ),
        };
        if manifest.restore_mode != expected.0
            || expected
                .1
                .is_some_and(|columns| manifest.columns != columns)
        {
            anyhow::bail!(
                "Codex SQLite backup has an invalid restore contract for table {}",
                manifest.table
            );
        }
    }
    if mutation == ProviderSourceMutation::Rename
        && (manifests.len() > 1
            || manifests
                .first()
                .is_some_and(|manifest| manifest.table != "threads"))
    {
        anyhow::bail!("Codex rename backup may contain only the threads title projection");
    }
    Ok(())
}

pub(super) fn insert_codex_full_backup_table(
    conn: &Connection,
    manifest: &CodexSqliteTableManifest,
) -> Result<()> {
    if manifest.row_count == 0 {
        return Ok(());
    }
    let column_list = manifest
        .columns
        .iter()
        .map(|column| quote_codex_sqlite_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute(
        &format!(
            "INSERT INTO main.{} ({column_list})
             SELECT {column_list} FROM memorph_backup.{}",
            quote_codex_sqlite_identifier(&manifest.table),
            quote_codex_sqlite_identifier(&manifest.table),
        ),
        [],
    )?;
    Ok(())
}

pub(super) fn restore_codex_assigned_threads(
    conn: &Connection,
    manifest: &CodexSqliteTableManifest,
) -> Result<()> {
    if manifest.row_count == 0 {
        return Ok(());
    }
    let updated = conn.execute(
        "UPDATE main.agent_job_items
         SET assigned_thread_id = (
             SELECT backup.assigned_thread_id
             FROM memorph_backup.agent_job_items AS backup
             WHERE backup.job_id = main.agent_job_items.job_id
               AND backup.item_id = main.agent_job_items.item_id
         )
         WHERE EXISTS (
             SELECT 1
             FROM memorph_backup.agent_job_items AS backup
             WHERE backup.job_id = main.agent_job_items.job_id
               AND backup.item_id = main.agent_job_items.item_id
         )",
        [],
    )?;
    if updated != manifest.row_count {
        anyhow::bail!("Codex restore could not find every agent job item captured by the backup");
    }
    Ok(())
}

pub(super) fn restore_codex_thread_title(
    conn: &Connection,
    manifest: &CodexSqliteTableManifest,
) -> Result<()> {
    if manifest.row_count == 0 {
        return Ok(());
    }
    let updated = conn.execute(
        "UPDATE main.threads
         SET title = (
             SELECT backup.title
             FROM memorph_backup.threads AS backup
             WHERE backup.id = main.threads.id
         )
         WHERE EXISTS (
             SELECT 1
             FROM memorph_backup.threads AS backup
             WHERE backup.id = main.threads.id
         )",
        [],
    )?;
    if updated != manifest.row_count {
        anyhow::bail!("Codex restore could not find the thread captured by the rename backup");
    }
    Ok(())
}

pub(super) fn codex_table_columns_in_schema(
    conn: &Connection,
    schema: &str,
    table: &str,
) -> Result<Vec<String>> {
    let pragma = format!(
        "PRAGMA {}.table_info({})",
        quote_codex_sqlite_identifier(schema),
        quote_codex_sqlite_identifier(table)
    );
    let mut stmt = conn.prepare(&pragma)?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if columns.is_empty() {
        anyhow::bail!("Codex database table not found: {schema}.{table}");
    }
    Ok(columns)
}

pub(super) fn quote_codex_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
pub(super) fn set_test_codex_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_CODEX_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Codex mutation failure lock") = mutation;
}

#[cfg(test)]
pub(super) fn fail_codex_mutation_after_file_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_CODEX_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Codex mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected Codex mutation failure after file write");
    }
    Ok(())
}

#[cfg(not(test))]
pub(super) fn fail_codex_mutation_after_file_write(
    _mutation: ProviderSourceMutation,
) -> Result<()> {
    Ok(())
}
