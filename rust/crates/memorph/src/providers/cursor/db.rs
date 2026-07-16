use crate::provider::ProviderSourceFingerprint;
use anyhow::{Context, Result};
use rusqlite::types::Value as SqliteValue;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

#[cfg(test)]
static TEST_CURSOR_DB_PATH: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

/// Cross-platform Cursor data directory.
pub fn cursor_data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().context("Unable to locate user home directory")?;
        return Ok(home.join("Library/Application Support/Cursor"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| anyhow::anyhow!("APPDATA environment variable not found"))?;
        return Ok(PathBuf::from(appdata).join("Cursor"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().context("Unable to locate user home directory")?;
        return Ok(home.join(".config/Cursor"));
    }

    #[allow(unreachable_code)]
    Err(anyhow::anyhow!(
        "Cursor data directory not supported on this platform"
    ))
}

/// Path to the global storage database that holds all AI session data.
pub fn global_state_db_path() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_CURSOR_DB_PATH
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Cursor database path lock")
        .clone()
    {
        return Ok(path);
    }

    Ok(cursor_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("state.vscdb"))
}

/// Build a provider-owned locator for a Cursor composer in the current global database.
pub fn cursor_source_locator(composer_id: &str) -> Result<String> {
    anyhow::ensure!(
        !composer_id.is_empty(),
        "Cursor composer ID cannot be empty"
    );
    anyhow::ensure!(
        !composer_id.contains('#'),
        "Cursor composer ID cannot contain locator separators"
    );
    Ok(format!(
        "{}#composer={}",
        global_state_db_path()?.display(),
        composer_id
    ))
}

/// Parse a current Cursor database locator. Raw composer IDs are not locators.
pub fn parse_cursor_source_locator(source_locator: &str) -> Result<(PathBuf, String)> {
    let (database_path, composer_id) = source_locator
        .rsplit_once("#composer=")
        .context("Cursor source locator must use '<database>#composer=<composerId>'")?;
    anyhow::ensure!(
        !database_path.is_empty() && !composer_id.is_empty(),
        "Cursor source locator must include a database path and composer ID"
    );
    anyhow::ensure!(
        !composer_id.contains('#'),
        "Cursor source locator contains an invalid composer ID"
    );
    Ok((PathBuf::from(database_path), composer_id.to_string()))
}

fn open_read_only_db(path: &Path) -> Result<Connection> {
    if !path.exists() {
        anyhow::bail!("Cursor state database not found: {}", path.display());
    }
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).with_context(|| {
        format!(
            "Failed to open Cursor state database read-only: {}",
            path.display()
        )
    })
}

fn open_global_read_only_db() -> Result<Connection> {
    open_read_only_db(&global_state_db_path()?)
}

#[cfg(test)]
pub(crate) fn set_test_cursor_db_path(path: Option<PathBuf>) {
    *TEST_CURSOR_DB_PATH
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Cursor database path lock") = path;
}

/// Open the global state database.
pub fn open_global_db() -> Result<Connection> {
    let path = global_state_db_path()?;
    if !path.exists() {
        anyhow::bail!("Cursor state database not found: {}", path.display());
    }
    Connection::open(&path)
        .with_context(|| format!("Failed to open Cursor state database: {}", path.display()))
}

pub fn key_prefix_bounds(prefix: &str) -> (String, String) {
    let mut upper = prefix.as_bytes().to_vec();
    for idx in (0..upper.len()).rev() {
        if upper[idx] < 0x7f {
            upper[idx] += 1;
            upper.truncate(idx + 1);
            return (prefix.to_string(), String::from_utf8(upper).unwrap());
        }
    }
    (prefix.to_string(), format!("{prefix}\u{10ffff}"))
}

/// Cursor Composer session metadata stored in cursorDiskKV.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct ComposerData {
    #[serde(rename = "composerId")]
    pub composer_id: String,
    pub status: Option<String>,
    pub text: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "workspaceIdentifier")]
    pub workspace_identifier: Option<WorkspaceIdentifier>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<i64>,
    #[serde(rename = "lastUpdatedAt")]
    pub last_updated_at: Option<i64>,
    #[serde(rename = "isAgentic")]
    pub is_agentic: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct WorkspaceIdentifier {
    pub id: String,
    pub uri: WorkspaceUri,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkspaceUri {
    #[serde(rename = "fsPath")]
    pub fs_path: String,
}

#[derive(Debug, Clone)]
pub struct CursorComposerHeader {
    pub created_at: Option<i64>,
    pub last_updated_at: Option<i64>,
    pub recency: Option<i64>,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CursorComposerRecord {
    pub data: ComposerData,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CursorSessionMetadata {
    pub composer_id: String,
    pub header: Option<CursorComposerHeader>,
    pub composer: Option<CursorComposerRecord>,
}

impl CursorSessionMetadata {
    pub fn title(&self) -> Option<String> {
        self.header
            .as_ref()
            .and_then(|header| json_nonempty_string(&header.value, "name"))
            .or_else(|| {
                self.composer
                    .as_ref()
                    .and_then(|composer| nonempty_string(composer.data.name.as_deref()))
            })
            .or_else(|| {
                self.header
                    .as_ref()
                    .and_then(|header| json_nonempty_string(&header.value, "subtitle"))
            })
            .or_else(|| {
                self.composer
                    .as_ref()
                    .and_then(|composer| nonempty_string(composer.data.text.as_deref()))
            })
    }

    pub fn workspace_dir(&self) -> Option<String> {
        self.header
            .as_ref()
            .and_then(|header| {
                header
                    .value
                    .get("workspaceIdentifier")
                    .and_then(|value| value.get("uri"))
                    .and_then(|value| value.get("fsPath"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .or_else(|| {
                self.composer.as_ref().and_then(|composer| {
                    composer
                        .data
                        .workspace_identifier
                        .as_ref()
                        .map(|workspace| workspace.uri.fs_path.clone())
                })
            })
    }

    pub fn created_at_ms(&self) -> Option<i64> {
        self.header
            .as_ref()
            .and_then(|header| header.created_at)
            .or_else(|| {
                self.composer
                    .as_ref()
                    .and_then(|composer| composer.data.created_at)
            })
    }

    pub fn last_active_at_ms(&self) -> Option<i64> {
        self.header
            .as_ref()
            .and_then(|header| header.recency)
            .or_else(|| {
                self.header
                    .as_ref()
                    .and_then(|header| header.last_updated_at)
            })
            .or_else(|| {
                self.composer
                    .as_ref()
                    .and_then(|composer| composer.data.last_updated_at)
            })
    }
}

#[derive(Debug, Clone)]
pub struct CursorBubbleRecord {
    pub key: String,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct CursorInvalidRow {
    pub key: String,
    pub raw: serde_json::Value,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct CursorLoadedSource {
    pub metadata: CursorSessionMetadata,
    pub bubbles: Vec<CursorBubbleRecord>,
    pub invalid_bubbles: Vec<CursorInvalidRow>,
}

const EMPTY_STATE_DRAFT_KEY: &str = "composerData:empty-state-draft";

/// Read current Cursor session metadata by joining composerHeaders and composerData identities.
pub fn list_session_metadata() -> Result<Vec<CursorSessionMetadata>> {
    let conn = open_global_read_only_db()?;
    validate_current_source_schema(&conn)?;
    let mut sessions = BTreeMap::<String, CursorSessionMetadata>::new();

    for (composer_id, header) in read_headers(&conn)? {
        sessions.insert(
            composer_id.clone(),
            CursorSessionMetadata {
                composer_id,
                header: Some(header),
                composer: None,
            },
        );
    }
    for (composer_id, composer) in read_composers(&conn)? {
        let session =
            sessions
                .entry(composer_id.clone())
                .or_insert_with(|| CursorSessionMetadata {
                    composer_id,
                    header: None,
                    composer: None,
                });
        session.composer = Some(composer);
    }

    Ok(sessions.into_values().collect())
}

/// Read one current Cursor session source from a provider-owned locator.
pub fn load_source(source_locator: &str) -> Result<CursorLoadedSource> {
    let (database_path, composer_id) = parse_cursor_source_locator(source_locator)?;
    anyhow::ensure!(
        composer_id != "empty-state-draft",
        "Cursor empty-state draft sentinel is not a session source"
    );
    let conn = open_read_only_db(&database_path)?;
    validate_current_source_schema(&conn)?;
    let mut headers = read_headers(&conn)?;
    let mut composers = read_composers(&conn)?;
    let header = headers.remove(&composer_id);
    let composer = composers.remove(&composer_id);
    anyhow::ensure!(
        header.is_some() || composer.is_some(),
        "Cursor session source does not exist: {composer_id}"
    );

    let prefix = format!("bubbleId:{composer_id}:");
    let (lower, upper) = key_prefix_bounds(&prefix);
    let mut stmt = conn.prepare(
        "SELECT key, value FROM cursorDiskKV
         WHERE key >= ?1 AND key < ?2 ORDER BY key ASC",
    )?;
    let rows = stmt.query_map(params![lower, upper], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, SqliteValue>(1)?))
    })?;
    let mut bubbles = Vec::new();
    let mut invalid_bubbles = Vec::new();
    for row in rows {
        let (key, value) = row?;
        match sqlite_json_value(value.clone()) {
            Ok(raw) if raw.is_object() => bubbles.push(CursorBubbleRecord { key, raw }),
            Ok(raw) => invalid_bubbles.push(CursorInvalidRow {
                key,
                raw,
                error: "Cursor bubble row must contain a JSON object".to_string(),
            }),
            Err(error) => invalid_bubbles.push(CursorInvalidRow {
                key,
                raw: sqlite_value_for_report(value),
                error: error.to_string(),
            }),
        }
    }

    Ok(CursorLoadedSource {
        metadata: CursorSessionMetadata {
            composer_id,
            header,
            composer,
        },
        bubbles,
        invalid_bubbles,
    })
}

fn read_headers(conn: &Connection) -> Result<BTreeMap<String, CursorComposerHeader>> {
    let mut stmt = conn.prepare(
        "SELECT composerId, createdAt, lastUpdatedAt, recency, value
         FROM composerHeaders ORDER BY composerId ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, SqliteValue>(4)?,
        ))
    })?;
    let mut headers = BTreeMap::new();
    for row in rows {
        let (composer_id, created_at, last_updated_at, recency, value) = row?;
        let raw = sqlite_json_value(value)
            .with_context(|| format!("Invalid Cursor composerHeaders value for {composer_id}"))?;
        anyhow::ensure!(
            raw.is_object(),
            "Cursor composerHeaders value must be a JSON object: {composer_id}"
        );
        anyhow::ensure!(
            raw.get("composerId").and_then(serde_json::Value::as_str) == Some(composer_id.as_str()),
            "Cursor composerHeaders identity mismatch: {composer_id}"
        );
        headers.insert(
            composer_id.clone(),
            CursorComposerHeader {
                created_at,
                last_updated_at,
                recency,
                value: raw,
            },
        );
    }
    Ok(headers)
}

fn read_composers(conn: &Connection) -> Result<BTreeMap<String, CursorComposerRecord>> {
    let (lower, upper) = key_prefix_bounds("composerData:");
    let mut stmt = conn.prepare(
        "SELECT key, value FROM cursorDiskKV
         WHERE key >= ?1 AND key < ?2 ORDER BY key ASC",
    )?;
    let rows = stmt.query_map(params![lower, upper], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, SqliteValue>(1)?))
    })?;
    let mut composers = BTreeMap::new();
    for row in rows {
        let (key, value) = row?;
        if key == EMPTY_STATE_DRAFT_KEY && matches!(value, SqliteValue::Null) {
            continue;
        }
        let composer_id = key
            .strip_prefix("composerData:")
            .context("Cursor composerData key is missing its prefix")?
            .to_string();
        let raw = sqlite_json_value(value)
            .with_context(|| format!("Invalid Cursor composerData row: {key}"))?;
        anyhow::ensure!(
            raw.is_object(),
            "Cursor composerData row must contain a JSON object: {key}"
        );
        let data: ComposerData = serde_json::from_value(raw.clone())
            .with_context(|| format!("Invalid Cursor composerData fields: {key}"))?;
        anyhow::ensure!(
            data.composer_id == composer_id,
            "Cursor composerData identity mismatch: key={composer_id}, value={}",
            data.composer_id
        );
        composers.insert(composer_id, CursorComposerRecord { data, raw });
    }
    Ok(composers)
}

fn sqlite_json_value(value: SqliteValue) -> Result<serde_json::Value> {
    let text = match value {
        SqliteValue::Text(text) => text,
        SqliteValue::Blob(bytes) => {
            String::from_utf8(bytes).context("Cursor JSON blob is not UTF-8")?
        }
        SqliteValue::Null => anyhow::bail!("Cursor JSON value is NULL"),
        SqliteValue::Integer(_) | SqliteValue::Real(_) => {
            anyhow::bail!("Cursor JSON value is not text or blob")
        }
    };
    serde_json::from_str(&text).context("Cursor row contains invalid JSON")
}

fn sqlite_value_for_report(value: SqliteValue) -> serde_json::Value {
    match value {
        SqliteValue::Text(text) => serde_json::Value::String(text),
        SqliteValue::Blob(bytes) => match String::from_utf8(bytes) {
            Ok(text) => serde_json::Value::String(text),
            Err(error) => serde_json::json!({
                "sqlite_type": "blob",
                "size_bytes": error.as_bytes().len()
            }),
        },
        SqliteValue::Integer(value) => serde_json::Value::Number(value.into()),
        SqliteValue::Real(value) => serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        SqliteValue::Null => serde_json::Value::Null,
    }
}

fn json_nonempty_string(value: &serde_json::Value, key: &str) -> Option<String> {
    nonempty_string(value.get(key).and_then(serde_json::Value::as_str))
}

fn nonempty_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Calculate a source fingerprint over the current Cursor rows for one composer.
pub fn source_fingerprint(source_locator: &str) -> Result<Option<ProviderSourceFingerprint>> {
    let (database_path, composer_id) = parse_cursor_source_locator(source_locator)?;
    let Some(database_metadata) = database_path.metadata().ok() else {
        return Ok(None);
    };
    let conn = open_read_only_db(&database_path)?;
    validate_current_source_schema(&conn)?;

    let composer_key = format!("composerData:{composer_id}");
    let bubble_prefix = format!("bubbleId:{composer_id}:");
    let (bubble_lower, bubble_upper) = key_prefix_bounds(&bubble_prefix);
    let has_composer_data = conn
        .query_row(
            "SELECT 1 FROM cursorDiskKV WHERE key = ?1",
            [&composer_key],
            |_| Ok(()),
        )
        .optional()?;
    let has_header = conn
        .query_row(
            "SELECT 1 FROM composerHeaders WHERE composerId = ?1",
            [&composer_id],
            |_| Ok(()),
        )
        .optional()?;
    if has_composer_data.is_none() && has_header.is_none() {
        return Ok(None);
    }

    let mut hasher = Sha256::new();
    hasher.update(b"cursor-source-v1\0");
    hash_current_header(&conn, &mut hasher, &composer_id)?;
    hash_keyed_value(&conn, &mut hasher, "cursorDiskKV", &composer_key)?;
    let mut bubble_stmt = conn.prepare(
        "SELECT key, value FROM cursorDiskKV
         WHERE key >= ?1 AND key < ?2 ORDER BY key ASC",
    )?;
    let bubble_rows = bubble_stmt.query_map(params![bubble_lower, bubble_upper], |row| {
        let value = row.get::<_, SqliteValue>(1)?;
        Ok((row.get::<_, String>(0)?, sqlite_value_bytes(value)))
    })?;
    for row in bubble_rows {
        let (key, value) = row?;
        hash_bytes(&mut hasher, b"cursorDiskKV", key.as_bytes(), &value);
    }
    hash_keyed_value(
        &conn,
        &mut hasher,
        "ItemTable",
        "composer.composerHeaders.migratedToTable",
    )?;

    let wal_path = PathBuf::from(format!("{}-wal", database_path.display()));
    let wal_metadata = wal_path.metadata().ok();
    let modified_at_ms = source_modified_at_ms(&database_metadata).max(
        wal_metadata
            .as_ref()
            .map(source_modified_at_ms)
            .unwrap_or(0),
    );
    let size_bytes = i64::try_from(database_metadata.len())
        .unwrap_or(i64::MAX)
        .saturating_add(
            wal_metadata
                .as_ref()
                .map(|metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
                .unwrap_or(0),
        );
    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes,
        value: format!("sqlite-rows-v1:{:x}", hasher.finalize()),
    }))
}

fn validate_current_source_schema(conn: &Connection) -> Result<()> {
    for (table, expected_columns) in [
        ("ItemTable", &["key", "value"][..]),
        ("cursorDiskKV", &["key", "value"][..]),
        (
            "composerHeaders",
            &[
                "composerId",
                "workspaceId",
                "createdAt",
                "lastUpdatedAt",
                "isArchived",
                "isSubagent",
                "recency",
                "checkpointAt",
                "value",
            ][..],
        ),
    ] {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        anyhow::ensure!(
            columns == expected_columns,
            "Cursor current source schema for {table} is not supported"
        );
    }
    Ok(())
}

fn hash_current_header(conn: &Connection, hasher: &mut Sha256, composer_id: &str) -> Result<()> {
    let row = conn
        .query_row(
            "SELECT composerId, workspaceId, createdAt, lastUpdatedAt, isArchived,
                    isSubagent, recency, checkpointAt, value
             FROM composerHeaders WHERE composerId = ?1",
            [composer_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        hasher.update(b"composerHeaders:missing\0");
        return Ok(());
    };
    let encoded = serde_json::to_vec(&row)?;
    hash_bytes(hasher, b"composerHeaders", composer_id.as_bytes(), &encoded);
    Ok(())
}

fn hash_keyed_value(conn: &Connection, hasher: &mut Sha256, table: &str, key: &str) -> Result<()> {
    let sql = format!("SELECT value FROM {table} WHERE key = ?1");
    let value = conn
        .query_row(&sql, [key], |row| {
            Ok(sqlite_value_bytes(row.get::<_, SqliteValue>(0)?))
        })
        .optional()?;
    if let Some(value) = value {
        hash_bytes(hasher, table.as_bytes(), key.as_bytes(), &value);
    } else {
        hasher.update(table.as_bytes());
        hasher.update(b":missing:");
        hasher.update(key.as_bytes());
        hasher.update([0]);
    }
    Ok(())
}

fn sqlite_value_bytes(value: SqliteValue) -> Vec<u8> {
    match value {
        SqliteValue::Blob(bytes) => bytes,
        SqliteValue::Text(text) => text.into_bytes(),
        SqliteValue::Integer(value) => value.to_le_bytes().to_vec(),
        SqliteValue::Real(value) => value.to_le_bytes().to_vec(),
        SqliteValue::Null => Vec::new(),
    }
}

fn hash_bytes(hasher: &mut Sha256, table: &[u8], key: &[u8], value: &[u8]) {
    for part in [table, key, value] {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
}

fn source_modified_at_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Calculate the total storage size (in bytes) of a composer and all its bubbles.
pub fn composer_size(composer_id: &str) -> Result<u64> {
    let conn = open_global_db()?;
    let mut total: u64 = 0;

    // Composer metadata size
    if let Ok(size) = conn.query_row(
        "SELECT length(value) FROM cursorDiskKV WHERE key = ?",
        [format!("composerData:{}", composer_id)],
        |row| row.get::<_, i64>(0),
    ) {
        total += size as u64;
    }

    // All bubbles size
    let prefix = format!("bubbleId:{}:", composer_id);
    let (lower, upper) = key_prefix_bounds(&prefix);
    let mut stmt =
        conn.prepare("SELECT length(value) FROM cursorDiskKV WHERE key >= ?1 AND key < ?2")?;
    let rows = stmt.query_map(params![lower, upper], |row| row.get::<_, i64>(0))?;
    for row in rows {
        if let Ok(size) = row {
            total += size as u64;
        }
    }

    Ok(total)
}

pub fn composer_sizes(composer_ids: &[&str]) -> Result<HashMap<String, u64>> {
    let conn = open_global_db()?;
    let mut sizes: HashMap<String, u64> = composer_ids
        .iter()
        .map(|composer_id| ((*composer_id).to_string(), 0))
        .collect();

    for composer_id in composer_ids {
        let composer_key = format!("composerData:{}", composer_id);
        if let Ok(size) = conn.query_row(
            "SELECT length(value) FROM cursorDiskKV WHERE key = ?1",
            [&composer_key],
            |row| row.get::<_, i64>(0),
        ) {
            if let Some(total) = sizes.get_mut(*composer_id) {
                *total += size as u64;
            }
        }

        let prefix = format!("bubbleId:{}:", composer_id);
        let (lower, upper) = key_prefix_bounds(&prefix);
        let mut stmt =
            conn.prepare("SELECT length(value) FROM cursorDiskKV WHERE key >= ?1 AND key < ?2")?;
        let rows = stmt.query_map(params![lower, upper], |row| row.get::<_, i64>(0))?;
        for row in rows {
            if let (Some(total), Ok(size)) = (sizes.get_mut(*composer_id), row) {
                *total += size as u64;
            }
        }
    }

    sizes.retain(|_, size| *size > 0);
    Ok(sizes)
}
