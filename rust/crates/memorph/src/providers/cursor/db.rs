use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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

/// Cursor bubble (message) stored in cursorDiskKV.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(dead_code)]
pub struct BubbleData {
    #[serde(rename = "bubbleId")]
    pub bubble_id: String,
    #[serde(rename = "type")]
    pub bubble_type: i32,
    pub text: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(rename = "requestId")]
    pub request_id: Option<String>,
    #[serde(rename = "modelInfo")]
    pub model_info: Option<serde_json::Value>,
}

/// Read all composer session metadata from the global database.
pub fn list_composers() -> Result<Vec<ComposerData>> {
    let conn = open_global_db()?;
    let (lower, upper) = key_prefix_bounds("composerData:");
    let mut stmt =
        conn.prepare("SELECT CAST(value AS TEXT) FROM cursorDiskKV WHERE key >= ?1 AND key < ?2")?;

    let rows = stmt.query_map(params![lower, upper], |row| {
        let json: String = row.get(0)?;
        let parsed: ComposerData = serde_json::from_str(&json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(parsed)
    })?;

    let mut composers = Vec::new();
    for row in rows {
        if let Ok(c) = row {
            composers.push(c);
        }
    }
    Ok(composers)
}

/// Read all bubbles for a given composer session.
pub fn list_bubbles(composer_id: &str) -> Result<Vec<BubbleData>> {
    let conn = open_global_db()?;
    let prefix = format!("bubbleId:{}:", composer_id);
    let (lower, upper) = key_prefix_bounds(&prefix);
    let mut stmt =
        conn.prepare("SELECT CAST(value AS TEXT) FROM cursorDiskKV WHERE key >= ?1 AND key < ?2")?;

    let rows = stmt.query_map(params![lower, upper], |row| {
        let json: String = row.get(0)?;
        let parsed: BubbleData = serde_json::from_str(&json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(parsed)
    })?;

    let mut bubbles = Vec::new();
    for row in rows {
        if let Ok(b) = row {
            bubbles.push(b);
        }
    }
    Ok(bubbles)
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
