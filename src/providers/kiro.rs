use crate::model::{
    ContentBlock, MemorphMessage, MemorphMeta, MemorphRole, MemorphSession, MessageMetadata,
    SessionInfo, SessionMeta, SourceMetadata,
};
use crate::provider::{Provider, ProviderCapabilities};
use crate::utils::truncate_summary;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde_json::{json, Value};
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
            load: true,
            write: true,
            delete: true,
            rename: true,
            resume: false,
        }
    }

    fn scan_sessions(&self) -> Result<Vec<SessionMeta>> {
        let global_dir = kiro_global_storage_dir()?;
        if !global_dir.exists() {
            return Ok(Vec::new());
        }
        scan_sessions_in(&global_dir)
    }

    fn load_session(&self, source_path: &str) -> Result<MemorphSession> {
        load_session_from_path(Path::new(source_path))
    }

    fn write_session(&self, session: &MemorphSession, target_dir: &Path) -> Result<String> {
        let global_dir = kiro_global_storage_dir()?;
        write_session_in(&global_dir, session, target_dir)
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

fn scan_sessions_in(global_dir: &Path) -> Result<Vec<SessionMeta>> {
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

            sessions.push(SessionMeta {
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

fn load_session_from_path(path: &Path) -> Result<MemorphSession> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read Kiro session: {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse Kiro session: {}", path.display()))?;

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
    let project_dir = value
        .get("workspaceDirectory")
        .and_then(|v| v.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    let session_time = path_mtime_ms(path).unwrap_or_else(|| Utc::now().timestamp_millis());
    let session_dt = chrono::DateTime::from_timestamp_millis(session_time).unwrap_or_else(Utc::now);

    let mut messages = Vec::new();
    if let Some(history) = value.get("history").and_then(|v| v.as_array()) {
        for (index, item) in history.iter().enumerate() {
            let Some(message) = item.get("message") else {
                continue;
            };
            let role_str = message
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user");
            let role = match role_str {
                "user" => MemorphRole::User,
                "assistant" | "bot" => MemorphRole::Assistant,
                "tool" => MemorphRole::Tool,
                "system" => MemorphRole::System,
                "developer" => MemorphRole::Developer,
                _ => continue,
            };
            let content = parse_kiro_content(message.get("content"));
            if content.is_empty() {
                continue;
            }
            let id = message
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            messages.push(MemorphMessage {
                id,
                role,
                content,
                timestamp: session_dt,
                metadata: Some(MessageMetadata {
                    source: Some(SourceMetadata {
                        provider: PROVIDER_ID.to_string(),
                        original_id: None,
                        original_role: Some(role_str.to_string()),
                    }),
                    model: None,
                    usage: None,
                    extra: json!({}),
                }),
                parent_id: None,
                turn_index: Some(index as u32),
            });
        }
    }

    Ok(MemorphSession {
        meta: MemorphMeta {
            version: "1.0".to_string(),
            converted_from: PROVIDER_ID.to_string(),
            converted_at: Utc::now(),
            memorph_version: env!("CARGO_PKG_VERSION").to_string(),
            source_session_id: session_id.clone(),
            source_provider: PROVIDER_ID.to_string(),
            converted_by: Some("memorph-cli".to_string()),
        },
        session: SessionInfo {
            id: session_id,
            title,
            project_dir,
            created_at: Some(session_dt),
            last_active_at: Some(session_dt),
            tags: None,
        },
        messages,
    })
}

fn write_session_in(
    global_dir: &Path,
    session: &MemorphSession,
    target_dir: &Path,
) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = infer_title(session);
    let now = Utc::now().timestamp_millis();
    let created = session
        .session
        .created_at
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(now);

    let history: Vec<Value> = session
        .messages
        .iter()
        .filter_map(memorph_message_to_kiro)
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

fn parse_kiro_content(value: Option<&Value>) -> Vec<ContentBlock> {
    match value {
        Some(Value::String(text)) => text_block(text).into_iter().collect(),
        Some(Value::Array(items)) => items.iter().filter_map(parse_kiro_content_item).collect(),
        Some(Value::Object(object)) => object
            .get("text")
            .and_then(|v| v.as_str())
            .and_then(text_block)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_kiro_content_item(value: &Value) -> Option<ContentBlock> {
    let object = value.as_object()?;
    if let Some(text) = object.get("text").and_then(|v| v.as_str()) {
        return text_block(text);
    }
    if let Some(thinking) = object.get("thinking").and_then(|v| v.as_str()) {
        if !thinking.trim().is_empty() {
            return Some(ContentBlock::Thinking {
                thinking: thinking.to_string(),
                signature: None,
            });
        }
    }
    None
}

fn memorph_message_to_kiro(message: &MemorphMessage) -> Option<Value> {
    let text = message_text(message);
    if text.trim().is_empty() {
        return None;
    }

    let role = match message.role {
        MemorphRole::Assistant => "assistant",
        _ => "user",
    };

    Some(json!({
        "message": {
            "id": message.id,
            "role": role,
            "content": [
                {
                    "type": "text",
                    "text": text
                }
            ]
        },
        "contextItems": [],
        "editorState": {}
    }))
}

fn message_text(message: &MemorphMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::Thinking { thinking, .. } => Some(format!("[Thinking]\n{}", thinking)),
            ContentBlock::ToolUse { id, name, input } => {
                let input_text = input
                    .as_ref()
                    .map(|value| format!("\n{}", value))
                    .unwrap_or_default();
                Some(format!("[Tool use: {} ({})]{}", name, id, input_text))
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let prefix = if is_error.unwrap_or(false) {
                    "Tool error"
                } else {
                    "Tool result"
                };
                Some(format!("[{}: {}]\n{}", prefix, tool_use_id, content))
            }
            ContentBlock::Image { mime_type, .. } => Some(format!("[Image: {}]", mime_type)),
            ContentBlock::File { path, content } => {
                let body = content
                    .as_ref()
                    .map(|value| format!("\n{}", value))
                    .unwrap_or_default();
                Some(format!("[File: {}]{}", path, body))
            }
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn infer_title(session: &MemorphSession) -> String {
    if let Some(title) = session
        .session
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return truncate_summary(title, TITLE_MAX_CHARS);
    }

    session
        .messages
        .iter()
        .map(message_text)
        .find(|text| !text.trim().is_empty())
        .map(|text| truncate_summary(&text, TITLE_MAX_CHARS))
        .unwrap_or_else(|| "Imported from memorph".to_string())
}

fn text_block(text: &str) -> Option<ContentBlock> {
    if text.trim().is_empty() {
        None
    } else {
        Some(ContentBlock::text(text))
    }
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
    use crate::model::MemorphMeta;

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
                            "content": "hello"
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

        let loaded = load_session_from_path(Path::new(sessions[0].source_path.as_ref().unwrap()))?;
        assert_eq!(loaded.session.id, session_id);
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, MemorphRole::User);
        assert_eq!(loaded.messages[1].role, MemorphRole::Assistant);

        Ok(())
    }

    #[test]
    fn write_rename_delete_roundtrip() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let global_dir = temp.path().join("kiro.kiroagent");
        let target_dir = temp.path().join("project");
        std::fs::create_dir_all(&target_dir)?;

        let source = MemorphSession {
            meta: MemorphMeta::default(),
            session: SessionInfo {
                id: "source-session".to_string(),
                title: Some("Imported Session".to_string()),
                project_dir: Some(target_dir.to_string_lossy().to_string()),
                created_at: Some(Utc::now()),
                last_active_at: Some(Utc::now()),
                tags: None,
            },
            messages: vec![
                MemorphMessage {
                    id: "user-1".to_string(),
                    role: MemorphRole::User,
                    content: vec![ContentBlock::text("Build this")],
                    timestamp: Utc::now(),
                    metadata: None,
                    parent_id: None,
                    turn_index: None,
                },
                MemorphMessage {
                    id: "assistant-1".to_string(),
                    role: MemorphRole::Assistant,
                    content: vec![ContentBlock::text("Done")],
                    timestamp: Utc::now(),
                    metadata: None,
                    parent_id: None,
                    turn_index: None,
                },
            ],
        };

        let new_id = write_session_in(&global_dir, &source, &target_dir)?;
        let sessions = scan_sessions_in(&global_dir)?;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, new_id);
        assert_eq!(sessions[0].title.as_deref(), Some("Imported Session"));

        let loaded = load_session_from_path(Path::new(sessions[0].source_path.as_ref().unwrap()))?;
        assert_eq!(loaded.messages.len(), 2);

        rename_session_in(&global_dir, &new_id, "Renamed")?;
        let renamed = scan_sessions_in(&global_dir)?;
        assert_eq!(renamed[0].title.as_deref(), Some("Renamed"));

        delete_session_in(&global_dir, &new_id)?;
        assert!(scan_sessions_in(&global_dir)?.is_empty());

        Ok(())
    }
}
