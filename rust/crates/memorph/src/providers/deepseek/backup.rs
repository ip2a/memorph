use super::{get_session_index_path, get_state_db_path, PROVIDER_ID};
use crate::provider::{ProviderSessionBackup, ProviderSourceMutation};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const DEEPSEEK_BACKUP_FORMAT: &str = "deepseek-session-backup-v1";
const DEEPSEEK_BACKUP_MIME: &str = "application/vnd.memorph.deepseek-session-backup";
const DEEPSEEK_BACKUP_DB_PATH: &str = "sqlite/deepseek-session.db";
const DEEPSEEK_INDEX_BACKUP_PATH: &str = "files/session_index.jsonl";
const DEEPSEEK_TABLES: [&str; 4] = ["threads", "messages", "checkpoints", "thread_dynamic_tools"];

#[cfg(test)]
static TEST_DEEPSEEK_BACKUP_FAILURE: std::sync::OnceLock<std::sync::Mutex<bool>> =
    std::sync::OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeepseekSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    db_path: PathBuf,
    sqlite_tables: Vec<DeepseekSqliteTableManifest>,
    session_index: Option<DeepseekFileManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeepseekSqliteTableManifest {
    table: String,
    columns: Vec<String>,
    row_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeepseekFileManifest {
    source_path: PathBuf,
    relative_path: PathBuf,
    present: bool,
    byte_len: u64,
    sha256: String,
}

enum SessionIndexRestore {
    Noop,
    Remove(PathBuf),
    Write(PathBuf, Vec<u8>),
}

pub(super) fn create_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    let db_path = get_state_db_path();
    let source_path = db_path.canonicalize().with_context(|| {
        format!(
            "Failed to resolve DeepSeek state database: {}",
            db_path.display()
        )
    })?;
    let mut conn = Connection::open(&source_path).with_context(|| {
        format!(
            "Failed to open DeepSeek state database: {}",
            source_path.display()
        )
    })?;
    validate_mutation_source(&conn, mutation, session_id)?;

    let provider_backup_root = backup_root.join(PROVIDER_ID);
    std::fs::create_dir_all(&provider_backup_root).with_context(|| {
        format!(
            "Failed to create DeepSeek backup root: {}",
            provider_backup_root.display()
        )
    })?;
    let backup_path = provider_backup_root.join(operation_id);
    std::fs::create_dir(&backup_path).with_context(|| {
        format!(
            "Failed to create DeepSeek session backup: {}",
            backup_path.display()
        )
    })?;

    let create_result = (|| -> Result<ProviderSessionBackup> {
        std::fs::create_dir(backup_path.join("sqlite"))?;
        let sqlite_tables = capture_sqlite_backup(
            &mut conn,
            mutation,
            session_id,
            &backup_path.join(DEEPSEEK_BACKUP_DB_PATH),
        )?;
        let session_index = if mutation == ProviderSourceMutation::Rename {
            std::fs::create_dir(backup_path.join("files"))?;
            Some(capture_session_index(&source_path, &backup_path)?)
        } else {
            None
        };
        fail_deepseek_backup_after_capture()?;

        let metadata = DeepseekSessionBackupMetadata {
            version: 1,
            provider_id: PROVIDER_ID.to_string(),
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            db_path: source_path.clone(),
            sqlite_tables,
            session_index,
        };
        std::fs::write(
            backup_path.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata)?,
        )?;

        Ok(ProviderSessionBackup {
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            source_path,
            backup_path: backup_path.clone(),
            restore_hint:
                "Restore this backup with memorph's DeepSeek native session restore flow before reopening DeepSeek."
                    .to_string(),
            mime_type: DEEPSEEK_BACKUP_MIME.to_string(),
            format: DEEPSEEK_BACKUP_FORMAT.to_string(),
            artifact_metadata: serde_json::json!({
                "role": "deepseek_prewrite_session_backup",
                "mutation": mutation,
                "sqlite_table_count": metadata.sqlite_tables.len(),
                "session_index_captured": metadata.session_index.is_some(),
            }),
            restore_metadata: serde_json::json!({
                "restore_mode": "deepseek_session_restore",
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
    if backup.format != DEEPSEEK_BACKUP_FORMAT {
        anyhow::bail!(
            "Unsupported DeepSeek session backup format: {}",
            backup.format
        );
    }
    if backup.mime_type != DEEPSEEK_BACKUP_MIME {
        anyhow::bail!(
            "Unsupported DeepSeek session backup MIME type: {}",
            backup.mime_type
        );
    }

    let metadata_path = backup.backup_path.join("metadata.json");
    let metadata: DeepseekSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read DeepSeek backup metadata: {}",
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
            "DeepSeek backup metadata does not match the registered restore context: {}",
            backup.backup_path.display()
        );
    }
    if (metadata.mutation == ProviderSourceMutation::Rename) != metadata.session_index.is_some() {
        anyhow::bail!("DeepSeek backup has an invalid session index restore contract");
    }
    let current_source_path = get_state_db_path().canonicalize().with_context(|| {
        format!(
            "Failed to resolve current DeepSeek state database: {}",
            get_state_db_path().display()
        )
    })?;
    if current_source_path != metadata.db_path {
        anyhow::bail!(
            "DeepSeek backup source database does not match the current state database: captured={} current={}",
            metadata.db_path.display(),
            current_source_path.display()
        );
    }

    let backup_db_path = backup.backup_path.join(DEEPSEEK_BACKUP_DB_PATH);
    validate_sqlite_backup(
        &metadata.db_path,
        &backup_db_path,
        metadata.mutation,
        &metadata.provider_session_id,
        &metadata.sqlite_tables,
    )?;
    let index_restore = metadata
        .session_index
        .as_ref()
        .map(|index| {
            let expected_source_path = metadata
                .db_path
                .parent()
                .context("DeepSeek database path has no parent directory")?
                .join("session_index.jsonl");
            if index.source_path != expected_source_path {
                anyhow::bail!("DeepSeek session index source path is invalid");
            }
            prepare_session_index_restore(&backup.backup_path, index, &metadata.provider_session_id)
        })
        .transpose()?
        .unwrap_or(SessionIndexRestore::Noop);

    restore_sqlite_backup(
        &metadata.db_path,
        &backup_db_path,
        metadata.mutation,
        &metadata.provider_session_id,
        &metadata.sqlite_tables,
    )?;
    apply_session_index_restore(index_restore)?;
    Ok(())
}

pub(super) fn validate_mutation_source(
    conn: &Connection,
    mutation: ProviderSourceMutation,
    session_id: &str,
) -> Result<()> {
    validate_source_schema(conn)?;
    let thread_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM threads WHERE id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    if thread_count != 1 {
        anyhow::bail!("DeepSeek thread not found or ambiguous: {session_id}");
    }
    if mutation == ProviderSourceMutation::Rename {
        validate_live_session_index()?;
    }
    Ok(())
}

fn validate_source_schema(conn: &Connection) -> Result<()> {
    for (table, selection_column) in [
        ("threads", "id"),
        ("messages", "thread_id"),
        ("checkpoints", "thread_id"),
        ("thread_dynamic_tools", "thread_id"),
    ] {
        let columns = table_columns(conn, "main", table)?;
        if !columns.iter().any(|column| column == selection_column) {
            anyhow::bail!(
                "DeepSeek table {table} must contain selection column {selection_column}"
            );
        }
    }
    for column in ["title", "preview", "updated_at"] {
        if !table_columns(conn, "main", "threads")?
            .iter()
            .any(|candidate| candidate == column)
        {
            anyhow::bail!("DeepSeek threads table must contain rename column {column}");
        }
    }
    if !table_has_unique_column(conn, "threads", "id")? {
        anyhow::bail!("DeepSeek threads table does not enforce a unique id");
    }
    validate_no_mutating_triggers(conn)?;
    validate_foreign_key_delete_effects(conn)?;
    Ok(())
}

fn validate_no_mutating_triggers(conn: &Connection) -> Result<()> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM sqlite_schema
         WHERE type = 'trigger' AND tbl_name IN (
             'threads', 'messages', 'checkpoints', 'thread_dynamic_tools'
         )",
        [],
        |row| row.get(0),
    )?;
    if count != 0 {
        anyhow::bail!("DeepSeek managed tables contain unsupported mutation triggers");
    }
    Ok(())
}

fn validate_foreign_key_delete_effects(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let tables = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for child_table in tables {
        let mut foreign_key_stmt = conn.prepare(&format!(
            "PRAGMA foreign_key_list({})",
            quote_identifier(&child_table)
        ))?;
        let foreign_keys = foreign_key_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(6)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (parent_table, child_column, parent_column, on_delete) in foreign_keys {
            if !DEEPSEEK_TABLES.contains(&parent_table.as_str())
                || matches!(on_delete.as_str(), "NO ACTION" | "RESTRICT")
            {
                continue;
            }
            let managed_thread_cascade = matches!(
                child_table.as_str(),
                "messages" | "checkpoints" | "thread_dynamic_tools"
            ) && parent_table == "threads"
                && child_column == "thread_id"
                && parent_column.as_deref().is_none_or(|column| column == "id")
                && on_delete == "CASCADE";
            if !managed_thread_cascade {
                anyhow::bail!(
                    "DeepSeek table {child_table} has unsupported ON DELETE {on_delete} behavior referencing {parent_table}"
                );
            }
        }
    }
    Ok(())
}

fn capture_sqlite_backup(
    conn: &mut Connection,
    mutation: ProviderSourceMutation,
    session_id: &str,
    backup_db_path: &Path,
) -> Result<Vec<DeepseekSqliteTableManifest>> {
    if backup_db_path.exists() {
        anyhow::bail!(
            "DeepSeek SQLite backup already exists: {}",
            backup_db_path.display()
        );
    }
    let backup_db_path_text = backup_db_path.to_str().with_context(|| {
        format!(
            "DeepSeek SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    conn.execute(
        "ATTACH DATABASE ?1 AS memorph_backup",
        [backup_db_path_text],
    )?;
    let capture_result = (|| -> Result<Vec<DeepseekSqliteTableManifest>> {
        let tx = conn.transaction()?;
        match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
                tx.execute(
                    "CREATE TABLE memorph_backup.threads AS
                     SELECT * FROM main.threads WHERE id = ?1",
                    [session_id],
                )?;
                for table in ["messages", "checkpoints", "thread_dynamic_tools"] {
                    tx.execute(
                        &format!(
                            "CREATE TABLE memorph_backup.{} AS
                             SELECT * FROM main.{} WHERE thread_id = ?1",
                            quote_identifier(table),
                            quote_identifier(table)
                        ),
                        [session_id],
                    )?;
                }
            }
            ProviderSourceMutation::Rename => {
                tx.execute(
                    "CREATE TABLE memorph_backup.threads AS
                     SELECT * FROM main.threads WHERE id = ?1",
                    [session_id],
                )?;
            }
        }
        let tables: &[&str] = match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => &DEEPSEEK_TABLES,
            ProviderSourceMutation::Rename => &["threads"],
        };
        let manifests = tables
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
    manifests: &[DeepseekSqliteTableManifest],
) -> Result<()> {
    if !backup_db_path.is_file() {
        anyhow::bail!(
            "DeepSeek SQLite backup does not exist: {}",
            backup_db_path.display()
        );
    }
    let mut conn = Connection::open(db_path).with_context(|| {
        format!(
            "Failed to open DeepSeek state database: {}",
            db_path.display()
        )
    })?;
    validate_source_schema(&conn)?;
    let backup_db_path_text = backup_db_path.to_str().with_context(|| {
        format!(
            "DeepSeek SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    conn.execute(
        "ATTACH DATABASE ?1 AS memorph_backup",
        [backup_db_path_text],
    )?;
    let restore_result = (|| -> Result<()> {
        validate_manifest_schemas(&conn, mutation, manifests)?;
        validate_backup_selection(&conn, mutation, session_id, manifests)?;
        let tx = conn.transaction()?;
        match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
                restore_deleted_rows(&tx, session_id, manifests)?
            }
            ProviderSourceMutation::Rename => restore_renamed_fields(&tx, session_id)?,
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

fn validate_sqlite_backup(
    db_path: &Path,
    backup_db_path: &Path,
    mutation: ProviderSourceMutation,
    session_id: &str,
    manifests: &[DeepseekSqliteTableManifest],
) -> Result<()> {
    if !backup_db_path.is_file() {
        anyhow::bail!(
            "DeepSeek SQLite backup does not exist: {}",
            backup_db_path.display()
        );
    }
    let conn = Connection::open(db_path).with_context(|| {
        format!(
            "Failed to open DeepSeek state database: {}",
            db_path.display()
        )
    })?;
    validate_source_schema(&conn)?;
    let backup_db_path_text = backup_db_path.to_str().with_context(|| {
        format!(
            "DeepSeek SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    conn.execute(
        "ATTACH DATABASE ?1 AS memorph_backup",
        [backup_db_path_text],
    )?;
    let validate_result = (|| -> Result<()> {
        validate_manifest_schemas(&conn, mutation, manifests)?;
        validate_backup_selection(&conn, mutation, session_id, manifests)
    })();
    let detach_result = conn.execute_batch("DETACH DATABASE memorph_backup;");
    match validate_result {
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

fn restore_deleted_rows(
    tx: &Transaction<'_>,
    session_id: &str,
    manifests: &[DeepseekSqliteTableManifest],
) -> Result<()> {
    for table in ["messages", "checkpoints", "thread_dynamic_tools"] {
        tx.execute(
            &format!(
                "DELETE FROM main.{} WHERE thread_id = ?1",
                quote_identifier(table)
            ),
            [session_id],
        )?;
    }
    tx.execute("DELETE FROM main.threads WHERE id = ?1", [session_id])?;
    insert_all_backup_rows(tx, manifest_for_table(manifests, "threads")?)?;
    for table in ["messages", "checkpoints", "thread_dynamic_tools"] {
        insert_all_backup_rows(tx, manifest_for_table(manifests, table)?)?;
    }
    Ok(())
}

fn restore_renamed_fields(tx: &Transaction<'_>, session_id: &str) -> Result<()> {
    let exists = tx
        .query_row(
            "SELECT 1 FROM main.threads WHERE id = ?1",
            [session_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(());
    }
    let updated = tx.execute(
        "UPDATE main.threads
         SET title = (
                 SELECT backup.title FROM memorph_backup.threads AS backup
                 WHERE backup.id = main.threads.id
             ),
             preview = (
                 SELECT backup.preview FROM memorph_backup.threads AS backup
                 WHERE backup.id = main.threads.id
             ),
             updated_at = (
                 SELECT backup.updated_at FROM memorph_backup.threads AS backup
                 WHERE backup.id = main.threads.id
             )
         WHERE id = ?1
           AND EXISTS (
               SELECT 1 FROM memorph_backup.threads AS backup
               WHERE backup.id = main.threads.id
           )",
        [session_id],
    )?;
    if updated != 1 {
        anyhow::bail!("DeepSeek rename restore could not update the captured thread");
    }
    Ok(())
}

fn capture_session_index(db_path: &Path, backup_path: &Path) -> Result<DeepseekFileManifest> {
    let source_path = db_path
        .parent()
        .context("DeepSeek database path has no parent directory")?
        .join("session_index.jsonl");
    let relative_path = PathBuf::from(DEEPSEEK_INDEX_BACKUP_PATH);
    if !source_path.exists() {
        return Ok(DeepseekFileManifest {
            source_path,
            relative_path,
            present: false,
            byte_len: 0,
            sha256: sha256_bytes(&[]),
        });
    }
    if !source_path.is_file() {
        anyhow::bail!(
            "DeepSeek session index is not a file: {}",
            source_path.display()
        );
    }
    let bytes = std::fs::read(&source_path)?;
    validate_index_prefix(&bytes)?;
    std::fs::write(backup_path.join(&relative_path), &bytes)?;
    Ok(DeepseekFileManifest {
        source_path,
        relative_path,
        present: true,
        byte_len: u64::try_from(bytes.len()).context("DeepSeek index is too large")?,
        sha256: sha256_bytes(&bytes),
    })
}

fn validate_live_session_index() -> Result<()> {
    let path = get_session_index_path();
    if !path.exists() {
        return Ok(());
    }
    if !path.is_file() {
        anyhow::bail!("DeepSeek session index is not a file: {}", path.display());
    }
    validate_index_prefix(&std::fs::read(path)?)
}

fn validate_index_prefix(bytes: &[u8]) -> Result<()> {
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        anyhow::bail!("DeepSeek session index does not end at a complete JSONL record");
    }
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_slice(line)
            .context("DeepSeek session index contains invalid JSONL")?;
        if !value.is_object() {
            anyhow::bail!("DeepSeek session index contains a non-object record");
        }
    }
    Ok(())
}

fn prepare_session_index_restore(
    backup_path: &Path,
    manifest: &DeepseekFileManifest,
    session_id: &str,
) -> Result<SessionIndexRestore> {
    if manifest.relative_path != Path::new(DEEPSEEK_INDEX_BACKUP_PATH) {
        anyhow::bail!("DeepSeek session index backup path is invalid");
    }
    let original = if manifest.present {
        let bytes = std::fs::read(backup_path.join(&manifest.relative_path))?;
        validate_file_manifest(manifest, &bytes)?;
        validate_index_prefix(&bytes)?;
        bytes
    } else {
        if manifest.byte_len != 0 || manifest.sha256 != sha256_bytes(&[]) {
            anyhow::bail!("DeepSeek absent session index manifest is invalid");
        }
        Vec::new()
    };

    let current = match std::fs::read(&manifest.source_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !manifest.present => {
            return Ok(SessionIndexRestore::Noop)
        }
        Err(error) => return Err(error.into()),
    };
    if !current.starts_with(&original) {
        anyhow::bail!("DeepSeek session index no longer has the captured byte prefix");
    }
    let suffix = &current[original.len()..];
    if suffix.is_empty() {
        return Ok(SessionIndexRestore::Noop);
    }
    if !suffix.ends_with(b"\n") {
        anyhow::bail!("DeepSeek session index suffix contains an incomplete JSONL record");
    }

    let mut target_count = 0;
    let mut kept_suffix = Vec::with_capacity(suffix.len());
    for raw_line in suffix.split_inclusive(|byte| *byte == b'\n') {
        let content = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        let content = content.strip_suffix(b"\r").unwrap_or(content);
        if content.is_empty() {
            kept_suffix.extend_from_slice(raw_line);
            continue;
        }
        let value: Value = serde_json::from_slice(content)
            .context("DeepSeek session index suffix contains invalid JSONL")?;
        let record_thread_id = value
            .get("thread_id")
            .and_then(Value::as_str)
            .context("DeepSeek session index suffix record has no thread_id")?;
        if record_thread_id == session_id {
            target_count += 1;
        } else {
            kept_suffix.extend_from_slice(raw_line);
        }
    }
    if target_count > 1 {
        anyhow::bail!(
            "DeepSeek session index suffix contains multiple target records for {session_id}"
        );
    }
    if target_count == 0 {
        return Ok(SessionIndexRestore::Noop);
    }

    let mut restored = original;
    restored.extend_from_slice(&kept_suffix);
    if restored.is_empty() && !manifest.present {
        Ok(SessionIndexRestore::Remove(manifest.source_path.clone()))
    } else {
        Ok(SessionIndexRestore::Write(
            manifest.source_path.clone(),
            restored,
        ))
    }
}

fn apply_session_index_restore(restore: SessionIndexRestore) -> Result<()> {
    match restore {
        SessionIndexRestore::Noop => {}
        SessionIndexRestore::Remove(path) => match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        },
        SessionIndexRestore::Write(path, bytes) => std::fs::write(path, bytes)?,
    }
    Ok(())
}

fn validate_file_manifest(manifest: &DeepseekFileManifest, bytes: &[u8]) -> Result<()> {
    if u64::try_from(bytes.len()).ok() != Some(manifest.byte_len)
        || sha256_bytes(bytes) != manifest.sha256
    {
        anyhow::bail!("DeepSeek session index backup does not match its manifest");
    }
    Ok(())
}

fn validate_manifest_schemas(
    conn: &Connection,
    mutation: ProviderSourceMutation,
    manifests: &[DeepseekSqliteTableManifest],
) -> Result<()> {
    let expected: HashSet<&str> = match mutation {
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
            DEEPSEEK_TABLES.into_iter().collect()
        }
        ProviderSourceMutation::Rename => HashSet::from(["threads"]),
    };
    let actual = manifests
        .iter()
        .map(|manifest| manifest.table.as_str())
        .collect::<HashSet<_>>();
    if manifests.len() != expected.len() || actual != expected {
        anyhow::bail!("DeepSeek backup table manifest is incomplete or duplicated");
    }
    for manifest in manifests {
        let source_columns = table_columns(conn, "main", &manifest.table)?;
        let backup_columns = table_columns(conn, "memorph_backup", &manifest.table)?;
        if source_columns != manifest.columns || backup_columns != manifest.columns {
            anyhow::bail!(
                "DeepSeek backup schema does not match source table {}",
                manifest.table
            );
        }
        if table_row_count(conn, "memorph_backup", &manifest.table)? != manifest.row_count {
            anyhow::bail!(
                "DeepSeek backup row count does not match manifest for {}",
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
    manifests: &[DeepseekSqliteTableManifest],
) -> Result<()> {
    if manifest_for_table(manifests, "threads")?.row_count != 1 {
        anyhow::bail!("DeepSeek backup does not contain exactly one thread row");
    }
    let thread_ids = selected_values(conn, "threads", "id")?;
    if thread_ids != vec![session_id.to_string()] {
        anyhow::bail!("DeepSeek backup contains a thread outside the target session");
    }
    if matches!(
        mutation,
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
    ) {
        for table in ["messages", "checkpoints", "thread_dynamic_tools"] {
            if selected_values(conn, table, "thread_id")?
                .iter()
                .any(|value| value != session_id)
            {
                anyhow::bail!(
                    "DeepSeek backup table {table} contains rows outside the target session"
                );
            }
        }
    }
    Ok(())
}

fn selected_values(conn: &Connection, table: &str, column: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM memorph_backup.{}",
        quote_identifier(column),
        quote_identifier(table)
    ))?;
    let values = stmt
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(anyhow::Error::from)?;
    Ok(values)
}

fn table_manifest(
    conn: &Connection,
    schema: &str,
    table: &str,
) -> Result<DeepseekSqliteTableManifest> {
    Ok(DeepseekSqliteTableManifest {
        table: table.to_string(),
        columns: table_columns(conn, schema, table)?,
        row_count: table_row_count(conn, schema, table)?,
    })
}

fn manifest_for_table<'a>(
    manifests: &'a [DeepseekSqliteTableManifest],
    table: &str,
) -> Result<&'a DeepseekSqliteTableManifest> {
    manifests
        .iter()
        .find(|manifest| manifest.table == table)
        .with_context(|| format!("DeepSeek backup manifest is missing table {table}"))
}

fn table_columns(conn: &Connection, schema: &str, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "PRAGMA {}.table_info({})",
        quote_identifier(schema),
        quote_identifier(table)
    ))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if columns.is_empty() {
        anyhow::bail!("DeepSeek SQLite table does not exist: {schema}.{table}");
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
    usize::try_from(count).context("DeepSeek backup row count is negative")
}

fn table_has_unique_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut table_info = conn.prepare(&format!(
        "PRAGMA main.table_info({})",
        quote_identifier(table)
    ))?;
    let primary_key_columns = table_info
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?
        .filter_map(|row| match row {
            Ok((name, position)) if position > 0 => Some(Ok((name, position))),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if primary_key_columns == vec![(column.to_string(), 1)] {
        return Ok(true);
    }

    let mut indexes = conn.prepare(&format!(
        "PRAGMA main.index_list({})",
        quote_identifier(table)
    ))?;
    let indexes = indexes
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(2)? != 0))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (index, unique) in indexes {
        if !unique {
            continue;
        }
        let mut index_info = conn.prepare(&format!(
            "PRAGMA main.index_info({})",
            quote_identifier(&index)
        ))?;
        let columns = index_info
            .query_map([], |row| row.get::<_, Option<String>>(2))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if columns == vec![Some(column.to_string())] {
            return Ok(true);
        }
    }
    Ok(false)
}

fn insert_all_backup_rows(
    tx: &Transaction<'_>,
    manifest: &DeepseekSqliteTableManifest,
) -> Result<()> {
    let columns = manifest
        .columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
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

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
pub(super) fn set_test_backup_failure(enabled: bool) {
    *TEST_DEEPSEEK_BACKUP_FAILURE
        .get_or_init(|| std::sync::Mutex::new(false))
        .lock()
        .expect("test DeepSeek backup failure lock") = enabled;
}

#[cfg(test)]
fn fail_deepseek_backup_after_capture() -> Result<()> {
    let mut enabled = TEST_DEEPSEEK_BACKUP_FAILURE
        .get_or_init(|| std::sync::Mutex::new(false))
        .lock()
        .expect("test DeepSeek backup failure lock");
    if *enabled {
        *enabled = false;
        anyhow::bail!("injected DeepSeek backup failure after native capture");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_deepseek_backup_after_capture() -> Result<()> {
    Ok(())
}
