//! Kiro CLI SQLite source: `data_local_dir/kiro-cli/data.sqlite3`
//! Table `conversations_v2` with ConversationState JSON in `value` column.

use super::*;
use crate::providers::q_conversation;
use rusqlite::{Connection, OpenFlags};

pub(super) const CLI_SOURCE_PREFIX: &str = "kiro-cli://";

#[cfg(test)]
static TEST_KIRO_CLI_DB_PATH: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

/// Resolve the Kiro CLI SQLite database path.
pub(super) fn kiro_cli_db_path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_KIRO_CLI_DB_PATH
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
    {
        return Some(path);
    }
    Some(
        dirs::data_local_dir()?
            .join("kiro-cli")
            .join("data.sqlite3"),
    )
}

#[cfg(test)]
pub(super) fn set_test_kiro_cli_db_path(path: Option<PathBuf>) {
    *TEST_KIRO_CLI_DB_PATH
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("Kiro CLI test DB lock") = path;
}

/// Scan all conversations from the Kiro CLI database.
pub(super) fn scan_cli_sessions() -> Result<Vec<ProviderSessionSummary>> {
    let Some(db_path) = kiro_cli_db_path() else {
        return Ok(Vec::new());
    };
    if !db_path.is_file() {
        return Ok(Vec::new());
    }
    let conn = open_readonly(&db_path)?;
    let mut stmt = conn.prepare(
        "SELECT conversation_id, key, value, created_at, updated_at
         FROM conversations_v2 ORDER BY updated_at DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(CliRow {
            conversation_id: row.get(0)?,
            key: row.get(1)?,
            value: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    })?;

    let mut sessions = Vec::new();
    for row in rows.flatten() {
        let title = q_conversation::first_prompt_text(&row.value, 100);
        let project_dir = if row.key.is_empty() {
            None
        } else {
            Some(row.key.clone())
        };
        let created_at = valid_timestamp_ms(row.created_at);
        let last_active_at = valid_timestamp_ms(row.updated_at).or(created_at);

        sessions.push(ProviderSessionSummary {
            archived: false,
            session_id: row.conversation_id.clone(),
            title,
            project_dir,
            created_at,
            last_active_at,
            source_path: Some(format!("{CLI_SOURCE_PREFIX}{}", row.conversation_id)),
        });
    }
    Ok(sessions)
}

/// Import a single CLI conversation by conversation_id.
pub(super) fn import_cli_session(conversation_id: &str) -> Result<ImportedSession> {
    let db_path = kiro_cli_db_path().context("Cannot resolve Kiro CLI data directory")?;
    anyhow::ensure!(
        db_path.is_file(),
        "Kiro CLI database not found: {}",
        db_path.display()
    );

    let conn = open_readonly(&db_path)?;
    let (key, value, created_at, updated_at): (String, String, i64, i64) = conn.query_row(
        "SELECT key, value, created_at, updated_at FROM conversations_v2 WHERE conversation_id = ?1",
        [conversation_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).with_context(|| format!("Kiro CLI conversation not found: {conversation_id}"))?;

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let events = q_conversation::parse_history(PROVIDER_ID, &value, conversation_id, &mut report);

    let title = q_conversation::first_prompt_text(&value, 100);
    let created = valid_timestamp_ms(created_at).and_then(DateTime::from_timestamp_millis);
    let last_active = valid_timestamp_ms(updated_at)
        .and_then(DateTime::from_timestamp_millis)
        .or(created);

    let event_meta = events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
        .collect::<Vec<_>>();

    Ok(ImportedSession {
        session: Session {
            lineage: Vec::new(),
            schema: Schema::default(),
            identity: Identity {
                id: conversation_id.to_string(),
                title,
            },
            context: Context {
                workspace: if key.is_empty() { None } else { Some(key) },
                created_at: created,
                last_active_at: last_active,
                tags: Vec::new(),
            },
            events,
            extensions: BTreeMap::new(),
        },
        provenance: Provenance {
            imported_at: Utc::now(),
            imported_by: Some("memorph-cli".to_string()),
            primary_source: ProviderRef {
                provider_id: PROVIDER_ID.to_string(),
                session_id: conversation_id.to_string(),
                source_path: Some(format!("{CLI_SOURCE_PREFIX}{conversation_id}")),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    })
}

/// Fingerprint for a CLI session: based on DB file mtime + conversation updated_at.
pub(super) fn cli_session_fingerprint(
    conversation_id: &str,
) -> Result<Option<ProviderSourceFingerprint>> {
    let Some(db_path) = kiro_cli_db_path() else {
        return Ok(None);
    };
    if !db_path.is_file() {
        return Ok(None);
    }
    let conn = open_readonly(&db_path)?;
    let updated_at: i64 = match conn.query_row(
        "SELECT updated_at FROM conversations_v2 WHERE conversation_id = ?1",
        [conversation_id],
        |row| row.get(0),
    ) {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let mut hasher = Sha256::new();
    hasher.update(b"kiro-cli-v1:");
    hasher.update(conversation_id.as_bytes());
    hasher.update(b":");
    hasher.update(updated_at.to_le_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let size_bytes = std::fs::metadata(&db_path)
        .map(|metadata| metadata.len().min(i64::MAX as u64) as i64)
        .unwrap_or(0);

    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms: updated_at.max(0),
        size_bytes,
        value: format!("kiro-cli-v1:{conversation_id}:{hash}"),
    }))
}

struct CliRow {
    conversation_id: String,
    key: String,
    value: String,
    created_at: i64,
    updated_at: i64,
}

fn open_readonly(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("Failed to open Kiro CLI DB: {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

fn valid_timestamp_ms(ms: i64) -> Option<i64> {
    (ms > 0).then_some(ms)
}
