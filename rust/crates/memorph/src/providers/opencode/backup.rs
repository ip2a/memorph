use super::*;

pub(super) fn create_opencode_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    let opencode_dir = get_opencode_dir();
    let source_path = opencode_dir.canonicalize().with_context(|| {
        format!(
            "Failed to resolve OpenCode data directory: {}",
            opencode_dir.display()
        )
    })?;
    let db_path = get_db_path();
    let database_present = db_path.exists();
    let mut database =
        if database_present {
            Some(Connection::open(&db_path).with_context(|| {
                format!("Failed to open OpenCode database: {}", db_path.display())
            })?)
        } else {
            None
        };
    let database_session_present = database
        .as_ref()
        .map(|conn| opencode_session_exists(conn, session_id))
        .transpose()?
        .unwrap_or(false);
    let message_ids = if matches!(
        mutation,
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
    ) {
        database
            .as_ref()
            .map(|conn| opencode_message_ids(conn, session_id))
            .transpose()?
            .unwrap_or_default()
    } else {
        HashSet::new()
    };
    let mutation_paths = discover_opencode_mutation_paths(session_id, &message_ids)?;
    let source_exists = database_session_present
        || !mutation_paths.session_files.is_empty()
        || (matches!(
            mutation,
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
        ) && (path_lexists(&mutation_paths.message_dir)
            || !mutation_paths.part_dirs.is_empty()));
    if !source_exists {
        anyhow::bail!("OpenCode session not found: {session_id}");
    }

    let provider_backup_root = backup_root.join(PROVIDER_ID);
    std::fs::create_dir_all(&provider_backup_root).with_context(|| {
        format!(
            "Failed to create OpenCode backup root: {}",
            provider_backup_root.display()
        )
    })?;
    let backup_path = provider_backup_root.join(operation_id);
    std::fs::create_dir(&backup_path).with_context(|| {
        format!(
            "Failed to create OpenCode session backup: {}",
            backup_path.display()
        )
    })?;

    let sqlite_dir = backup_path.join("sqlite");
    std::fs::create_dir(&sqlite_dir)?;
    let sqlite_tables = database
        .as_mut()
        .map(|conn| {
            capture_opencode_sqlite_backup(
                conn,
                mutation,
                session_id,
                &backup_path.join(OPENCODE_BACKUP_DB_PATH),
            )
        })
        .transpose()?
        .unwrap_or_default();

    let mut filesystem_entries = Vec::new();
    for (index, session_path) in mutation_paths.session_files.iter().enumerate() {
        filesystem_entries.push(capture_opencode_filesystem_entry(
            session_path,
            PathBuf::from("filesystem")
                .join("session")
                .join(format!("{index:03}")),
            OpenCodeFilesystemEntryKind::File,
            &backup_path,
        )?);
    }
    if matches!(
        mutation,
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
    ) {
        filesystem_entries.push(capture_opencode_filesystem_entry(
            &mutation_paths.message_dir,
            PathBuf::from("filesystem").join("message"),
            OpenCodeFilesystemEntryKind::Directory,
            &backup_path,
        )?);
        for (index, part_dir) in mutation_paths.part_dirs.iter().enumerate() {
            filesystem_entries.push(capture_opencode_filesystem_entry(
                part_dir,
                PathBuf::from("filesystem")
                    .join("part")
                    .join(format!("{index:03}")),
                OpenCodeFilesystemEntryKind::Directory,
                &backup_path,
            )?);
        }
    }

    let metadata = OpenCodeSessionBackupMetadata {
        version: 1,
        provider_id: PROVIDER_ID.to_string(),
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        opencode_dir: source_path.clone(),
        db_path,
        database_present,
        sqlite_tables,
        filesystem_entries,
    };
    std::fs::write(
        backup_path.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )
    .with_context(|| {
        format!(
            "Failed to write OpenCode backup metadata: {}",
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
            "Restore this backup with memorph's OpenCode native session restore flow before reopening OpenCode."
                .to_string(),
        mime_type: OPENCODE_BACKUP_MIME.to_string(),
        format: OPENCODE_BACKUP_FORMAT.to_string(),
        artifact_metadata: serde_json::json!({
            "role": "opencode_prewrite_session_backup",
            "mutation": mutation,
            "sqlite_table_count": metadata.sqlite_tables.len(),
            "filesystem_entry_count": metadata.filesystem_entries.len(),
        }),
        restore_metadata: serde_json::json!({
            "restore_mode": "opencode_session_restore",
            "metadata_file": "metadata.json",
            "mutation": mutation,
        }),
    })
}

pub(super) fn restore_opencode_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != OPENCODE_BACKUP_FORMAT {
        anyhow::bail!(
            "Unsupported OpenCode session backup format: {}",
            backup.format
        );
    }
    let metadata_path = backup.backup_path.join("metadata.json");
    let metadata: OpenCodeSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read OpenCode backup metadata: {}",
                metadata_path.display()
            )
        })?)?;
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.mutation != backup.mutation
        || metadata.opencode_dir != backup.source_path
    {
        anyhow::bail!(
            "OpenCode backup metadata does not match the registered restore context: {}",
            backup.backup_path.display()
        );
    }

    if metadata.database_present {
        restore_opencode_sqlite_backup(
            &metadata.db_path,
            &backup.backup_path.join(OPENCODE_BACKUP_DB_PATH),
            metadata.mutation,
            &metadata.provider_session_id,
            &metadata.sqlite_tables,
        )?;
    } else if !metadata.sqlite_tables.is_empty() {
        anyhow::bail!("OpenCode backup contains SQLite rows without a source database");
    }

    for entry in &metadata.filesystem_entries {
        restore_opencode_filesystem_entry(&backup.backup_path, entry)?;
    }
    Ok(())
}

pub(super) fn opencode_session_exists(conn: &Connection, session_id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM session WHERE id = ?1", [session_id], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

pub(super) fn opencode_message_ids(conn: &Connection, session_id: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT id FROM message WHERE session_id = ?1")?;
    let message_ids = stmt
        .query_map([session_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    Ok(message_ids)
}

pub(super) fn capture_opencode_sqlite_backup(
    conn: &mut Connection,
    mutation: ProviderSourceMutation,
    session_id: &str,
    backup_db_path: &Path,
) -> Result<Vec<OpenCodeSqliteTableManifest>> {
    if path_lexists(backup_db_path) {
        anyhow::bail!(
            "OpenCode SQLite backup already exists: {}",
            backup_db_path.display()
        );
    }
    let backup_db_path = backup_db_path.to_str().with_context(|| {
        format!(
            "OpenCode SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    conn.execute("ATTACH DATABASE ?1 AS memorph_backup", [backup_db_path])?;

    let capture_result = (|| -> Result<Vec<OpenCodeSqliteTableManifest>> {
        let foreign_keys = load_opencode_foreign_keys(conn)?;
        let tx = conn.transaction()?;
        let session_table = quote_sqlite_identifier("session");
        tx.execute(
            &format!(
                "CREATE TABLE memorph_backup.{session_table} AS
                 SELECT * FROM main.{session_table} WHERE id = ?1"
            ),
            [session_id],
        )?;
        let mut manifests = vec![opencode_backup_table_manifest(&tx, "session")?];
        if mutation == ProviderSourceMutation::Rename || manifests[0].row_count == 0 {
            tx.commit()?;
            return Ok(manifests);
        }

        let mut captured_tables = HashSet::from(["session".to_string()]);
        let mut queue = VecDeque::from(["session".to_string()]);
        while let Some(parent_table) = queue.pop_front() {
            for foreign_key in foreign_keys
                .iter()
                .filter(|foreign_key| foreign_key.parent_table == parent_table)
            {
                if foreign_key.child_columns.len() != 1 || foreign_key.parent_columns.len() != 1 {
                    anyhow::bail!(
                        "OpenCode session backup does not support composite foreign key {} -> {}",
                        foreign_key.child_table,
                        foreign_key.parent_table
                    );
                }
                let parent_column = match &foreign_key.parent_columns[0] {
                    Some(parent_column) => parent_column.clone(),
                    None => single_primary_key_column(&tx, &foreign_key.parent_table)?,
                };
                let matching_rows =
                    count_opencode_backup_child_rows(&tx, foreign_key, &parent_column)?;
                if matching_rows == 0 {
                    continue;
                }
                if !foreign_key.on_delete.eq_ignore_ascii_case("cascade") {
                    anyhow::bail!(
                        "OpenCode session delete would mutate {} through unsupported ON DELETE {} behavior",
                        foreign_key.child_table,
                        foreign_key.on_delete
                    );
                }
                if !captured_tables.insert(foreign_key.child_table.clone()) {
                    anyhow::bail!(
                        "OpenCode session backup found multiple cascading paths to table {}",
                        foreign_key.child_table
                    );
                }
                capture_opencode_backup_child_table(&tx, foreign_key, &parent_column)?;
                manifests.push(opencode_backup_table_manifest(
                    &tx,
                    &foreign_key.child_table,
                )?);
                queue.push_back(foreign_key.child_table.clone());
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
            let _ = std::fs::remove_file(backup_db_path);
            Err(error)
        }
    }
}

pub(super) fn load_opencode_foreign_keys(conn: &Connection) -> Result<Vec<OpenCodeForeignKey>> {
    let mut tables_stmt = conn.prepare(
        "SELECT name
         FROM sqlite_master
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let table_names = tables_stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut foreign_keys = Vec::new();

    for child_table in table_names {
        let pragma = format!(
            "PRAGMA foreign_key_list({})",
            quote_sqlite_identifier(&child_table)
        );
        let mut stmt = conn.prepare(&pragma)?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        let mut groups: BTreeMap<i64, Vec<(i64, String, String, Option<String>, String)>> =
            BTreeMap::new();
        for row in rows {
            let (id, sequence, parent_table, child_column, parent_column, on_delete) = row?;
            groups.entry(id).or_default().push((
                sequence,
                parent_table,
                child_column,
                parent_column,
                on_delete,
            ));
        }
        for (_, mut columns) in groups {
            columns.sort_by_key(|column| column.0);
            let parent_table = columns[0].1.clone();
            let on_delete = columns[0].4.clone();
            let mut child_columns = Vec::with_capacity(columns.len());
            let mut parent_columns = Vec::with_capacity(columns.len());
            for (_, column_parent_table, child_column, parent_column, column_on_delete) in columns {
                if column_parent_table != parent_table || column_on_delete != on_delete {
                    anyhow::bail!(
                        "OpenCode database has an inconsistent foreign key on table {}",
                        child_table
                    );
                }
                child_columns.push(child_column);
                parent_columns.push(parent_column.filter(|column| !column.is_empty()));
            }
            foreign_keys.push(OpenCodeForeignKey {
                child_table: child_table.clone(),
                child_columns,
                parent_table,
                parent_columns,
                on_delete,
            });
        }
    }
    Ok(foreign_keys)
}

pub(super) fn single_primary_key_column(conn: &Connection, table: &str) -> Result<String> {
    let pragma = format!("PRAGMA table_info({})", quote_sqlite_identifier(table));
    let mut stmt = conn.prepare(&pragma)?;
    let primary_keys = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?
        .filter_map(|row| match row {
            Ok((column, position)) if position > 0 => Some(Ok(column)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if primary_keys.len() != 1 {
        anyhow::bail!(
            "OpenCode table {} does not have a single-column primary key",
            table
        );
    }
    Ok(primary_keys[0].clone())
}

pub(super) fn count_opencode_backup_child_rows(
    conn: &Connection,
    foreign_key: &OpenCodeForeignKey,
    parent_column: &str,
) -> Result<usize> {
    let child_table = quote_sqlite_identifier(&foreign_key.child_table);
    let parent_table = quote_sqlite_identifier(&foreign_key.parent_table);
    let child_column = quote_sqlite_identifier(&foreign_key.child_columns[0]);
    let parent_column = quote_sqlite_identifier(parent_column);
    let count: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*)
             FROM main.{child_table} AS child
             WHERE EXISTS (
                 SELECT 1
                 FROM memorph_backup.{parent_table} AS parent
                 WHERE child.{child_column} = parent.{parent_column}
             )"
        ),
        [],
        |row| row.get(0),
    )?;
    usize::try_from(count).context("OpenCode child row count does not fit in usize")
}

pub(super) fn capture_opencode_backup_child_table(
    conn: &Connection,
    foreign_key: &OpenCodeForeignKey,
    parent_column: &str,
) -> Result<()> {
    let child_table = quote_sqlite_identifier(&foreign_key.child_table);
    let parent_table = quote_sqlite_identifier(&foreign_key.parent_table);
    let child_column = quote_sqlite_identifier(&foreign_key.child_columns[0]);
    let parent_column = quote_sqlite_identifier(parent_column);
    conn.execute_batch(&format!(
        "CREATE TABLE memorph_backup.{child_table} AS
         SELECT child.*
         FROM main.{child_table} AS child
         WHERE EXISTS (
             SELECT 1
             FROM memorph_backup.{parent_table} AS parent
             WHERE child.{child_column} = parent.{parent_column}
         );"
    ))?;
    Ok(())
}

pub(super) fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(super) fn opencode_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    opencode_table_columns_in_schema(conn, "main", table)
}

pub(super) fn opencode_table_columns_in_schema(
    conn: &Connection,
    schema: &str,
    table: &str,
) -> Result<Vec<String>> {
    let pragma = format!(
        "PRAGMA {}.table_info({})",
        quote_sqlite_identifier(schema),
        quote_sqlite_identifier(table)
    );
    let mut stmt = conn.prepare(&pragma)?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if columns.is_empty() {
        anyhow::bail!("OpenCode database table not found: {schema}.{table}");
    }
    Ok(columns)
}

pub(super) fn opencode_backup_table_manifest(
    conn: &Connection,
    table: &str,
) -> Result<OpenCodeSqliteTableManifest> {
    let quoted_table = quote_sqlite_identifier(table);
    let row_count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM memorph_backup.{quoted_table}"),
        [],
        |row| row.get(0),
    )?;
    Ok(OpenCodeSqliteTableManifest {
        table: table.to_string(),
        columns: opencode_table_columns_in_schema(conn, "memorph_backup", table)?,
        row_count: usize::try_from(row_count)
            .context("OpenCode backup row count does not fit in usize")?,
    })
}

pub(super) fn restore_opencode_sqlite_backup(
    db_path: &Path,
    backup_db_path: &Path,
    mutation: ProviderSourceMutation,
    session_id: &str,
    manifests: &[OpenCodeSqliteTableManifest],
) -> Result<()> {
    if !db_path.exists() {
        anyhow::bail!(
            "OpenCode database required by backup no longer exists: {}",
            db_path.display()
        );
    }
    if !backup_db_path.exists() {
        anyhow::bail!(
            "OpenCode SQLite backup does not exist: {}",
            backup_db_path.display()
        );
    }
    let backup_db_path_str = backup_db_path.to_str().with_context(|| {
        format!(
            "OpenCode SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    let mut conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute("ATTACH DATABASE ?1 AS memorph_backup", [backup_db_path_str])?;

    let restore_result = (|| -> Result<()> {
        validate_opencode_sqlite_backup(&conn, mutation, manifests)?;
        let tx = conn.transaction()?;
        match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
                tx.execute("DELETE FROM main.session WHERE id = ?1", [session_id])?;
                for manifest in manifests {
                    insert_opencode_backup_table(&tx, manifest, false)?;
                }
            }
            ProviderSourceMutation::Rename => {
                let session = manifests
                    .iter()
                    .find(|manifest| manifest.table == "session")
                    .context("OpenCode rename backup does not contain session table")?;
                insert_opencode_backup_table(&tx, session, true)?;
            }
        }
        tx.commit()?;
        Ok(())
    })();
    let detach_result = conn.execute_batch("DETACH DATABASE memorph_backup;");
    restore_result?;
    detach_result?;
    Ok(())
}

pub(super) fn validate_opencode_sqlite_backup(
    conn: &Connection,
    mutation: ProviderSourceMutation,
    manifests: &[OpenCodeSqliteTableManifest],
) -> Result<()> {
    if manifests.first().map(|manifest| manifest.table.as_str()) != Some("session") {
        anyhow::bail!("OpenCode SQLite backup must start with the session table");
    }
    if mutation == ProviderSourceMutation::Rename && manifests.len() != 1 {
        anyhow::bail!("OpenCode rename backup must contain only the session table");
    }

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
        anyhow::bail!("OpenCode SQLite backup table manifest does not match backup database");
    }

    for manifest in manifests {
        let live_columns = opencode_table_columns(conn, &manifest.table)?;
        let backup_columns =
            opencode_table_columns_in_schema(conn, "memorph_backup", &manifest.table)?;
        if live_columns != manifest.columns || backup_columns != manifest.columns {
            anyhow::bail!(
                "OpenCode table {} schema changed since backup",
                manifest.table
            );
        }
        let quoted_table = quote_sqlite_identifier(&manifest.table);
        let row_count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM memorph_backup.{quoted_table}"),
            [],
            |row| row.get(0),
        )?;
        if usize::try_from(row_count).ok() != Some(manifest.row_count) {
            anyhow::bail!(
                "OpenCode SQLite backup row count mismatch for table {}",
                manifest.table
            );
        }
    }
    Ok(())
}

pub(super) fn insert_opencode_backup_table(
    conn: &Connection,
    manifest: &OpenCodeSqliteTableManifest,
    upsert: bool,
) -> Result<()> {
    if manifest.row_count == 0 {
        return Ok(());
    }
    let column_list = manifest
        .columns
        .iter()
        .map(|column| quote_sqlite_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = if upsert {
        let primary_key = single_primary_key_column(conn, &manifest.table)?;
        let assignments = manifest
            .columns
            .iter()
            .filter(|column| *column != &primary_key)
            .map(|column| {
                let quoted = quote_sqlite_identifier(column);
                format!("{quoted} = excluded.{quoted}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        let conflict_action = if assignments.is_empty() {
            "DO NOTHING".to_string()
        } else {
            format!("DO UPDATE SET {assignments}")
        };
        format!(
            "INSERT INTO main.{} ({column_list})
             SELECT {column_list} FROM memorph_backup.{} WHERE true
             ON CONFLICT ({}) {conflict_action}",
            quote_sqlite_identifier(&manifest.table),
            quote_sqlite_identifier(&manifest.table),
            quote_sqlite_identifier(&primary_key),
        )
    } else {
        format!(
            "INSERT INTO main.{} ({column_list})
             SELECT {column_list} FROM memorph_backup.{}",
            quote_sqlite_identifier(&manifest.table),
            quote_sqlite_identifier(&manifest.table),
        )
    };
    conn.execute(&sql, [])?;
    Ok(())
}

pub(super) fn discover_opencode_mutation_paths(
    session_id: &str,
    database_message_ids: &HashSet<String>,
) -> Result<OpenCodeMutationPaths> {
    let storage_dir = get_opencode_dir().join("storage");
    let session_files = find_opencode_session_files(session_id)?;
    let message_dir = storage_dir.join("message").join(session_id);
    let mut message_ids = database_message_ids.clone();
    if message_dir.is_dir() {
        for entry in std::fs::read_dir(&message_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
                if let Some(message_id) = path.file_stem().and_then(|stem| stem.to_str()) {
                    message_ids.insert(message_id.to_string());
                }
            }
        }
    }

    let mut part_dirs = Vec::new();
    let parts_root = storage_dir.join("part");
    if parts_root.is_dir() {
        for entry in std::fs::read_dir(&parts_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let message_id = entry.file_name().to_string_lossy().to_string();
            if message_ids.contains(&message_id)
                || opencode_part_dir_belongs_to_session(&path, session_id)?
            {
                part_dirs.push(path);
            }
        }
    }
    part_dirs.sort();

    Ok(OpenCodeMutationPaths {
        session_files,
        message_dir,
        part_dirs,
    })
}

pub(super) fn find_opencode_session_files(session_id: &str) -> Result<Vec<PathBuf>> {
    let root = get_opencode_dir().join("storage").join("session");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in WalkDir::new(&root).max_depth(3).follow_links(false) {
        let entry = entry
            .with_context(|| format!("Failed to scan OpenCode sessions: {}", root.display()))?;
        let path = entry.path();
        if (entry.file_type().is_file() || entry.file_type().is_symlink())
            && path.extension().and_then(|extension| extension.to_str()) == Some("json")
            && path.file_stem().and_then(|stem| stem.to_str()) == Some(session_id)
        {
            paths.push(path.to_path_buf());
        }
    }
    paths.sort();
    Ok(paths)
}

pub(super) fn opencode_part_dir_belongs_to_session(path: &Path, session_id: &str) -> Result<bool> {
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let part_path = entry.path();
        if part_path
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("json")
        {
            continue;
        }
        let Ok(content) = std::fs::read(&part_path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&content) else {
            continue;
        };
        if value.get("sessionID").and_then(Value::as_str) == Some(session_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn capture_opencode_filesystem_entry(
    source_path: &Path,
    relative_path: PathBuf,
    absent_kind: OpenCodeFilesystemEntryKind,
    backup_path: &Path,
) -> Result<OpenCodeFilesystemEntryBackup> {
    let metadata = match std::fs::symlink_metadata(source_path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let Some(metadata) = metadata else {
        return Ok(OpenCodeFilesystemEntryBackup {
            source_path: source_path.to_path_buf(),
            relative_path,
            kind: absent_kind,
            present: false,
        });
    };
    let kind = if metadata.file_type().is_symlink() {
        OpenCodeFilesystemEntryKind::Symlink
    } else if metadata.is_file() {
        OpenCodeFilesystemEntryKind::File
    } else if metadata.is_dir() {
        OpenCodeFilesystemEntryKind::Directory
    } else {
        anyhow::bail!(
            "OpenCode source contains unsupported filesystem entry: {}",
            source_path.display()
        );
    };
    let destination = backup_path.join(&relative_path);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    copy_opencode_filesystem_entry(source_path, &destination)?;
    Ok(OpenCodeFilesystemEntryBackup {
        source_path: source_path.to_path_buf(),
        relative_path,
        kind,
        present: true,
    })
}

pub(super) fn restore_opencode_filesystem_entry(
    backup_path: &Path,
    entry: &OpenCodeFilesystemEntryBackup,
) -> Result<()> {
    remove_opencode_filesystem_entry(&entry.source_path)?;
    if !entry.present {
        return Ok(());
    }
    let source = backup_path.join(&entry.relative_path);
    if let Some(parent) = entry.source_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    copy_opencode_filesystem_entry(&source, &entry.source_path).with_context(|| {
        format!(
            "Failed to restore OpenCode filesystem entry: {}",
            entry.source_path.display()
        )
    })
}

pub(super) fn copy_opencode_filesystem_entry(source: &Path, destination: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        copy_opencode_symlink(source, destination)?;
        return Ok(());
    }
    if metadata.is_file() {
        std::fs::copy(source, destination)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        anyhow::bail!(
            "OpenCode source contains unsupported filesystem entry: {}",
            source.display()
        );
    }

    std::fs::create_dir(destination)?;
    for entry in WalkDir::new(source).follow_links(false).into_iter().skip(1) {
        let entry = entry
            .with_context(|| format!("Failed to walk OpenCode source: {}", source.display()))?;
        let relative = entry.path().strip_prefix(source)?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir(&target)?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &target)?;
        } else if entry.file_type().is_symlink() {
            copy_opencode_symlink(entry.path(), &target)?;
        } else {
            anyhow::bail!(
                "OpenCode source contains unsupported filesystem entry: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

pub(super) fn remove_opencode_filesystem_entry(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub(super) fn path_lexists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

#[cfg(unix)]
pub(super) fn copy_opencode_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = std::fs::read_link(source)?;
    std::os::unix::fs::symlink(target, destination)?;
    Ok(())
}

#[cfg(windows)]
pub(super) fn copy_opencode_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = std::fs::read_link(source)?;
    let resolved_target = source
        .parent()
        .map(|parent| parent.join(&target))
        .unwrap_or_else(|| target.clone());
    if resolved_target.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination)?;
    } else {
        std::os::windows::fs::symlink_file(target, destination)?;
    }
    Ok(())
}
