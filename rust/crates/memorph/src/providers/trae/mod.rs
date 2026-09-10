pub mod adapter;
pub mod hook;

use crate::provider::{
    PageStrategy, Provider, ProviderCapabilities, ProviderContentFidelity, ProviderSessionSummary,
    ProviderSourceFingerprint, ResumeQuality, ScanStrategy, StorageShape, TurnQuality,
};
use crate::session::{
    Block, Context, Event, EventKind, Fidelity, Identity, ImportedSession, Links, MappingDirection,
    MappingReport, Metadata, Provenance, ProviderRef, Role, Schema, Session,
};
use anyhow::{bail, Context as _, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct TraeProvider;

const PROVIDER_ID: &str = "trae";
const STORAGE_KEY: &str = "memento/icube-ai-agent-storage";

#[cfg(test)]
static TEST_TRAE_CONFIG_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

impl Provider for TraeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "Trae"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            storage_shape: StorageShape::Sqlite,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::FullImport,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Unsupported),
                tool_result: Some(Fidelity::Unsupported),
                provider_payload: Some(Fidelity::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            resume_quality: ResumeQuality::None,
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let mut sessions = Vec::new();
        for root in state_paths() {
            for db in db_paths(&root) {
                if !db.exists() {
                    continue;
                }
                let conn = Connection::open(&db)
                    .with_context(|| format!("failed to open Trae state db at {}", db.display()))?;
                let Some(data) = read_storage(&conn)? else {
                    continue;
                };
                for session in session_list(&data) {
                    let Some(id) = session_id(session) else {
                        continue;
                    };
                    sessions.push(ProviderSessionSummary {
                        archived: false,
                        session_id: id,
                        title: session_title(session),
                        project_dir: workspace_dir(&db),
                        created_at: None,
                        last_active_at: session_updated_at(session),
                        source_path: Some(locator(
                            &db,
                            session_id(session).as_deref().unwrap_or_default(),
                        )),
                    });
                }
            }
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
        sessions.dedup_by(|a, b| a.session_id == b.session_id);
        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let (db, id) = parse_locator(source_path)?;
        let conn = Connection::open(&db)?;
        let data = read_storage(&conn)?.context("Trae storage key is missing")?;
        let session = session_list(&data)
            .into_iter()
            .find(|session| session_id(session).as_deref() == Some(id.as_str()))
            .context("Trae session was not found")?;
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let events: Vec<Event> = session_messages(session)
            .iter()
            .enumerate()
            .filter_map(|(index, message)| map_message(message, index, &mut report))
            .collect();
        let event_meta = events
            .iter()
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect::<Vec<_>>();
        let title = session_title(session);
        let now = session_updated_at(session)
            .and_then(datetime_from_ms)
            .unwrap_or_else(Utc::now);
        let mut extensions = BTreeMap::new();
        extensions.insert("trae_storage_key".into(), Value::String(STORAGE_KEY.into()));
        Ok(ImportedSession {
            session: Session {
                lineage: Vec::new(),
                schema: Schema::default(),
                identity: Identity {
                    id: id.clone(),
                    title,
                },
                context: Context {
                    workspace: workspace_dir(&db),
                    created_at: None,
                    last_active_at: Some(now),
                    tags: Vec::new(),
                },
                events,
                extensions,
            },
            provenance: Provenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".into()),
                primary_source: ProviderRef {
                    provider_id: PROVIDER_ID.into(),
                    session_id: id,
                    source_path: Some(source_path.into()),
                },
                aliases: Vec::new(),
            },
            event_meta,
            report,
        })
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        let (db, id) = parse_locator(source_path)?;
        if !db.exists() {
            return Ok(None);
        }
        let conn = Connection::open(&db)?;
        let Some(data) = read_storage(&conn)? else {
            return Ok(None);
        };
        let Some(session) = session_list(&data)
            .into_iter()
            .find(|s| session_id(s).as_deref() == Some(id.as_str()))
        else {
            return Ok(None);
        };
        let bytes = serde_json::to_vec(session)?;
        let digest = Sha256::digest(&bytes);
        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms: session_updated_at(session).unwrap_or_else(|| file_modified_ms(&db)),
            size_bytes: bytes.len().min(i64::MAX as usize) as i64,
            value: format!("trae-workspace-v1:{digest:x}"),
        }))
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        state_paths()
    }
}

fn trae_config_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(dir) = TEST_TRAE_CONFIG_DIR
        .get()
        .and_then(|v| v.lock().ok())
        .and_then(|v| v.clone())
    {
        return Some(dir);
    }
    dirs::config_dir()
}

fn state_paths() -> Vec<PathBuf> {
    let Some(cfg) = trae_config_dir() else {
        return Vec::new();
    };
    vec![cfg.join("Trae").join("User").join("workspaceStorage")]
}

fn workspace_dir(db: &Path) -> Option<String> {
    std::fs::read_to_string(db.parent()?.join("workspace.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.get("folder").and_then(Value::as_str).map(str::to_owned))
}

fn db_paths(root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("state.vscdb"))
}

fn read_storage(conn: &Connection) -> Result<Option<Value>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [STORAGE_KEY],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| serde_json::from_str(&value).context("Trae storage JSON is invalid"))
        .transpose()
}

fn session_list(data: &Value) -> Vec<&Value> {
    data.get("list")
        .and_then(Value::as_array)
        .map(|v| v.iter().collect())
        .unwrap_or_default()
}
fn session_id(session: &Value) -> Option<String> {
    session
        .get("sessionId")
        .or_else(|| session.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
fn session_title(session: &Value) -> Option<String> {
    session
        .get("title")
        .or_else(|| session.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
fn session_messages(session: &Value) -> &[Value] {
    session
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn session_updated_at(session: &Value) -> Option<i64> {
    session
        .get("updatedAt")
        .or_else(|| session.get("createdAt"))
        .and_then(Value::as_i64)
}
fn datetime_from_ms(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
}
fn file_modified_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn locator(db: &Path, session_id: &str) -> String {
    format!("{}#session={session_id}", db.display())
}
fn parse_locator(source: &str) -> Result<(PathBuf, String)> {
    let (path, fragment) = source
        .split_once("#session=")
        .context("invalid Trae locator")?;
    if path.is_empty() || fragment.is_empty() {
        bail!("invalid Trae locator")
    }
    Ok((PathBuf::from(path), fragment.to_owned()))
}

fn map_message(message: &Value, index: usize, report: &mut MappingReport) -> Option<Event> {
    let role = match message.get("role").and_then(Value::as_str) {
        Some("user") => Role::User,
        Some("assistant") | Some("ai") => Role::Assistant,
        Some("tool") => Role::Tool,
        _ => {
            report.push_issue(crate::session::MappingIssue {
                level: crate::session::MappingIssueLevel::Warning,
                disposition: Fidelity::Unsupported,
                code: "trae-unknown-role".into(),
                message: "Dropped Trae message with unknown role".into(),
                path: Some(format!("messages[{index}]")),
                raw: Some(message.clone()),
            });
            return None;
        }
    };
    let text = message
        .get("content")
        .or_else(|| message.get("text"))
        .or_else(|| message.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut blocks = text
        .map(|text| vec![Block::Text { text }])
        .unwrap_or_default();
    if let Some(task) = message.get("agentTaskContent") {
        blocks.push(Block::Other { raw: task.clone() });
    }
    if blocks.is_empty() {
        return None;
    }
    report.push_issue(crate::session::MappingIssue {
        level: crate::session::MappingIssueLevel::Info,
        disposition: Fidelity::Preserved,
        code: "trae-native-message".into(),
        message: "Mapped Trae workspaceStorage message".into(),
        path: Some(format!("messages[{index}]")),
        raw: None,
    });
    Some(Event {
        id: format!("trae:event:{index}"),
        kind: EventKind::Message,
        role,
        timestamp: message
            .get("timestamp")
            .and_then(Value::as_i64)
            .and_then(datetime_from_ms)
            .unwrap_or_else(Utc::now),
        links: Links::default(),
        blocks,
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::tempdir;

    struct ConfigDirGuard;

    impl Drop for ConfigDirGuard {
        fn drop(&mut self) {
            *TEST_TRAE_CONFIG_DIR.get().unwrap().lock().unwrap() = None;
        }
    }

    fn config_dir_guard(dir: &Path) -> ConfigDirGuard {
        *TEST_TRAE_CONFIG_DIR
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap() = Some(dir.to_path_buf());
        ConfigDirGuard
    }

    #[test]
    fn scans_and_imports_macos_workspace_storage_session() {
        let root = tempdir().unwrap();
        let workspace = root.path().join("Trae/User/workspaceStorage/abc");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("workspace.json"),
            r#"{"folder":"file:///tmp/demo"}"#,
        )
        .unwrap();
        let db = workspace.join("state.vscdb");
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
        let data = serde_json::json!({"list":[{"sessionId":"trae-1","title":"Fix bug","createdAt":1000,"updatedAt":2000,"messages":[{"id":"m1","role":"user","content":"hello","timestamp":1000},{"id":"m2","role":"assistant","content":"world","timestamp":2000}]}]});
        conn.execute(
            "INSERT INTO ItemTable(key,value) VALUES(?1,?2)",
            params![STORAGE_KEY, data.to_string()],
        )
        .unwrap();
        let _guard = config_dir_guard(root.path());
        let provider = TraeProvider;
        let sessions = provider.scan_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "trae-1");
        let imported = provider
            .import_session(sessions[0].source_path.as_deref().unwrap())
            .unwrap();
        assert_eq!(imported.session.events.len(), 2);
        assert_eq!(imported.session.identity.id, "trae-1");
        assert!(provider
            .session_source_fingerprint(sessions[0].source_path.as_deref().unwrap())
            .unwrap()
            .is_some());
    }
}
