pub mod adapter;
mod backup;
pub mod hook;

use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::provider::{
    canonical_event_visible_message_role, canonical_export_result, canonical_session_title,
    canonical_visible_block_text, Provider, ProviderBackupSupport, ProviderCapabilities,
    ProviderSessionBackup, ProviderSessionSummary, ProviderSourceMutation,
};
use crate::utils::truncate_summary;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct KiroProvider;

const PROVIDER_ID: &str = "kiro";
const TITLE_MAX_CHARS: usize = 80;

#[cfg(test)]
static TEST_KIRO_GLOBAL_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_KIRO_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

impl Provider for KiroProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Kiro"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: false,
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
            },
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let global_dir = kiro_global_storage_dir()?;
        if !global_dir.exists() {
            return Ok(Vec::new());
        }
        scan_sessions_in(&global_dir)
    }

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        let global_dir = kiro_global_storage_dir()?;
        if !global_dir.exists() {
            return Ok(None);
        }

        for list_path in session_list_paths(&global_dir)? {
            let Some(session_dir) = list_path.parent() else {
                continue;
            };
            for entry in read_session_list(&list_path)? {
                if entry.get("hidden").and_then(|v| v.as_bool()) == Some(true) {
                    continue;
                }
                let Some(id) = entry.get("sessionId").and_then(|v| v.as_str()) else {
                    continue;
                };
                if id != session_id {
                    continue;
                }

                let session_path = session_dir.join(format!("{}.json", session_id));
                let title = entry
                    .get("title")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string);
                let project_dir = entry
                    .get("workspaceDirectory")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string);
                let last_active_at = path_mtime_ms(&session_path)
                    .or_else(|| entry.get("dateCreated").and_then(parse_ms));

                return Ok(Some(ProviderSessionSummary {
                    session_id: session_id.to_string(),
                    title,
                    project_dir,
                    last_active_at,
                    source_path: Some(session_path.to_string_lossy().to_string()),
                }));
            }
        }

        Ok(None)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        import_canonical_session_from_path(Path::new(source_path))
    }

    fn export_session(
        &self,
        session: &CanonicalSession,
        target_dir: &Path,
    ) -> Result<ExportedSession> {
        let global_dir = kiro_global_storage_dir()?;
        let session_id = export_canonical_session_in(&global_dir, session, target_dir)?;
        Ok(canonical_export_result(
            PROVIDER_ID,
            session_id.clone(),
            self.resume_command(&session_id),
            session,
            self.capabilities(),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let global_dir = kiro_global_storage_dir()?;
        backup::delete_session(&global_dir, session_id)
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let global_dir = kiro_global_storage_dir()?;
        backup::rename_session(&global_dir, session_id, new_title)
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        let global_dir = kiro_global_storage_dir()?;
        backup::create_session_backup(&global_dir, mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        backup::restore_session_backup(backup)
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let global_dir = kiro_global_storage_dir()?;
        for list_path in session_list_paths(&global_dir)? {
            let Some(session_dir) = list_path.parent() else {
                continue;
            };
            let session_path = session_dir.join(format!("{}.json", session_id));
            if session_path.exists() {
                return Ok(std::fs::metadata(session_path)?.len());
            }
        }
        Ok(0)
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        kiro_global_storage_dir().ok().into_iter().collect()
    }
}

fn kiro_data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().context("Unable to locate user home directory")?;
        return Ok(home.join("Library/Application Support/Kiro"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| anyhow::anyhow!("APPDATA environment variable not found"))?;
        return Ok(PathBuf::from(appdata).join("Kiro"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().context("Unable to locate user home directory")?;
        return Ok(home.join(".config/Kiro"));
    }

    #[allow(unreachable_code)]
    Err(anyhow::anyhow!(
        "Kiro data directory not supported on this platform"
    ))
}

fn kiro_global_storage_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_KIRO_GLOBAL_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kiro global dir lock")
        .clone()
    {
        return Ok(path);
    }

    Ok(kiro_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("kiro.kiroagent"))
}

fn scan_sessions_in(global_dir: &Path) -> Result<Vec<ProviderSessionSummary>> {
    let mut sessions = Vec::new();

    for list_path in session_list_paths(global_dir)? {
        let Some(session_dir) = list_path.parent() else {
            continue;
        };
        for entry in read_session_list(&list_path)? {
            if entry.get("hidden").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }

            let Some(session_id) = entry.get("sessionId").and_then(|v| v.as_str()) else {
                continue;
            };

            let session_path = session_dir.join(format!("{}.json", session_id));
            let title = entry
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string);
            let project_dir = entry
                .get("workspaceDirectory")
                .and_then(|v| v.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string);
            let last_active_at = path_mtime_ms(&session_path)
                .or_else(|| entry.get("dateCreated").and_then(parse_ms));

            sessions.push(ProviderSessionSummary {
                session_id: session_id.to_string(),
                title,
                project_dir,
                last_active_at,
                source_path: Some(session_path.to_string_lossy().to_string()),
            });
        }
    }

    Ok(sessions)
}

fn import_canonical_session_from_path(path: &Path) -> Result<ImportedSession> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read Kiro session: {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse Kiro session: {}", path.display()))?;
    import_canonical_session_from_value(path, value)
}

fn import_canonical_session_from_value(path: &Path, value: Value) -> Result<ImportedSession> {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let fallback_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    let session_id = value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or(&fallback_id)
        .to_string();
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let workspace_dir = value
        .get("workspaceDirectory")
        .and_then(|v| v.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let session_time = path_mtime_ms(path).unwrap_or_else(|| Utc::now().timestamp_millis());
    let session_dt = chrono::DateTime::from_timestamp_millis(session_time).unwrap_or_else(Utc::now);

    let mut events = Vec::new();
    if let Some(history) = value.get("history").and_then(|v| v.as_array()) {
        for (index, item) in history.iter().enumerate() {
            match canonical_event_from_kiro_history_item(index, item, session_dt, &mut report) {
                Some(event) => events.push(event),
                None => report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Dropped,
                    code: "empty_history_item_dropped".to_string(),
                    message: "Dropped Kiro history item without message content".to_string(),
                    path: Some(format!("history:{}", index)),
                    raw: Some(item.clone()),
                }),
            }
        }
    }

    let mut extensions = BTreeMap::new();
    extensions.insert("kiro_session".to_string(), value);

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: session_id.clone(),
                source_title: title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id,
                    source_path: Some(path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir,
                created_at: Some(session_dt),
                last_active_at: Some(session_dt),
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn canonical_event_from_kiro_history_item(
    index: usize,
    item: &Value,
    timestamp: chrono::DateTime<Utc>,
    report: &mut MappingReport,
) -> Option<SessionEvent> {
    let message = item.get("message")?;
    let role_str = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("user");
    let role = match role_str {
        "user" => EventRole::User,
        "assistant" | "bot" => EventRole::Assistant,
        "tool" => EventRole::Tool,
        "system" => EventRole::System,
        "developer" => EventRole::Developer,
        other => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Normalized,
                code: "unknown_role_normalized".to_string(),
                message: format!("Normalized unknown Kiro role '{}'", other),
                path: Some(format!("history:{}", index)),
                raw: Some(item.clone()),
            });
            EventRole::Unknown
        }
    };
    let blocks = kiro_event_blocks(message.get("content"), item, index, report);
    if blocks.is_empty() {
        return None;
    }
    let id = message
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("kiro:history:{}", index));

    Some(SessionEvent {
        id,
        kind: kiro_event_kind(&blocks),
        role,
        timestamp,
        links: EventLinks {
            parent_event_id: None,
            provider_parent_id: None,
            provider_turn_id: None,
            turn_index: Some(index as u32),
            turn_boundary: None,
            related_event_ids: Vec::new(),
        },
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: message
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                original_role: Some(role_str.to_string()),
                phase: None,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("kiro_history_item".to_string(), item.clone());
                ext
            },
        },
    })
}

fn kiro_event_blocks(
    content: Option<&Value>,
    raw_item: &Value,
    index: usize,
    report: &mut MappingReport,
) -> Vec<EventBlock> {
    match content {
        Some(Value::String(text)) => {
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![EventBlock::Text { text: text.clone() }]
            }
        }
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(block_index, item)| {
                kiro_content_event_block(item, raw_item, index, block_index, report)
            })
            .collect(),
        Some(Value::Object(object)) => {
            if let Some(text) = object.get("text").and_then(|v| v.as_str()) {
                if text.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![EventBlock::Text {
                        text: text.to_string(),
                    }]
                }
            } else {
                vec![EventBlock::ProviderPayload {
                    kind: "content".to_string(),
                    payload: content.cloned().unwrap_or(Value::Null),
                }]
            }
        }
        Some(other) => vec![EventBlock::Unknown { raw: other.clone() }],
        None => Vec::new(),
    }
}

fn kiro_content_event_block(
    value: &Value,
    raw_item: &Value,
    index: usize,
    block_index: usize,
    report: &mut MappingReport,
) -> EventBlock {
    if let Some(text) = value.get("text").and_then(|v| v.as_str()) {
        return EventBlock::Text {
            text: text.to_string(),
        };
    }
    if let Some(thinking) = value.get("thinking").and_then(|v| v.as_str()) {
        return EventBlock::Thinking {
            text: thinking.to_string(),
            signature: None,
        };
    }

    let kind = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("content");
    report.push_issue(MappingIssue {
        level: MappingIssueLevel::Info,
        disposition: MappingDisposition::Preserved,
        code: "provider_block_preserved".to_string(),
        message: format!("Preserved unsupported Kiro content block '{}'", kind),
        path: Some(format!("history:{}:block:{}", index, block_index)),
        raw: Some(raw_item.clone()),
    });
    EventBlock::ProviderPayload {
        kind: kind.to_string(),
        payload: value.clone(),
    }
}

fn kiro_event_kind(blocks: &[EventBlock]) -> SessionEventKind {
    if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolResult { .. }))
    {
        SessionEventKind::ToolResult
    } else if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolCall { .. }))
    {
        SessionEventKind::ToolCall
    } else if blocks.iter().all(|block| {
        matches!(
            block,
            EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. }
        )
    }) {
        SessionEventKind::Unknown
    } else {
        SessionEventKind::Message
    }
}

fn export_canonical_session_in(
    global_dir: &Path,
    session: &CanonicalSession,
    target_dir: &Path,
) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = truncate_summary(&canonical_session_title(session), TITLE_MAX_CHARS);
    let now = Utc::now().timestamp_millis();
    let created = session
        .context
        .created_at
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(now);

    let history: Vec<Value> = session
        .events
        .iter()
        .filter_map(canonical_event_to_kiro_history_item)
        .collect();

    let session_json = json!({
        "history": history,
        "sessionId": session_id,
        "title": title,
        "workspaceDirectory": target_dir_str,
        "sessionType": "vibe",
        "contextUsagePercentage": 0
    });

    let sessions_dir = sessions_folder(global_dir, Some(&target_dir_str));
    std::fs::create_dir_all(&sessions_dir)?;
    let session_path = sessions_dir.join(format!("{}.json", session_id));
    std::fs::write(&session_path, serde_json::to_string_pretty(&session_json)?)?;

    let list_path = sessions_dir.join("sessions.json");
    upsert_session_list_entry(
        &list_path,
        json!({
            "sessionId": session_id,
            "title": title,
            "dateCreated": created.to_string(),
            "workspaceDirectory": target_dir_str,
            "hidden": false
        }),
    )?;

    Ok(session_id)
}

fn canonical_event_to_kiro_history_item(event: &SessionEvent) -> Option<Value> {
    let visible_role = canonical_event_visible_message_role(event)?;
    let content = canonical_event_kiro_content(event);
    if content.is_empty() {
        return None;
    }
    let role = match visible_role {
        EventRole::Assistant => "assistant",
        _ => "user",
    };

    Some(json!({
        "message": {
            "id": event.id,
            "role": role,
            "content": content
        },
        "contextItems": [],
        "editorState": {}
    }))
}

fn canonical_event_kiro_content(event: &SessionEvent) -> Vec<Value> {
    event
        .blocks
        .iter()
        .filter_map(|block| match block {
            EventBlock::Text { text } => Some(json!({
                "type": "text",
                "text": text
            })),
            EventBlock::Thinking { text, .. } => Some(json!({
                "thinking": text
            })),
            _ => {
                let text = canonical_visible_block_text(block)?;
                (!text.trim().is_empty()).then(|| {
                    json!({
                        "type": "text",
                        "text": text
                    })
                })
            }
        })
        .collect()
}

fn session_list_paths(global_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let global_sessions = global_dir.join("sessions").join("sessions.json");
    if global_sessions.exists() {
        paths.push(global_sessions);
    }

    let workspace_root = global_dir.join("workspace-sessions");
    if workspace_root.exists() {
        for entry in std::fs::read_dir(&workspace_root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let list_path = path.join("sessions.json");
                if list_path.exists() {
                    paths.push(list_path);
                }
            }
        }
    }

    Ok(paths)
}

fn sessions_folder(global_dir: &Path, workspace_dir: Option<&str>) -> PathBuf {
    match workspace_dir.filter(|value| !value.trim().is_empty()) {
        Some(workspace) => global_dir
            .join("workspace-sessions")
            .join(workspace_hash(workspace)),
        None => global_dir.join("sessions"),
    }
}

fn workspace_hash(path: &str) -> String {
    STANDARD.encode(path).replace(['/', '+', '='], "_")
}

fn read_session_list(path: &Path) -> Result<Vec<Value>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read Kiro session list: {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse Kiro session list: {}", path.display()))?;
    Ok(value.as_array().cloned().unwrap_or_default())
}

fn write_session_list(path: &Path, entries: &[Value]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_file_atomically(path, &serde_json::to_vec_pretty(entries)?)
}

fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("Kiro write target has no parent directory")?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .context("Kiro write target has an invalid file name")?;
    let temporary_path = parent.join(format!(".{file_name}.memorph-{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<()> {
        let mut file = File::create(&temporary_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary_path, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    write_result
}

fn fail_kiro_mutation_after_write(mutation: ProviderSourceMutation) -> Result<()> {
    #[cfg(test)]
    {
        let mut configured = TEST_KIRO_MUTATION_FAILURE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if configured.as_ref() == Some(&mutation) {
            *configured = None;
            anyhow::bail!("injected Kiro mutation failure after provider write");
        }
    }
    let _ = mutation;
    Ok(())
}

fn upsert_session_list_entry(path: &Path, entry: Value) -> Result<()> {
    let mut entries = if path.exists() {
        read_session_list(path)?
    } else {
        Vec::new()
    };
    let session_id = entry.get("sessionId").and_then(|v| v.as_str());
    let mut replaced = false;

    if let Some(session_id) = session_id {
        for existing in &mut entries {
            if existing.get("sessionId").and_then(|v| v.as_str()) == Some(session_id) {
                *existing = entry.clone();
                replaced = true;
                break;
            }
        }
    }

    if !replaced {
        entries.push(entry);
    }
    write_session_list(path, &entries)
}

fn parse_ms(value: &Value) -> Option<i64> {
    if let Some(ms) = value.as_i64() {
        return Some(ms);
    }
    value.as_str()?.parse::<i64>().ok()
}

fn path_mtime_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::session_management, storage::local_store};
    use tempfile::tempdir;

    static TEST_KIRO_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    struct TestKiroGlobalDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestKiroGlobalDirGuard {
        fn drop(&mut self) {
            crate::cache::global_cache().invalidate(PROVIDER_ID);
            set_test_kiro_mutation_failure(None);
            backup::set_test_backup_failure(false);
            *TEST_KIRO_GLOBAL_DIR
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }

    fn use_test_kiro_global_dir(path: PathBuf) -> TestKiroGlobalDirGuard {
        let lock = TEST_KIRO_TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *TEST_KIRO_GLOBAL_DIR
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        TestKiroGlobalDirGuard { _lock: lock }
    }

    fn set_test_kiro_mutation_failure(mutation: Option<ProviderSourceMutation>) {
        *TEST_KIRO_MUTATION_FAILURE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = mutation;
    }

    fn kiro_audit_fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/providers/kiro/fixtures/v1_0_138")
    }

    fn read_jsonl_values(path: &Path) -> Vec<Result<Value, serde_json::Error>> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect()
    }

    fn write_native_kiro_scope(global_dir: &Path, scope_dir: &Path, session_id: &str, title: &str) {
        let scope_dir = global_dir.join(scope_dir);
        std::fs::create_dir_all(&scope_dir).unwrap();
        write_session_list(
            &scope_dir.join("sessions.json"),
            &[
                json!({
                    "sessionId": "other-session",
                    "title": "Other",
                    "nativeIndex": "preserve"
                }),
                json!({
                    "sessionId": session_id,
                    "title": title,
                    "nativeIndex": "target"
                }),
            ],
        )
        .unwrap();
        std::fs::write(
            scope_dir.join(format!("{session_id}.json")),
            serde_json::to_vec_pretty(&json!({
                "sessionId": session_id,
                "title": title,
                "history": [],
                "nativeSession": {"preserve": true}
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn target_entry(path: &Path, session_id: &str) -> Value {
        read_session_list(path)
            .unwrap()
            .into_iter()
            .find(|entry| entry.get("sessionId").and_then(Value::as_str) == Some(session_id))
            .unwrap()
    }

    fn session_value(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn kiro_v2_audit_fixture_matches_official_session_directory_contract() {
        use sha2::{Digest, Sha256};

        let root = kiro_audit_fixture_root();
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("fixture.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["provider"], "kiro");
        assert_eq!(manifest["source_plane"], "kiro-agent-v2");
        assert_eq!(manifest["observed_ide_version"], "1.0.138");
        assert_eq!(manifest["observed_extension_version"], "1.0.231");
        assert_eq!(manifest["observed_schema_version"], "1.0.0");
        assert_eq!(manifest["observed_data_model_version"], 1);
        assert_eq!(manifest["raw_user_content_committed"], false);
        assert_eq!(manifest["storage_root"], "~/.kiro/sessions");
        assert_eq!(
            manifest["official_artifact_sha256"],
            "29c7541056b4ca6849d73c1062ae1d215a80a9f7fc74a8240cb2bf9b8e1fd68b"
        );

        let session_id = manifest["normal_session_id"].as_str().unwrap();
        let workspace_path = "/workspace/sanitized-project";
        let workspace_hash = format!("{:x}", Sha256::digest(workspace_path.as_bytes()));
        let workspace_hash = &workspace_hash[..16];
        assert_eq!(workspace_hash, "8f3d1d8bb1bd8116");

        let session_dir = root.join("sessions").join(workspace_hash).join(session_id);
        let metadata: Value = serde_json::from_str(
            &std::fs::read_to_string(session_dir.join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["schemaVersion"], "1.0.0");
        assert_eq!(metadata["dataModelVersion"], 1);
        assert_eq!(metadata["id"], session_id);
        assert_eq!(metadata["workspacePaths"], json!([workspace_path]));
        assert_eq!(metadata["title"], "Sanitized Kiro session");
        assert_eq!(metadata["status"], "completed");

        assert!(session_dir.join("messages.jsonl").is_file());
        assert!(session_dir.join("sub-executions/subexec-1.jsonl").is_file());
        assert!(session_dir
            .join("tool-outputs/tool-1-a1b2c3d4.txt")
            .is_file());
        assert!(session_dir
            .join("snapshots/snap0001/src/example.rs")
            .is_file());
        assert!(session_dir.join("snapshots/snap0001/.hash").is_file());

        let messages = read_jsonl_values(&session_dir.join("messages.jsonl"));
        assert_eq!(messages.len(), 10);
        assert!(messages.iter().all(Result::is_ok));
        let payload_types = messages
            .into_iter()
            .map(Result::unwrap)
            .map(|message| message["payload"]["type"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            payload_types,
            [
                "session_start",
                "turn_start",
                "user",
                "assistant",
                "tool_call",
                "tool_result",
                "assistant",
                "usage_summary",
                "turn_end",
                "session_metadata",
            ]
        );

        let global_id = manifest["global_session_id"].as_str().unwrap();
        let global_dir = root.join("sessions").join("_global").join(global_id);
        let global_metadata: Value = serde_json::from_str(
            &std::fs::read_to_string(global_dir.join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(global_metadata["id"], global_id);
        assert_eq!(global_metadata["workspacePaths"], json!([]));
        assert_eq!(
            read_jsonl_values(&global_dir.join("messages.jsonl")).len(),
            4
        );
    }

    #[test]
    fn kiro_v2_audit_fixture_covers_projection_changes_and_invalid_records() {
        let root = kiro_audit_fixture_root();
        let variants = root.join("variants");
        let normal_dir = root
            .join("sessions/8f3d1d8bb1bd8116")
            .join("sess_11111111-1111-4111-8111-111111111111");

        let original_metadata: Value = serde_json::from_str(
            &std::fs::read_to_string(normal_dir.join("session.json")).unwrap(),
        )
        .unwrap();
        let updated_metadata: Value = serde_json::from_str(
            &std::fs::read_to_string(variants.join("session.updated.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(original_metadata["id"], updated_metadata["id"]);
        assert_ne!(original_metadata["title"], updated_metadata["title"]);
        assert_ne!(
            original_metadata["lastModifiedAt"],
            updated_metadata["lastModifiedAt"]
        );

        assert_eq!(
            read_jsonl_values(&normal_dir.join("messages.jsonl")).len(),
            10
        );
        assert_eq!(
            read_jsonl_values(&variants.join("messages.updated.jsonl")).len(),
            14
        );
        assert_eq!(
            read_jsonl_values(&normal_dir.join("sub-executions/subexec-1.jsonl")).len(),
            2
        );
        assert_eq!(
            read_jsonl_values(&variants.join("sub-execution.updated.jsonl")).len(),
            3
        );

        let malformed = read_jsonl_values(&variants.join("messages.malformed.jsonl"));
        assert_eq!(malformed.len(), 3);
        assert_eq!(malformed.iter().filter(|value| value.is_ok()).count(), 2);
        assert_eq!(malformed.iter().filter(|value| value.is_err()).count(), 1);

        let unknown = read_jsonl_values(&variants.join("messages.unknown.jsonl"));
        assert_eq!(unknown.len(), 1);
        assert_eq!(
            unknown[0].as_ref().unwrap()["payload"]["type"],
            "future_kiro_payload"
        );
        assert_eq!(
            unknown[0].as_ref().unwrap()["payload"]["futureField"]["preserve"],
            true
        );
    }

    #[test]
    fn scans_and_loads_workspace_session() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let global_dir = temp.path().join("kiro.kiroagent");
        let workspace = "/tmp/kiro-project";
        let sessions_dir = sessions_folder(&global_dir, Some(workspace));
        std::fs::create_dir_all(&sessions_dir)?;

        let session_id = "kiro-session-1";
        write_session_list(
            &sessions_dir.join("sessions.json"),
            &[json!({
                "sessionId": session_id,
                "title": "Hello Kiro",
                "dateCreated": "1700000000000",
                "workspaceDirectory": workspace,
                "hidden": false
            })],
        )?;
        std::fs::write(
            sessions_dir.join(format!("{}.json", session_id)),
            serde_json::to_string_pretty(&json!({
                "history": [
                    {
                        "message": {
                            "id": "m1",
                            "role": "user",
                            "content": [{"type": "text", "text": "hi"}]
                        }
                    },
                    {
                        "message": {
                            "id": "m2",
                            "role": "assistant",
                            "content": [
                                {"thinking": "checking"},
                                {"type": "text", "text": "hello"},
                                {"type": "kiro_extra", "payload": {"ok": true}}
                            ]
                        }
                    }
                ],
                "sessionId": session_id,
                "title": "Hello Kiro",
                "workspaceDirectory": workspace,
                "sessionType": "vibe",
                "contextUsagePercentage": 0
            }))?,
        )?;

        let sessions = scan_sessions_in(&global_dir)?;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, session_id);
        assert_eq!(sessions[0].title.as_deref(), Some("Hello Kiro"));
        assert_eq!(sessions[0].project_dir.as_deref(), Some(workspace));

        let imported = import_canonical_session_from_path(Path::new(
            sessions[0].source_path.as_ref().unwrap(),
        ))?;
        assert_eq!(imported.session.identity.canonical_id, session_id);
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Hello Kiro")
        );
        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some(workspace)
        );
        assert_eq!(imported.session.events.len(), 2);
        assert!(matches!(
            imported.session.events[0].blocks.first(),
            Some(EventBlock::Text { text }) if text == "hi"
        ));
        assert!(imported.session.events[1]
            .blocks
            .iter()
            .any(|block| matches!(
                block,
                EventBlock::Thinking { text, .. } if text == "checking"
            )));
        assert!(imported.session.events[1]
            .blocks
            .iter()
            .any(|block| matches!(
                block,
                EventBlock::ProviderPayload { kind, .. } if kind == "kiro_extra"
            )));

        Ok(())
    }

    #[test]
    fn compressed_segment_exports_as_portable_kiro_text() {
        let event = SessionEvent {
            id: "compressed-source".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Compressed {
                source_provider_id: "opencode".to_string(),
                summary: "compressed summary".to_string(),
                source_event_ids: vec![
                    "old-event-1".to_string(),
                    "old-event-2".to_string(),
                    "old-event-3".to_string(),
                ],
                source_event_count: None,
                archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "memorph".to_string(),
                    original_id: None,
                    original_role: Some("assistant".to_string()),
                    phase: Some("compression".to_string()),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        let item = canonical_event_to_kiro_history_item(&event).expect("kiro history item");
        let text = item
            .pointer("/message/content/0/text")
            .and_then(Value::as_str)
            .expect("portable compressed text");

        assert!(text.contains("[Compressed session segment from opencode]"));
        assert!(text.contains("compressed summary"));
        assert!(text.contains("Source event count: 3"));
        assert!(text.contains("Archive: memorph-archive://s1/archive.json.gz"));
        assert!(text.contains("memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"));
        assert!(!text.contains("old-event-1"));
        assert!(!text.contains("old-event-2"));
        assert!(!text.contains("old-event-3"));
    }

    #[test]
    fn internal_events_do_not_export_as_kiro_history_items() {
        let event = SessionEvent {
            id: "internal".to_string(),
            kind: SessionEventKind::Lifecycle,
            role: EventRole::System,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: "internal context".to_string(),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "codex".to_string(),
                    original_id: None,
                    original_role: Some("user".to_string()),
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        assert!(canonical_event_to_kiro_history_item(&event).is_none());
    }

    #[test]
    fn write_rename_delete_roundtrip() -> Result<()> {
        let _lock = TEST_KIRO_TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir()?;
        let global_dir = temp.path().join("kiro.kiroagent");
        let target_dir = temp.path().join("project");
        std::fs::create_dir_all(&target_dir)?;

        let now = Utc::now();
        let source = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "source-session".to_string(),
                source_title: Some("Imported Session".to_string()),
            },
            provenance: SessionProvenance {
                imported_at: now,
                imported_by: Some("memorph-test".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: "source-session".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: Some(target_dir.to_string_lossy().to_string()),
                created_at: Some(now),
                last_active_at: Some(now),
                tags: Vec::new(),
            },
            events: vec![
                SessionEvent {
                    id: "user-1".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: now,
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "Build this".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: PROVIDER_ID.to_string(),
                            original_id: None,
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
                SessionEvent {
                    id: "assistant-1".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::Assistant,
                    timestamp: now,
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "Done".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: PROVIDER_ID.to_string(),
                            original_id: None,
                            original_role: Some("assistant".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };

        let new_id = export_canonical_session_in(&global_dir, &source, &target_dir)?;
        let sessions = scan_sessions_in(&global_dir)?;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, new_id);
        assert_eq!(sessions[0].title.as_deref(), Some("Imported Session"));

        let imported = import_canonical_session_from_path(Path::new(
            sessions[0].source_path.as_ref().unwrap(),
        ))?;
        let canonical_global_dir = temp.path().join("canonical-kiro.kiroagent");
        let canonical_id =
            export_canonical_session_in(&canonical_global_dir, &imported.session, &target_dir)?;
        let canonical_sessions = scan_sessions_in(&canonical_global_dir)?;
        assert_eq!(canonical_sessions.len(), 1);
        assert_eq!(canonical_sessions[0].session_id, canonical_id);
        assert_eq!(
            canonical_sessions[0].title.as_deref(),
            Some("Imported Session")
        );

        backup::rename_session(&global_dir, &new_id, "Renamed")?;
        let renamed = scan_sessions_in(&global_dir)?;
        assert_eq!(renamed[0].title.as_deref(), Some("Renamed"));

        backup::delete_session(&global_dir, &new_id)?;
        assert!(scan_sessions_in(&global_dir)?.is_empty());

        Ok(())
    }

    #[test]
    fn delete_backup_restores_all_kiro_scopes_and_preserves_unrelated_index_changes() {
        let dir = tempdir().unwrap();
        let global_dir = dir.path().join("kiro.kiroagent");
        let _guard = use_test_kiro_global_dir(global_dir.clone());
        let session_id = "kiro-delete";
        let scopes = [
            PathBuf::from("sessions"),
            PathBuf::from("workspace-sessions/workspace-a"),
            PathBuf::from("workspace-sessions/workspace-b"),
        ];
        for scope in &scopes {
            write_native_kiro_scope(&global_dir, scope, session_id, "Before");
        }
        let backup = KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kiro-delete",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        KiroProvider.delete_session(session_id).unwrap();
        for scope in &scopes {
            let list_path = global_dir.join(scope).join("sessions.json");
            let mut entries = read_session_list(&list_path).unwrap();
            entries.push(json!({
                "sessionId": format!("concurrent-{}", scope.display()),
                "title": "Concurrent"
            }));
            write_session_list(&list_path, &entries).unwrap();
        }

        KiroProvider.restore_session_backup(&backup).unwrap();
        KiroProvider.restore_session_backup(&backup).unwrap();

        for scope in &scopes {
            let scope_dir = global_dir.join(scope);
            assert_eq!(
                target_entry(&scope_dir.join("sessions.json"), session_id)["title"],
                "Before"
            );
            assert!(read_session_list(&scope_dir.join("sessions.json"))
                .unwrap()
                .iter()
                .any(|entry| entry["title"] == "Concurrent"));
            assert_eq!(
                session_value(&scope_dir.join(format!("{session_id}.json")))["nativeSession"]
                    ["preserve"],
                true
            );
        }
    }

    #[test]
    fn rename_restore_only_restores_titles_and_preserves_concurrent_changes() {
        let dir = tempdir().unwrap();
        let global_dir = dir.path().join("kiro.kiroagent");
        let _guard = use_test_kiro_global_dir(global_dir.clone());
        let session_id = "kiro-rename";
        let scope = PathBuf::from("workspace-sessions/workspace-a");
        write_native_kiro_scope(&global_dir, &scope, session_id, "Before");
        let backup = KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-kiro-rename",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        KiroProvider.rename_session(session_id, "After").unwrap();
        let scope_dir = global_dir.join(&scope);
        let list_path = scope_dir.join("sessions.json");
        let mut entries = read_session_list(&list_path).unwrap();
        entries
            .iter_mut()
            .find(|entry| entry["sessionId"] == session_id)
            .unwrap()["concurrentIndex"] = json!("keep");
        entries.push(json!({"sessionId": "concurrent", "title": "Concurrent"}));
        write_session_list(&list_path, &entries).unwrap();
        let session_path = scope_dir.join(format!("{session_id}.json"));
        let mut session = session_value(&session_path);
        session["concurrentSession"] = json!("keep");
        write_file_atomically(&session_path, &serde_json::to_vec_pretty(&session).unwrap())
            .unwrap();

        KiroProvider.restore_session_backup(&backup).unwrap();

        let restored_entry = target_entry(&list_path, session_id);
        assert_eq!(restored_entry["title"], "Before");
        assert_eq!(restored_entry["concurrentIndex"], "keep");
        assert!(read_session_list(&list_path)
            .unwrap()
            .iter()
            .any(|entry| entry["sessionId"] == "concurrent"));
        let restored_session = session_value(&session_path);
        assert_eq!(restored_session["title"], "Before");
        assert_eq!(restored_session["concurrentSession"], "keep");
    }

    #[test]
    fn rename_restore_does_not_recreate_concurrently_deleted_targets() {
        let dir = tempdir().unwrap();
        let global_dir = dir.path().join("kiro.kiroagent");
        let _guard = use_test_kiro_global_dir(global_dir.clone());
        let session_id = "kiro-concurrent-delete";
        let scope = PathBuf::from("sessions");
        write_native_kiro_scope(&global_dir, &scope, session_id, "Before");
        let backup = KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-kiro-concurrent-delete",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        KiroProvider.rename_session(session_id, "After").unwrap();

        let scope_dir = global_dir.join(scope);
        let list_path = scope_dir.join("sessions.json");
        let mut entries = read_session_list(&list_path).unwrap();
        entries.retain(|entry| entry["sessionId"] != session_id);
        write_session_list(&list_path, &entries).unwrap();
        let session_path = scope_dir.join(format!("{session_id}.json"));
        std::fs::remove_file(&session_path).unwrap();

        KiroProvider.restore_session_backup(&backup).unwrap();

        assert!(!read_session_list(&list_path)
            .unwrap()
            .iter()
            .any(|entry| entry["sessionId"] == session_id));
        assert!(!session_path.exists());
    }

    #[test]
    fn kiro_backup_rejects_ambiguous_invalid_and_unsafe_sources() {
        let dir = tempdir().unwrap();
        let global_dir = dir.path().join("kiro.kiroagent");
        let _guard = use_test_kiro_global_dir(global_dir.clone());
        let session_id = "kiro-invalid";
        let scope = global_dir.join("sessions");
        std::fs::create_dir_all(&scope).unwrap();
        write_session_list(
            &scope.join("sessions.json"),
            &[
                json!({"sessionId": session_id, "title": "One"}),
                json!({"sessionId": session_id, "title": "Two"}),
            ],
        )
        .unwrap();
        std::fs::write(
            scope.join(format!("{session_id}.json")),
            serde_json::to_vec(&json!({"sessionId": session_id})).unwrap(),
        )
        .unwrap();
        assert!(KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-duplicate",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err()
            .to_string()
            .contains("duplicate entries"));

        std::fs::write(scope.join("sessions.json"), b"{}").unwrap();
        assert!(KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-non-array",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err()
            .to_string()
            .contains("must contain a JSON array"));

        write_session_list(
            &scope.join("sessions.json"),
            &[json!({"sessionId": session_id})],
        )
        .unwrap();
        std::fs::write(
            scope.join(format!("{session_id}.json")),
            serde_json::to_vec(&json!({"sessionId": "wrong"})).unwrap(),
        )
        .unwrap();
        assert!(KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-wrong-id",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err()
            .to_string()
            .contains("identity does not match"));

        std::fs::write(
            scope.join(format!("{session_id}.json")),
            serde_json::to_vec(&json!(["not", "an", "object"])).unwrap(),
        )
        .unwrap();
        assert!(KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-non-object",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err()
            .to_string()
            .contains("must contain a JSON object"));

        #[cfg(unix)]
        {
            let real_list = scope.join("real-sessions.json");
            write_session_list(&real_list, &[json!({"sessionId": session_id})]).unwrap();
            std::fs::remove_file(scope.join("sessions.json")).unwrap();
            std::os::unix::fs::symlink(&real_list, scope.join("sessions.json")).unwrap();
            assert!(KiroProvider
                .create_session_backup(
                    ProviderSourceMutation::Delete,
                    "operation-symlink",
                    session_id,
                    &dir.path().join("backups"),
                )
                .unwrap_err()
                .to_string()
                .contains("not a regular file"));
        }
    }

    #[test]
    fn kiro_restore_rejects_payload_and_source_path_tampering_before_writes() {
        let dir = tempdir().unwrap();
        let global_dir = dir.path().join("kiro.kiroagent");
        let _guard = use_test_kiro_global_dir(global_dir.clone());
        let session_id = "kiro-tamper";
        write_native_kiro_scope(&global_dir, Path::new("sessions"), session_id, "Before");
        let backup_root = dir.path().join("backups");
        let payload_backup = KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-payload-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        KiroProvider.delete_session(session_id).unwrap();
        std::fs::write(
            payload_backup.backup_path.join("files/0000-session.json"),
            b"tampered",
        )
        .unwrap();
        assert!(KiroProvider
            .restore_session_backup(&payload_backup)
            .unwrap_err()
            .to_string()
            .contains("does not match its manifest"));
        assert!(
            !read_session_list(&global_dir.join("sessions/sessions.json"))
                .unwrap()
                .iter()
                .any(|entry| entry["sessionId"] == session_id)
        );
        assert!(!global_dir
            .join("sessions")
            .join(format!("{session_id}.json"))
            .exists());

        write_native_kiro_scope(&global_dir, Path::new("sessions"), session_id, "Before");
        let path_backup = KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-path-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        let metadata_path = path_backup.backup_path.join("metadata.json");
        let mut metadata: Value =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        metadata["scopes"][0]["scope_dir"] = json!("../outside");
        std::fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        assert!(KiroProvider
            .restore_session_backup(&path_backup)
            .unwrap_err()
            .to_string()
            .contains("scope path is invalid"));
    }

    #[test]
    fn kiro_backup_contract_registration_and_partial_failure_recovery() {
        let dir = tempdir().unwrap();
        let global_dir = dir.path().join("kiro.kiroagent");
        let _guard = use_test_kiro_global_dir(global_dir.clone());
        let delete_id = "kiro-partial-delete";
        let rename_id = "kiro-partial-rename";
        write_native_kiro_scope(
            &global_dir,
            Path::new("sessions"),
            delete_id,
            "Delete Before",
        );
        write_native_kiro_scope(
            &global_dir,
            Path::new("workspace-sessions/workspace-a"),
            rename_id,
            "Rename Before",
        );
        let backup_root = dir.path().join("backups");
        let contract = KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-contract",
                delete_id,
                &backup_root,
            )
            .unwrap();
        let capabilities = KiroProvider.capabilities();
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert_eq!(contract.source_path, global_dir.canonicalize().unwrap());
        assert_eq!(contract.format, "kiro-session-backup-v1");
        assert_eq!(
            contract.mime_type,
            "application/vnd.memorph.kiro-session-backup"
        );
        assert!(contract.backup_path.join("metadata.json").is_file());
        assert!(contract
            .backup_path
            .join("files/0000-session.json")
            .is_file());

        let mut unconfigured_artifact_conn = rusqlite::Connection::open_in_memory().unwrap();
        let registration_results = session_management::delete_sessions(
            PROVIDER_ID,
            &[delete_id],
            &["operation-registration".to_string()],
            &backup_root,
            &mut unconfigured_artifact_conn,
        );
        assert!(registration_results[0]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("Delete cancelled before provider write"));
        assert!(global_dir
            .join("sessions")
            .join(format!("{delete_id}.json"))
            .is_file());

        let mut artifact_conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();
        set_test_kiro_mutation_failure(Some(ProviderSourceMutation::Delete));
        let delete_results = session_management::delete_sessions(
            PROVIDER_ID,
            &[delete_id],
            &["operation-partial-delete".to_string()],
            &backup_root,
            &mut artifact_conn,
        );
        assert!(delete_results[0]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            target_entry(&global_dir.join("sessions/sessions.json"), delete_id)["title"],
            "Delete Before"
        );

        set_test_kiro_mutation_failure(Some(ProviderSourceMutation::Rename));
        let rename_error = session_management::rename_session(
            PROVIDER_ID,
            rename_id,
            "After",
            "operation-partial-rename",
            &backup_root,
            &mut artifact_conn,
        )
        .unwrap_err();
        assert!(rename_error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            target_entry(
                &global_dir.join("workspace-sessions/workspace-a/sessions.json"),
                rename_id,
            )["title"],
            "Rename Before"
        );
    }

    #[test]
    fn failed_kiro_backup_creation_removes_operation_directory() {
        let dir = tempdir().unwrap();
        let global_dir = dir.path().join("kiro.kiroagent");
        let _guard = use_test_kiro_global_dir(global_dir.clone());
        let session_id = "kiro-backup-failure";
        write_native_kiro_scope(&global_dir, Path::new("sessions"), session_id, "Before");
        backup::set_test_backup_failure(true);

        let error = KiroProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-backup-failure",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err();

        assert!(error.to_string().contains("injected Kiro backup failure"));
        assert!(!dir
            .path()
            .join("backups/kiro/operation-backup-failure")
            .exists());
    }
}
