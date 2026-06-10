use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::provider::{
    canonical_block_text, canonical_export_result, canonical_session_title, Provider,
    ProviderCapabilities, ProviderSessionSummary,
};
use crate::utils::truncate_summary;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct KiroProvider;

const PROVIDER_ID: &str = "kiro";
const TITLE_MAX_CHARS: usize = 80;

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
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let global_dir = kiro_global_storage_dir()?;
        if !global_dir.exists() {
            return Ok(Vec::new());
        }
        scan_sessions_in(&global_dir)
    }

    fn get_session_meta(
        &self,
        session_id: &str,
    ) -> Result<Option<ProviderSessionSummary>> {
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
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let global_dir = kiro_global_storage_dir()?;
        delete_session_in(&global_dir, session_id)
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let global_dir = kiro_global_storage_dir()?;
        rename_session_in(&global_dir, session_id, new_title)
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
            turn_index: Some(index as u32),
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
    let content = canonical_event_kiro_content(event);
    if content.is_empty() {
        return None;
    }
    let role = match event.role {
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
                let text = canonical_block_text(block);
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

fn delete_session_in(global_dir: &Path, session_id: &str) -> Result<()> {
    let mut found = false;

    for list_path in session_list_paths(global_dir)? {
        let Some(session_dir) = list_path.parent() else {
            continue;
        };

        let mut entries = read_session_list(&list_path)?;
        let original_len = entries.len();
        entries.retain(|entry| {
            entry
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(|id| id != session_id)
                .unwrap_or(true)
        });
        if entries.len() != original_len {
            write_session_list(&list_path, &entries)?;
            found = true;
        }

        let session_path = session_dir.join(format!("{}.json", session_id));
        if session_path.exists() {
            std::fs::remove_file(session_path)?;
            found = true;
        }
    }

    if !found {
        anyhow::bail!("Kiro session not found: {}", session_id);
    }

    Ok(())
}

fn rename_session_in(global_dir: &Path, session_id: &str, new_title: &str) -> Result<()> {
    let mut found = false;

    for list_path in session_list_paths(global_dir)? {
        let Some(session_dir) = list_path.parent() else {
            continue;
        };

        let mut entries = read_session_list(&list_path)?;
        let mut list_changed = false;
        for entry in &mut entries {
            if entry.get("sessionId").and_then(|v| v.as_str()) == Some(session_id) {
                if let Some(object) = entry.as_object_mut() {
                    object.insert("title".to_string(), Value::String(new_title.to_string()));
                    list_changed = true;
                    found = true;
                }
            }
        }
        if list_changed {
            write_session_list(&list_path, &entries)?;
        }

        let session_path = session_dir.join(format!("{}.json", session_id));
        if session_path.exists() {
            let raw = std::fs::read_to_string(&session_path)?;
            let mut value: Value = serde_json::from_str(&raw)?;
            if let Some(object) = value.as_object_mut() {
                object.insert("title".to_string(), Value::String(new_title.to_string()));
                std::fs::write(&session_path, serde_json::to_string_pretty(&value)?)?;
                found = true;
            }
        }
    }

    if !found {
        anyhow::bail!("Kiro session not found: {}", session_id);
    }

    Ok(())
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
    std::fs::write(path, serde_json::to_string_pretty(entries)?)?;
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
    fn write_rename_delete_roundtrip() -> Result<()> {
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

        rename_session_in(&global_dir, &new_id, "Renamed")?;
        let renamed = scan_sessions_in(&global_dir)?;
        assert_eq!(renamed[0].title.as_deref(), Some("Renamed"));

        delete_session_in(&global_dir, &new_id)?;
        assert!(scan_sessions_in(&global_dir)?.is_empty());

        Ok(())
    }
}
