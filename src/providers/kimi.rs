use crate::model::{
    ContentBlock, MemorphMessage, MemorphMeta, MemorphRole, MemorphSession, SessionInfo,
    SessionMeta,
};
use crate::provider::{Provider, ProviderCapabilities};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct KimiProvider;

const PROVIDER_ID: &str = "kimi";
const TITLE_MAX_CHARS: usize = 80;

impl Provider for KimiProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Kimi"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_session_management()
    }

    fn scan_sessions(&self) -> Result<Vec<SessionMeta>> {
        let root = get_kimi_sessions_dir();
        if !root.exists() {
            return Ok(Vec::new());
        }

        let dir_map = load_work_dir_map()?;
        let mut sessions = Vec::new();

        for entry in WalkDir::new(&root)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let depth = path.components().count() - root.components().count();
            if depth != 2 {
                continue;
            }

            let wire_path = path.join("wire.jsonl");
            if !wire_path.exists() {
                continue;
            }

            let session_id = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if session_id.is_empty() {
                continue;
            }

            let project_hash = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let state_path = path.join("state.json");
            let (title, archived) = if state_path.exists() {
                match read_state_json(&state_path) {
                    Ok(s) => (s.custom_title, s.archived),
                    Err(_) => (None, false),
                }
            } else {
                (None, false)
            };

            if archived {
                continue;
            }

            let project_dir = dir_map.get(&project_hash).cloned();

            let last_active_at = wire_last_timestamp(&wire_path);

            sessions.push(SessionMeta {
                session_id,
                title: title.filter(|t| !t.is_empty()),
                project_dir,
                last_active_at,
                source_path: Some(wire_path.to_string_lossy().to_string()),
            });
        }

        Ok(sessions)
    }

    fn load_session(&self, source_path: &str) -> Result<MemorphSession> {
        let wire_path = Path::new(source_path);
        let session_dir = wire_path
            .parent()
            .with_context(|| format!("Invalid Kimi session path: {}", source_path))?;

        let state_path = session_dir.join("state.json");
        let state = if state_path.exists() {
            read_state_json(&state_path).ok()
        } else {
            None
        };

        let session_id = session_dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        let (messages, created_at, last_active_at) = load_from_wire(wire_path)?;

        let meta = MemorphMeta {
            version: "1.0".to_string(),
            converted_from: PROVIDER_ID.to_string(),
            converted_at: Utc::now(),
            memorph_version: env!("CARGO_PKG_VERSION").to_string(),
            source_session_id: session_id.clone().unwrap_or_default(),
            source_provider: PROVIDER_ID.to_string(),
            converted_by: Some("memorph-cli".to_string()),
        };

        let session = SessionInfo {
            id: session_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            title: state.and_then(|s| s.custom_title).filter(|t| !t.is_empty()),
            project_dir: None,
            created_at,
            last_active_at,
            tags: None,
        };

        Ok(MemorphSession {
            meta,
            session,
            messages,
        })
    }

    fn write_session(&self, session: &MemorphSession, target_dir: &Path) -> Result<String> {
        let session_id = Uuid::new_v4().to_string();
        let project_hash = md5_hex(target_dir.to_string_lossy().as_bytes());
        let session_dir = get_kimi_sessions_dir()
            .join(&project_hash)
            .join(&session_id);
        std::fs::create_dir_all(&session_dir)?;

        let wire_path = session_dir.join("wire.jsonl");
        let context_path = session_dir.join("context.jsonl");
        let state_path = session_dir.join("state.json");

        let mut wire_file = File::create(&wire_path)?;
        let mut context_file = File::create(&context_path)?;

        // Metadata line
        let meta_line = serde_json::json!({"type": "metadata", "protocol_version": "1.9"});
        writeln!(wire_file, "{}", meta_line)?;

        let title = session
            .session
            .title
            .clone()
            .or_else(|| {
                session
                    .messages
                    .iter()
                    .find(|m| m.role == MemorphRole::User)
                    .and_then(|m| {
                        m.content.iter().find_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                    })
            })
            .unwrap_or_else(|| "Imported session".to_string());

        // Write messages
        let mut i = 0;
        while i < session.messages.len() {
            let msg = &session.messages[i];
            let ts = msg.timestamp.timestamp_millis() as f64 / 1000.0;

            match msg.role {
                MemorphRole::User
                | MemorphRole::Tool
                | MemorphRole::System
                | MemorphRole::Developer => {
                    let text = msg
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    // wire.jsonl
                    let turn_begin = serde_json::json!({
                        "timestamp": ts,
                        "message": {
                            "type": "TurnBegin",
                            "payload": {
                                "user_input": [{"type": "text", "text": text}]
                            }
                        }
                    });
                    writeln!(wire_file, "{}", turn_begin)?;

                    let step_begin = serde_json::json!({
                        "timestamp": ts,
                        "message": {
                            "type": "StepBegin",
                            "payload": {"n": 1}
                        }
                    });
                    writeln!(wire_file, "{}", step_begin)?;

                    // context.jsonl
                    let ctx_line = serde_json::json!({"role": "user", "content": text});
                    writeln!(context_file, "{}", ctx_line)?;

                    // Write assistant content if next message is assistant
                    i += 1;
                    if i < session.messages.len()
                        && session.messages[i].role == MemorphRole::Assistant
                    {
                        let assistant = &session.messages[i];
                        let assistant_ts = assistant.timestamp.timestamp_millis() as f64 / 1000.0;
                        let mut assistant_content = Vec::new();

                        for block in &assistant.content {
                            match block {
                                ContentBlock::Text { text } => {
                                    let part = serde_json::json!({
                                        "timestamp": assistant_ts,
                                        "message": {
                                            "type": "ContentPart",
                                            "payload": {"type": "text", "text": text}
                                        }
                                    });
                                    writeln!(wire_file, "{}", part)?;
                                    assistant_content.push(serde_json::json!({
                                        "type": "text",
                                        "text": text
                                    }));
                                }
                                ContentBlock::Thinking { thinking, .. } => {
                                    let part = serde_json::json!({
                                        "timestamp": assistant_ts,
                                        "message": {
                                            "type": "ContentPart",
                                            "payload": {
                                                "type": "think",
                                                "think": thinking,
                                                "encrypted": null
                                            }
                                        }
                                    });
                                    writeln!(wire_file, "{}", part)?;
                                    assistant_content.push(serde_json::json!({
                                        "type": "think",
                                        "think": thinking,
                                        "encrypted": null
                                    }));
                                }
                                _ => {}
                            }
                        }

                        let status = serde_json::json!({
                            "timestamp": assistant_ts,
                            "message": {
                                "type": "StatusUpdate",
                                "payload": {}
                            }
                        });
                        writeln!(wire_file, "{}", status)?;

                        let turn_end = serde_json::json!({
                            "timestamp": assistant_ts,
                            "message": {
                                "type": "TurnEnd",
                                "payload": {}
                            }
                        });
                        writeln!(wire_file, "{}", turn_end)?;

                        let ctx_line = serde_json::json!({
                            "role": "assistant",
                            "content": assistant_content
                        });
                        writeln!(context_file, "{}", ctx_line)?;

                        i += 1;
                    } else {
                        // No assistant reply: end turn immediately
                        let turn_end = serde_json::json!({
                            "timestamp": ts,
                            "message": {
                                "type": "TurnEnd",
                                "payload": {}
                            }
                        });
                        writeln!(wire_file, "{}", turn_end)?;
                    }
                }
                MemorphRole::Assistant => {
                    // Assistant message without preceding user message
                    let text = msg
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let ctx_line = serde_json::json!({"role": "assistant", "content": text});
                    writeln!(context_file, "{}", ctx_line)?;
                    i += 1;
                }
            }
        }

        // state.json
        let state = serde_json::json!({
            "version": 1,
            "approval": {
                "yolo": false,
                "auto_approve_actions": []
            },
            "additional_dirs": [],
            "custom_title": title.chars().take(TITLE_MAX_CHARS).collect::<String>(),
            "title_generated": false,
            "title_generate_attempts": 0,
            "plan_mode": false,
            "plan_session_id": null,
            "plan_slug": null,
            "wire_mtime": null,
            "archived": false,
            "archived_at": null,
            "auto_archive_exempt": false,
            "todos": []
        });
        let mut state_file = File::create(&state_path)?;
        write!(state_file, "{}", serde_json::to_string_pretty(&state)?)?;

        Ok(session_id)
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        if let Some(dir) = find_session_dir(session_id) {
            std::fs::remove_dir_all(&dir)
                .with_context(|| format!("Failed to delete Kimi session dir: {}", dir.display()))?;
        }
        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let dir = find_session_dir(session_id)
            .with_context(|| format!("Kimi session not found: {}", session_id))?;
        let state_path = dir.join("state.json");
        if !state_path.exists() {
            anyhow::bail!("Kimi state.json not found for session: {}", session_id);
        }

        let raw = std::fs::read_to_string(&state_path)?;
        let mut state: Value = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse state.json: {}", state_path.display()))?;

        if let Some(obj) = state.as_object_mut() {
            obj.insert(
                "custom_title".to_string(),
                Value::String(new_title.to_string()),
            );
        }

        let mut file = File::create(&state_path)?;
        write!(file, "{}", serde_json::to_string_pretty(&state)?)?;
        Ok(())
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("kimi resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let dir = find_session_dir(session_id)
            .with_context(|| format!("Kimi session not found: {}", session_id))?;
        let mut total: u64 = 0;
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
        Ok(total)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_kimi_sessions_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".kimi").join("sessions"))
        .unwrap_or_else(|| PathBuf::from(".kimi").join("sessions"))
}

fn get_kimi_json_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".kimi").join("kimi.json"))
        .unwrap_or_else(|| PathBuf::from(".kimi").join("kimi.json"))
}

fn md5_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let hash = md5::compute(data);
    let mut hex = String::with_capacity(32);
    for byte in hash.as_ref() {
        write!(&mut hex, "{:02x}", byte).unwrap();
    }
    hex
}

#[derive(Debug, serde::Deserialize)]
struct KimiState {
    #[serde(default)]
    custom_title: Option<String>,
    #[serde(default)]
    archived: bool,
}

fn read_state_json(path: &Path) -> Result<KimiState> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read state.json: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse state.json: {}", path.display()))
}

fn load_work_dir_map() -> Result<HashMap<String, String>> {
    let path = get_kimi_json_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read kimi.json: {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse kimi.json: {}", path.display()))?;

    let mut map = HashMap::new();
    if let Some(dirs) = value.get("work_dirs").and_then(|v| v.as_array()) {
        for entry in dirs {
            if let Some(path_str) = entry.get("path").and_then(|v| v.as_str()) {
                let hash = md5_hex(path_str.as_bytes());
                map.insert(hash, path_str.to_string());
            }
        }
    }
    Ok(map)
}

fn find_session_dir(session_id: &str) -> Option<PathBuf> {
    let root = get_kimi_sessions_dir();
    for entry in WalkDir::new(&root)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == session_id {
                    return Some(path.to_path_buf());
                }
            }
        }
    }
    None
}

fn wire_last_timestamp(wire_path: &Path) -> Option<i64> {
    let file = File::open(wire_path).ok()?;
    let reader = BufReader::new(file);
    let mut last_ts: Option<f64> = None;
    for line in reader.lines() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line).ok()?;
        if let Some(ts) = value.get("timestamp").and_then(|v| v.as_f64()) {
            last_ts = Some(ts);
        }
    }
    last_ts.map(|ts| ts as i64)
}

fn load_from_wire(
    wire_path: &Path,
) -> Result<(
    Vec<MemorphMessage>,
    Option<chrono::DateTime<Utc>>,
    Option<chrono::DateTime<Utc>>,
)> {
    let file = File::open(wire_path)
        .with_context(|| format!("Failed to open Kimi wire.jsonl: {}", wire_path.display()))?;
    let reader = BufReader::new(file);

    let mut messages: Vec<MemorphMessage> = Vec::new();
    let mut current_user_msg: Option<MemorphMessage> = None;
    let mut current_assistant_blocks: Vec<ContentBlock> = Vec::new();
    let mut current_assistant_ts = Utc::now();
    let mut first_ts: Option<chrono::DateTime<Utc>> = None;
    let mut last_ts: Option<chrono::DateTime<Utc>> = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = value
            .get("message")
            .and_then(|m| m.get("type"))
            .and_then(|v| v.as_str());

        let ts = parse_wire_timestamp(&value).unwrap_or_else(Utc::now);
        if first_ts.is_none() {
            first_ts = Some(ts);
        }
        last_ts = Some(ts);

        match msg_type {
            Some("TurnBegin") => {
                // Flush previous assistant
                if !current_assistant_blocks.is_empty() {
                    messages.push(MemorphMessage {
                        id: Uuid::new_v4().to_string(),
                        role: MemorphRole::Assistant,
                        content: current_assistant_blocks.drain(..).collect(),
                        timestamp: current_assistant_ts,
                        metadata: None,
                        parent_id: None,
                        turn_index: None,
                    });
                }

                let payload = value.get("message").and_then(|m| m.get("payload"));
                let content = parse_user_input(payload);
                if !content.is_empty() {
                    current_user_msg = Some(MemorphMessage {
                        id: Uuid::new_v4().to_string(),
                        role: MemorphRole::User,
                        content,
                        timestamp: ts,
                        metadata: None,
                        parent_id: None,
                        turn_index: None,
                    });
                }
            }
            Some("ContentPart") => {
                if let Some(user_msg) = current_user_msg.take() {
                    messages.push(user_msg);
                }

                let payload = value.get("message").and_then(|m| m.get("payload"));
                if let Some(block) = parse_content_part(payload) {
                    current_assistant_blocks.push(block);
                    current_assistant_ts = ts;
                }
            }
            Some("TurnEnd") => {
                if let Some(user_msg) = current_user_msg.take() {
                    messages.push(user_msg);
                }
                if !current_assistant_blocks.is_empty() {
                    messages.push(MemorphMessage {
                        id: Uuid::new_v4().to_string(),
                        role: MemorphRole::Assistant,
                        content: current_assistant_blocks.drain(..).collect(),
                        timestamp: current_assistant_ts,
                        metadata: None,
                        parent_id: None,
                        turn_index: None,
                    });
                }
            }
            _ => {}
        }
    }

    // Flush any remaining messages
    if let Some(user_msg) = current_user_msg.take() {
        messages.push(user_msg);
    }
    if !current_assistant_blocks.is_empty() {
        messages.push(MemorphMessage {
            id: Uuid::new_v4().to_string(),
            role: MemorphRole::Assistant,
            content: current_assistant_blocks.drain(..).collect(),
            timestamp: current_assistant_ts,
            metadata: None,
            parent_id: None,
            turn_index: None,
        });
    }

    Ok((messages, first_ts, last_ts))
}

fn parse_wire_timestamp(value: &Value) -> Option<chrono::DateTime<Utc>> {
    let ts = value.get("timestamp").and_then(|v| v.as_f64())?;
    let secs = ts as i64;
    let nanos = ((ts - secs as f64) * 1e9).max(0.0) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
}

fn parse_user_input(payload: Option<&Value>) -> Vec<ContentBlock> {
    let payload = match payload {
        Some(p) => p,
        None => return Vec::new(),
    };
    let inputs = match payload.get("user_input").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    inputs
        .iter()
        .filter_map(|item| {
            let typ = item.get("type").and_then(|v| v.as_str())?;
            match typ {
                "text" => item
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| ContentBlock::text(s)),
                "image_url" => {
                    let url = item
                        .get("image_url")
                        .and_then(|v| v.get("url"))
                        .and_then(|v| v.as_str())?;
                    Some(ContentBlock::Image {
                        mime_type: "image/png".to_string(),
                        data: url.to_string(),
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn parse_content_part(payload: Option<&Value>) -> Option<ContentBlock> {
    let payload = payload?;
    let typ = payload.get("type").and_then(|v| v.as_str())?;
    match typ {
        "text" => payload
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| ContentBlock::text(s)),
        "think" => {
            let thinking = payload.get("think").and_then(|v| v.as_str())?.to_string();
            Some(ContentBlock::Thinking {
                thinking,
                signature: None,
            })
        }
        _ => None,
    }
}
