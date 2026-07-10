use crate::provider::{ProviderSessionBackup, ProviderSourceMutation};
use crate::providers::cursor::db::{global_state_db_path, key_prefix_bounds};
use anyhow::{Context, Result};
use rusqlite::types::Value as SqliteValue;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use std::path::Path;

const PROVIDER_ID: &str = "cursor";
const CURSOR_BACKUP_FORMAT: &str = "cursor-session-backup-v1";
const CURSOR_BACKUP_MIME: &str = "application/vnd.memorph.cursor-session-backup";
const CURSOR_BACKUP_DB_PATH: &str = "sqlite/cursor-session.db";
const COMPOSER_INDEX_KEY: &str = "composer.composerHeaders";
const CURSOR_TABLES: [&str; 2] = ["cursorDiskKV", "ItemTable"];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CursorSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    db_path: std::path::PathBuf,
    sqlite_tables: Vec<CursorSqliteTableManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CursorSqliteTableManifest {
    table: String,
    columns: Vec<String>,
    row_count: usize,
}

pub(super) fn create_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    let db_path = global_state_db_path()?;
    let source_path = db_path.canonicalize().with_context(|| {
        format!(
            "Failed to resolve Cursor state database: {}",
            db_path.display()
        )
    })?;
    let mut conn = Connection::open(&source_path).with_context(|| {
        format!(
            "Failed to open Cursor state database: {}",
            source_path.display()
        )
    })?;
    validate_source_schema(&conn)?;
    let composer_key = composer_key(session_id);
    let composer_present = conn
        .query_row(
            "SELECT 1 FROM cursorDiskKV WHERE key = ?1",
            [&composer_key],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !composer_present {
        anyhow::bail!("Cursor composer not found: {session_id}");
    }

    let provider_backup_root = backup_root.join(PROVIDER_ID);
    std::fs::create_dir_all(&provider_backup_root).with_context(|| {
        format!(
            "Failed to create Cursor backup root: {}",
            provider_backup_root.display()
        )
    })?;
    let backup_path = provider_backup_root.join(operation_id);
    std::fs::create_dir(&backup_path).with_context(|| {
        format!(
            "Failed to create Cursor session backup: {}",
            backup_path.display()
        )
    })?;
    let create_result = (|| -> Result<ProviderSessionBackup> {
        std::fs::create_dir(backup_path.join("sqlite"))?;
        let sqlite_tables = capture_sqlite_backup(
            &mut conn,
            mutation,
            session_id,
            &backup_path.join(CURSOR_BACKUP_DB_PATH),
        )?;

        let metadata = CursorSessionBackupMetadata {
            version: 1,
            provider_id: PROVIDER_ID.to_string(),
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            db_path: source_path.clone(),
            sqlite_tables,
        };
        std::fs::write(
            backup_path.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata)?,
        )
        .with_context(|| {
            format!(
                "Failed to write Cursor backup metadata: {}",
                backup_path.display()
            )
        })?;

        Ok(ProviderSessionBackup {
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            source_path,
            backup_path: backup_path.clone(),
            restore_hint:
                "Restore this backup with memorph's Cursor native session restore flow before reopening Cursor."
                    .to_string(),
            mime_type: CURSOR_BACKUP_MIME.to_string(),
            format: CURSOR_BACKUP_FORMAT.to_string(),
            artifact_metadata: serde_json::json!({
                "role": "cursor_prewrite_session_backup",
                "mutation": mutation,
                "sqlite_table_count": metadata.sqlite_tables.len(),
            }),
            restore_metadata: serde_json::json!({
                "restore_mode": "cursor_session_restore",
                "metadata_file": "metadata.json",
                "mutation": mutation,
            }),
        })
    })();
    if create_result.is_err() {
        let _ = std::fs::remove_dir_all(&backup_path);
    }
    create_result
}

pub(super) fn restore_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != CURSOR_BACKUP_FORMAT {
        anyhow::bail!(
            "Unsupported Cursor session backup format: {}",
            backup.format
        );
    }
    if backup.mime_type != CURSOR_BACKUP_MIME {
        anyhow::bail!(
            "Unsupported Cursor session backup MIME type: {}",
            backup.mime_type
        );
    }

    let metadata_path = backup.backup_path.join("metadata.json");
    let metadata: CursorSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read Cursor backup metadata: {}",
                metadata_path.display()
            )
        })?)?;
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.mutation != backup.mutation
        || metadata.db_path != backup.source_path
    {
        anyhow::bail!(
            "Cursor backup metadata does not match the registered restore context: {}",
            backup.backup_path.display()
        );
    }

    restore_sqlite_backup(
        &metadata.db_path,
        &backup.backup_path.join(CURSOR_BACKUP_DB_PATH),
        metadata.mutation,
        &metadata.provider_session_id,
        &metadata.sqlite_tables,
    )
}

fn capture_sqlite_backup(
    conn: &mut Connection,
    mutation: ProviderSourceMutation,
    session_id: &str,
    backup_db_path: &Path,
) -> Result<Vec<CursorSqliteTableManifest>> {
    if backup_db_path.exists() {
        anyhow::bail!(
            "Cursor SQLite backup already exists: {}",
            backup_db_path.display()
        );
    }
    let backup_db_path_text = backup_db_path.to_str().with_context(|| {
        format!(
            "Cursor SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    conn.execute(
        "ATTACH DATABASE ?1 AS memorph_backup",
        [backup_db_path_text],
    )?;

    let capture_result = (|| -> Result<Vec<CursorSqliteTableManifest>> {
        let tx = conn.transaction()?;
        let composer_key = composer_key(session_id);
        let bubble_prefix = bubble_prefix(session_id);
        let (bubble_lower, bubble_upper) = key_prefix_bounds(&bubble_prefix);
        match mutation {
            ProviderSourceMutation::Delete => {
                tx.execute(
                    "CREATE TABLE memorph_backup.cursorDiskKV AS
                     SELECT * FROM main.cursorDiskKV
                     WHERE key = ?1 OR (key >= ?2 AND key < ?3)",
                    params![composer_key, bubble_lower, bubble_upper],
                )?;
            }
            ProviderSourceMutation::Rename => {
                tx.execute(
                    "CREATE TABLE memorph_backup.cursorDiskKV AS
                     SELECT * FROM main.cursorDiskKV WHERE key = ?1",
                    [composer_key],
                )?;
            }
        }
        tx.execute(
            "CREATE TABLE memorph_backup.ItemTable AS
             SELECT * FROM main.ItemTable WHERE key = ?1",
            [COMPOSER_INDEX_KEY],
        )?;
        let manifests = CURSOR_TABLES
            .iter()
            .map(|table| table_manifest(&tx, "memorph_backup", table))
            .collect::<Result<Vec<_>>>()?;
        validate_backup_selection(&tx, mutation, session_id, &manifests)?;
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

fn restore_sqlite_backup(
    db_path: &Path,
    backup_db_path: &Path,
    mutation: ProviderSourceMutation,
    session_id: &str,
    manifests: &[CursorSqliteTableManifest],
) -> Result<()> {
    if !backup_db_path.is_file() {
        anyhow::bail!(
            "Cursor SQLite backup does not exist: {}",
            backup_db_path.display()
        );
    }
    let mut conn = Connection::open(db_path).with_context(|| {
        format!(
            "Failed to open Cursor state database: {}",
            db_path.display()
        )
    })?;
    validate_source_schema(&conn)?;
    let backup_db_path_text = backup_db_path.to_str().with_context(|| {
        format!(
            "Cursor SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    conn.execute(
        "ATTACH DATABASE ?1 AS memorph_backup",
        [backup_db_path_text],
    )?;

    let restore_result = (|| -> Result<()> {
        validate_backup_selection(&conn, mutation, session_id, manifests)?;
        validate_manifest_schemas(&conn, manifests)?;
        let tx = conn.transaction()?;
        match mutation {
            ProviderSourceMutation::Delete => {
                restore_deleted_cursor_rows(&tx, session_id, manifests)?;
                restore_composer_index(&tx, mutation, session_id)?;
            }
            ProviderSourceMutation::Rename => {
                restore_composer_name(&tx, session_id)?;
                restore_composer_index(&tx, mutation, session_id)?;
            }
        }
        tx.commit()?;
        Ok(())
    })();
    let detach_result = conn.execute_batch("DETACH DATABASE memorph_backup;");
    match restore_result {
        Ok(()) => {
            detach_result?;
            Ok(())
        }
        Err(error) => {
            let _ = detach_result;
            Err(error)
        }
    }
}

fn restore_deleted_cursor_rows(
    tx: &Transaction<'_>,
    session_id: &str,
    manifests: &[CursorSqliteTableManifest],
) -> Result<()> {
    let bubble_prefix = bubble_prefix(session_id);
    let (bubble_lower, bubble_upper) = key_prefix_bounds(&bubble_prefix);
    tx.execute(
        "DELETE FROM main.cursorDiskKV
         WHERE key = ?1 OR (key >= ?2 AND key < ?3)",
        params![composer_key(session_id), bubble_lower, bubble_upper],
    )?;
    let manifest = manifest_for_table(manifests, "cursorDiskKV")?;
    insert_all_backup_rows(tx, manifest)?;
    Ok(())
}

fn restore_composer_name(tx: &Transaction<'_>, session_id: &str) -> Result<()> {
    let key = composer_key(session_id);
    let backup_value = stored_value(tx, "memorph_backup", "cursorDiskKV", &key)?
        .context("Cursor rename backup is missing the composer row")?;
    let current_value = stored_value(tx, "main", "cursorDiskKV", &key)?;
    let Some(current_value) = current_value else {
        return Ok(());
    };

    let backup_json = parse_json_value(&backup_value, "Cursor backup composer")?;
    let mut current_json = parse_json_value(&current_value, "current Cursor composer")?;
    let backup_object = backup_json
        .as_object()
        .context("Cursor backup composer is not a JSON object")?;
    let current_object = current_json
        .as_object_mut()
        .context("Current Cursor composer is not a JSON object")?;
    match backup_object.get("name") {
        Some(name) => {
            current_object.insert("name".to_string(), name.clone());
        }
        None => {
            current_object.remove("name");
        }
    }
    let restored_value = serialize_json_value(&current_value, &current_json)?;
    tx.execute(
        "UPDATE main.cursorDiskKV SET value = ?1 WHERE key = ?2",
        params![restored_value, key],
    )?;
    Ok(())
}

fn restore_composer_index(
    tx: &Transaction<'_>,
    mutation: ProviderSourceMutation,
    session_id: &str,
) -> Result<()> {
    let backup_value = stored_value(tx, "memorph_backup", "ItemTable", COMPOSER_INDEX_KEY)?;
    let current_value = stored_value(tx, "main", "ItemTable", COMPOSER_INDEX_KEY)?;
    match (backup_value, current_value) {
        (Some(backup_value), None) => {
            if mutation == ProviderSourceMutation::Rename {
                return Ok(());
            }
            let backup_json = parse_json_value(&backup_value, "Cursor backup composer index")?;
            let backup_entries = target_index_entries(&backup_json, session_id)?;
            if backup_entries.is_empty() {
                return Ok(());
            }
            insert_backup_row(tx, "ItemTable", COMPOSER_INDEX_KEY)?;
            let target_only_index = index_with_entries(
                &backup_json,
                backup_entries.into_iter().map(|(_, entry)| entry),
            )?;
            let restored_value = serialize_json_value(&backup_value, &target_only_index)?;
            tx.execute(
                "UPDATE main.ItemTable SET value = ?1 WHERE key = ?2",
                params![restored_value, COMPOSER_INDEX_KEY],
            )?;
            Ok(())
        }
        (Some(backup_value), Some(current_value)) => {
            let backup_json = parse_json_value(&backup_value, "Cursor backup composer index")?;
            let mut current_json =
                parse_json_value(&current_value, "current Cursor composer index")?;
            let backup_entries = target_index_entries(&backup_json, session_id)?;

            match mutation {
                ProviderSourceMutation::Delete => {
                    target_index_entries(&current_json, session_id)?;
                    let current_entries = index_entries_mut(&mut current_json)?;
                    current_entries.retain(|entry| !is_target_index_entry(entry, session_id));
                    for (position, entry) in backup_entries {
                        current_entries.insert(position.min(current_entries.len()), entry);
                    }
                }
                ProviderSourceMutation::Rename => {
                    let backup_entry = backup_entries.first().map(|(_, entry)| entry);
                    let Some(backup_entry) = backup_entry else {
                        return Ok(());
                    };
                    target_index_entries(&current_json, session_id)?;
                    let current_entries = index_entries_mut(&mut current_json)?;
                    let mut found = false;
                    for entry in current_entries
                        .iter_mut()
                        .filter(|entry| is_target_index_entry(entry, session_id))
                    {
                        found = true;
                        restore_json_field(entry, backup_entry, "name")?;
                    }
                    if !found {
                        if let Some((position, entry)) = backup_entries.into_iter().next() {
                            current_entries.insert(position.min(current_entries.len()), entry);
                        }
                    }
                }
            }

            let restored_value = serialize_json_value(&current_value, &current_json)?;
            tx.execute(
                "UPDATE main.ItemTable SET value = ?1 WHERE key = ?2",
                params![restored_value, COMPOSER_INDEX_KEY],
            )?;
            Ok(())
        }
        (None, Some(current_value)) => {
            if mutation == ProviderSourceMutation::Rename {
                return Ok(());
            }
            let mut current_json =
                parse_json_value(&current_value, "current Cursor composer index")?;
            target_index_entries(&current_json, session_id)?;
            index_entries_mut(&mut current_json)?
                .retain(|entry| !is_target_index_entry(entry, session_id));
            if is_empty_default_index(&current_json) {
                tx.execute(
                    "DELETE FROM main.ItemTable WHERE key = ?1",
                    [COMPOSER_INDEX_KEY],
                )?;
            } else {
                let restored_value = serialize_json_value(&current_value, &current_json)?;
                tx.execute(
                    "UPDATE main.ItemTable SET value = ?1 WHERE key = ?2",
                    params![restored_value, COMPOSER_INDEX_KEY],
                )?;
            }
            Ok(())
        }
        (None, None) => Ok(()),
    }
}

fn index_with_entries(
    template: &JsonValue,
    entries: impl IntoIterator<Item = JsonValue>,
) -> Result<JsonValue> {
    let mut index = template.clone();
    *index_entries_mut(&mut index)? = entries.into_iter().collect();
    Ok(index)
}

fn restore_json_field(
    current_entry: &mut JsonValue,
    backup_entry: &JsonValue,
    field: &str,
) -> Result<()> {
    let current_object = current_entry
        .as_object_mut()
        .context("Cursor composer index entry is not a JSON object")?;
    let backup_object = backup_entry
        .as_object()
        .context("Cursor backup composer index entry is missing")?;
    match backup_object.get(field) {
        Some(value) => {
            current_object.insert(field.to_string(), value.clone());
        }
        None => {
            current_object.remove(field);
        }
    }
    Ok(())
}

fn target_index_entries(index: &JsonValue, session_id: &str) -> Result<Vec<(usize, JsonValue)>> {
    let entries = index
        .get("allComposers")
        .and_then(JsonValue::as_array)
        .context("Cursor composer index does not contain an allComposers array")?;
    let matches = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| is_target_index_entry(entry, session_id))
        .map(|(position, entry)| (position, entry.clone()))
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        anyhow::bail!("Cursor composer index contains duplicate entries for {session_id}");
    }
    Ok(matches)
}

fn index_entries_mut(index: &mut JsonValue) -> Result<&mut Vec<JsonValue>> {
    index
        .get_mut("allComposers")
        .and_then(JsonValue::as_array_mut)
        .context("Cursor composer index does not contain an allComposers array")
}

fn is_target_index_entry(entry: &JsonValue, session_id: &str) -> bool {
    entry.get("composerId").and_then(JsonValue::as_str) == Some(session_id)
}

fn is_empty_default_index(index: &JsonValue) -> bool {
    index.as_object().is_some_and(|object| object.len() == 1)
        && index
            .get("allComposers")
            .and_then(JsonValue::as_array)
            .is_some_and(Vec::is_empty)
}

fn validate_source_schema(conn: &Connection) -> Result<()> {
    for table in CURSOR_TABLES {
        let columns = table_columns(conn, "main", table)?;
        if !columns.iter().any(|column| column == "key")
            || !columns.iter().any(|column| column == "value")
        {
            anyhow::bail!("Cursor table {table} must contain key and value columns");
        }
        if !table_has_unique_key(conn, table)? {
            anyhow::bail!("Cursor table {table} does not enforce a unique key");
        }
    }
    Ok(())
}

fn table_has_unique_key(conn: &Connection, table: &str) -> Result<bool> {
    let table_arg = quote_sqlite_string(table);
    let mut stmt = conn.prepare(&format!("PRAGMA main.index_list({table_arg})"))?;
    let indexes = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(2)? != 0))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (index, unique) in indexes {
        if !unique {
            continue;
        }
        let index_arg = quote_sqlite_string(&index);
        let mut index_stmt = conn.prepare(&format!("PRAGMA main.index_info({index_arg})"))?;
        let columns = index_stmt
            .query_map([], |row| row.get::<_, Option<String>>(2))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if columns == vec![Some("key".to_string())] {
            return Ok(true);
        }
    }
    Ok(false)
}

fn validate_manifest_schemas(
    conn: &Connection,
    manifests: &[CursorSqliteTableManifest],
) -> Result<()> {
    if manifests.len() != CURSOR_TABLES.len()
        || manifests
            .iter()
            .map(|manifest| manifest.table.as_str())
            .collect::<HashSet<_>>()
            != CURSOR_TABLES.into_iter().collect::<HashSet<_>>()
    {
        anyhow::bail!("Cursor backup table manifest is incomplete");
    }
    for manifest in manifests {
        let source_columns = table_columns(conn, "main", &manifest.table)?;
        let backup_columns = table_columns(conn, "memorph_backup", &manifest.table)?;
        if source_columns != manifest.columns || backup_columns != manifest.columns {
            anyhow::bail!(
                "Cursor backup schema does not match source table {}",
                manifest.table
            );
        }
        let row_count = table_row_count(conn, "memorph_backup", &manifest.table)?;
        if row_count != manifest.row_count {
            anyhow::bail!(
                "Cursor backup row count does not match manifest for {}",
                manifest.table
            );
        }
    }
    Ok(())
}

fn validate_backup_selection(
    conn: &Connection,
    mutation: ProviderSourceMutation,
    session_id: &str,
    manifests: &[CursorSqliteTableManifest],
) -> Result<()> {
    let disk_manifest = manifest_for_table(manifests, "cursorDiskKV")?;
    let item_manifest = manifest_for_table(manifests, "ItemTable")?;
    if disk_manifest.row_count == 0 || item_manifest.row_count > 1 {
        anyhow::bail!("Cursor backup row selection is invalid");
    }
    let composer_key = composer_key(session_id);
    let bubble_prefix = bubble_prefix(session_id);
    let mut stmt = conn.prepare("SELECT key FROM memorph_backup.cursorDiskKV ORDER BY key ASC")?;
    let keys = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if keys.iter().filter(|key| *key == &composer_key).count() != 1 {
        anyhow::bail!("Cursor backup does not contain exactly one composer row");
    }
    if keys.iter().any(|key| {
        key != &composer_key
            && (mutation != ProviderSourceMutation::Delete || !key.starts_with(&bubble_prefix))
    }) {
        anyhow::bail!("Cursor backup contains rows outside the target session");
    }
    if mutation == ProviderSourceMutation::Rename && keys.len() != 1 {
        anyhow::bail!("Cursor rename backup contains non-composer rows");
    }

    let item_keys = conn
        .prepare("SELECT key FROM memorph_backup.ItemTable")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if item_keys
        .iter()
        .any(|key| key.as_str() != COMPOSER_INDEX_KEY)
    {
        anyhow::bail!("Cursor backup contains an unexpected ItemTable row");
    }
    if let Some(index_value) =
        stored_value(conn, "memorph_backup", "ItemTable", COMPOSER_INDEX_KEY)?
    {
        let index = parse_json_value(&index_value, "Cursor backup composer index")?;
        target_index_entries(&index, session_id)?;
    }
    Ok(())
}

fn table_manifest(
    conn: &Connection,
    schema: &str,
    table: &str,
) -> Result<CursorSqliteTableManifest> {
    Ok(CursorSqliteTableManifest {
        table: table.to_string(),
        columns: table_columns(conn, schema, table)?,
        row_count: table_row_count(conn, schema, table)?,
    })
}

fn manifest_for_table<'a>(
    manifests: &'a [CursorSqliteTableManifest],
    table: &str,
) -> Result<&'a CursorSqliteTableManifest> {
    manifests
        .iter()
        .find(|manifest| manifest.table == table)
        .with_context(|| format!("Cursor backup manifest is missing table {table}"))
}

fn table_columns(conn: &Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    let schema = quote_identifier(schema);
    let table_arg = quote_sqlite_string(table);
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table_arg})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if columns.is_empty() {
        anyhow::bail!("Cursor SQLite table does not exist: {schema}.{table}");
    }
    Ok(columns)
}

fn table_row_count(conn: &Connection, schema: &str, table: &str) -> Result<usize> {
    let count: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM {}.{}",
            quote_identifier(schema),
            quote_identifier(table)
        ),
        [],
        |row| row.get(0),
    )?;
    usize::try_from(count).context("Cursor backup row count is negative")
}

fn insert_all_backup_rows(
    tx: &Transaction<'_>,
    manifest: &CursorSqliteTableManifest,
) -> Result<()> {
    let columns = quoted_columns(&manifest.columns);
    tx.execute(
        &format!(
            "INSERT INTO main.{table} ({columns})
             SELECT {columns} FROM memorph_backup.{table}",
            table = quote_identifier(&manifest.table)
        ),
        [],
    )?;
    Ok(())
}

fn insert_backup_row(tx: &Transaction<'_>, table: &str, key: &str) -> Result<()> {
    let columns = table_columns(tx, "memorph_backup", table)?;
    let columns = quoted_columns(&columns);
    tx.execute(
        &format!(
            "INSERT INTO main.{table} ({columns})
             SELECT {columns} FROM memorph_backup.{table} WHERE key = ?1",
            table = quote_identifier(table)
        ),
        [key],
    )?;
    Ok(())
}

fn stored_value(
    conn: &Connection,
    schema: &str,
    table: &str,
    key: &str,
) -> Result<Option<SqliteValue>> {
    conn.query_row(
        &format!(
            "SELECT value FROM {}.{} WHERE key = ?1",
            quote_identifier(schema),
            quote_identifier(table)
        ),
        [key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn parse_json_value(value: &SqliteValue, context: &str) -> Result<JsonValue> {
    match value {
        SqliteValue::Text(text) => {
            serde_json::from_str(text).with_context(|| format!("Failed to parse {context}"))
        }
        SqliteValue::Blob(bytes) => {
            serde_json::from_slice(bytes).with_context(|| format!("Failed to parse {context}"))
        }
        _ => anyhow::bail!("{context} is not stored as TEXT or BLOB"),
    }
}

fn serialize_json_value(template: &SqliteValue, value: &JsonValue) -> Result<SqliteValue> {
    match template {
        SqliteValue::Text(_) => Ok(SqliteValue::Text(serde_json::to_string(value)?)),
        SqliteValue::Blob(_) => Ok(SqliteValue::Blob(serde_json::to_vec(value)?)),
        _ => anyhow::bail!("Cursor JSON value template is not TEXT or BLOB"),
    }
}

fn composer_key(session_id: &str) -> String {
    format!("composerData:{session_id}")
}

fn bubble_prefix(session_id: &str) -> String {
    format!("bubbleId:{session_id}:")
}

fn quoted_columns(columns: &[String]) -> String {
    columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ")
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_sqlite_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
