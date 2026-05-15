use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Deserialize;
use std::path::PathBuf;

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
    Ok(cursor_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("state.vscdb"))
}

/// Open the global state database read-only.
pub fn open_global_db() -> Result<Connection> {
    let path = global_state_db_path()?;
    if !path.exists() {
        anyhow::bail!("Cursor state database not found: {}", path.display());
    }
    Connection::open(&path)
        .with_context(|| format!("Failed to open Cursor state database: {}", path.display()))
}

/// Cursor Composer session metadata stored in cursorDiskKV.
#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct WorkspaceIdentifier {
    pub id: String,
    pub uri: WorkspaceUri,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceUri {
    #[serde(rename = "fsPath")]
    pub fs_path: String,
}

/// Cursor bubble (message) stored in cursorDiskKV.
#[derive(Debug, Clone, Deserialize)]
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
    let mut stmt = conn
        .prepare("SELECT CAST(value AS TEXT) FROM cursorDiskKV WHERE key LIKE 'composerData:%'")?;

    let rows = stmt.query_map([], |row| {
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
    let pattern = format!("bubbleId:{}:%", composer_id);
    let mut stmt =
        conn.prepare("SELECT CAST(value AS TEXT) FROM cursorDiskKV WHERE key LIKE ?1")?;

    let rows = stmt.query_map([&pattern], |row| {
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
    let pattern = format!("bubbleId:{}:%", composer_id);
    let mut stmt = conn.prepare("SELECT length(value) FROM cursorDiskKV WHERE key LIKE ?")?;
    let rows = stmt.query_map([&pattern], |row| row.get::<_, i64>(0))?;
    for row in rows {
        if let Ok(size) = row {
            total += size as u64;
        }
    }

    Ok(total)
}
