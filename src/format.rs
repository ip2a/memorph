use crate::model::{MemorphMeta, MemorphSession, MemorphMessage, SessionInfo};
use anyhow::{Context, Result};
// use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Read session from a .morph file
pub fn read_session(path: &Path) -> Result<MemorphSession> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open morph file: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut meta: Option<MemorphMeta> = None;
    let mut session_info: Option<SessionInfo> = None;
    let mut messages: Vec<MemorphMessage> = Vec::new();

    for (idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("Failed to read line {} from {}", idx + 1, path.display()))?;
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(&line)
            .with_context(|| format!("Failed to parse JSON at line {} in {}", idx + 1, path.display()))?;

        let line_type = value.get("type").and_then(|v| v.as_str());

        match line_type {
            Some("meta") => {
                let m: MemorphMeta = serde_json::from_value(
                    value.get("_memorph").cloned().unwrap_or(Value::Null)
                )?;
                let s: SessionInfo = serde_json::from_value(
                    value.get("session").cloned().unwrap_or(Value::Null)
                )?;
                meta = Some(m);
                session_info = Some(s);
            }
            Some("message") => {
                let msg = parse_message_line(value)
                    .with_context(|| format!("Failed to parse message at line {} in {}", idx + 1, path.display()))?;
                messages.push(msg);
            }
            _ => {}
        }
    }

    let meta = meta.context("Missing meta line in morph file")?;
    let session = session_info.context("Missing session info in morph file")?;

    Ok(MemorphSession {
        meta,
        session,
        messages,
    })
}

/// Write session to a .morph file
pub fn write_session(path: &Path, session: &MemorphSession) -> Result<()> {
    let mut file = File::create(path)
        .with_context(|| format!("Failed to create morph file: {}", path.display()))?;

    // Write meta line
    let meta_value = serde_json::json!({
        "type": "meta",
        "_memorph": session.meta,
        "session": session.session,
    });
    writeln!(file, "{}", serde_json::to_string(&meta_value)?)?;

    // Write message lines
    for msg in &session.messages {
        let msg_value = serde_json::json!({
            "type": "message",
            "id": msg.id,
            "role": msg.role.to_string(),
            "content": {
                "blocks": msg.content
            },
            "timestamp": msg.timestamp.to_rfc3339(),
            "metadata": msg.metadata,
            "parent_id": msg.parent_id,
            "turn_index": msg.turn_index,
        });
        writeln!(file, "{}", serde_json::to_string(&msg_value)?)?;
    }

    Ok(())
}

/// Manually parse a message line, handling the content.blocks wrapper structure
fn parse_message_line(value: Value) -> anyhow::Result<MemorphMessage> {
    let id = value.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let role_str = value.get("role").and_then(|v| v.as_str()).unwrap_or("user");
    let role = match role_str {
        "user" => crate::model::MemorphRole::User,
        "assistant" => crate::model::MemorphRole::Assistant,
        "tool" => crate::model::MemorphRole::Tool,
        "system" => crate::model::MemorphRole::System,
        "developer" => crate::model::MemorphRole::Developer,
        _ => crate::model::MemorphRole::User,
    };

    let content = value
        .get("content")
        .and_then(|c| c.get("blocks"))
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let timestamp = value
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let metadata = value.get("metadata").cloned().map(|v| {
        serde_json::from_value(v).unwrap_or(crate::model::MessageMetadata {
            source: None,
            model: None,
            usage: None,
            extra: serde_json::Value::Null,
        })
    });

    let parent_id = value.get("parent_id").and_then(|v| v.as_str()).map(|s| s.to_string());
    let turn_index = value.get("turn_index").and_then(|v| v.as_u64()).map(|v| v as u32);

    Ok(MemorphMessage {
        id,
        role,
        content,
        timestamp,
        metadata,
        parent_id,
        turn_index,
    })
}

