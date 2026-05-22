use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::provider::{
    canonical_block_text, canonical_event_text, canonical_export_result, canonical_session_title,
    Provider, ProviderCapabilities, ProviderSessionSummary,
};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
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

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
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

            sessions.push(ProviderSessionSummary {
                session_id,
                title: title.filter(|t| !t.is_empty()),
                project_dir,
                last_active_at,
                source_path: Some(wire_path.to_string_lossy().to_string()),
            });
        }

        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        import_canonical_session_from_wire(Path::new(source_path))
    }

    fn export_session(
        &self,
        session: &CanonicalSession,
        target_dir: &Path,
    ) -> Result<ExportedSession> {
        let session_id = export_canonical_session(session, target_dir)?;
        Ok(canonical_export_result(
            PROVIDER_ID,
            session_id.clone(),
            self.resume_command(&session_id),
        ))
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

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
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

    writeln!(
        wire_file,
        "{}",
        serde_json::json!({"type": "metadata", "protocol_version": "1.9"})
    )?;

    for event in &session.events {
        let ts = event.timestamp.timestamp_millis() as f64 / 1000.0;
        match event.role {
            EventRole::Assistant => {
                for block in &event.blocks {
                    if let Some(payload) = canonical_block_to_kimi_content_part(block) {
                        writeln!(
                            wire_file,
                            "{}",
                            serde_json::json!({
                                "timestamp": ts,
                                "message": {
                                    "type": "ContentPart",
                                    "payload": payload
                                }
                            })
                        )?;
                    }
                }
                writeln!(
                    context_file,
                    "{}",
                    serde_json::json!({
                        "role": "assistant",
                        "content": canonical_event_text(event)
                    })
                )?;
            }
            _ => {
                let text = canonical_event_text(event);
                if text.trim().is_empty() {
                    continue;
                }
                writeln!(
                    wire_file,
                    "{}",
                    serde_json::json!({
                        "timestamp": ts,
                        "message": {
                            "type": "TurnBegin",
                            "payload": {
                                "user_input": [{"type": "text", "text": text}]
                            }
                        }
                    })
                )?;
                writeln!(
                    wire_file,
                    "{}",
                    serde_json::json!({
                        "timestamp": ts,
                        "message": {
                            "type": "StepBegin",
                            "payload": {"n": 1}
                        }
                    })
                )?;
                writeln!(
                    context_file,
                    "{}",
                    serde_json::json!({"role": "user", "content": text})
                )?;
            }
        }
    }

    let end_ts = session
        .context
        .last_active_at
        .or_else(|| session.events.last().map(|event| event.timestamp))
        .unwrap_or_else(Utc::now)
        .timestamp_millis() as f64
        / 1000.0;
    writeln!(
        wire_file,
        "{}",
        serde_json::json!({
            "timestamp": end_ts,
            "message": {
                "type": "StatusUpdate",
                "payload": {}
            }
        })
    )?;
    writeln!(
        wire_file,
        "{}",
        serde_json::json!({
            "timestamp": end_ts,
            "message": {
                "type": "TurnEnd",
                "payload": {}
            }
        })
    )?;

    let title = canonical_session_title(session)
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect::<String>();
    let state = serde_json::json!({
        "version": 1,
        "approval": {
            "yolo": false,
            "auto_approve_actions": []
        },
        "additional_dirs": [],
        "custom_title": title,
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

fn canonical_block_to_kimi_content_part(block: &EventBlock) -> Option<Value> {
    match block {
        EventBlock::Text { text } => Some(serde_json::json!({
            "type": "text",
            "text": text
        })),
        EventBlock::Thinking { text, .. } => Some(serde_json::json!({
            "type": "think",
            "think": text,
            "encrypted": null
        })),
        _ => {
            let text = canonical_block_text(block);
            (!text.trim().is_empty()).then(|| {
                serde_json::json!({
                    "type": "text",
                    "text": text
                })
            })
        }
    }
}

fn import_canonical_session_from_wire(wire_path: &Path) -> Result<ImportedSession> {
    let session_dir = wire_path
        .parent()
        .with_context(|| format!("Invalid Kimi session path: {}", wire_path.display()))?;
    let state_path = session_dir.join("state.json");
    let state_value = if state_path.exists() {
        std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    } else {
        None
    };
    let title = state_value
        .as_ref()
        .and_then(|state| state.get("custom_title"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    let session_id = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let project_dir = kimi_project_dir_for_session_dir(session_dir);

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let (events, created_at, last_active_at) = canonical_events_from_wire(wire_path, &mut report)?;
    let mut extensions = BTreeMap::new();
    if let Some(state) = state_value {
        extensions.insert("kimi_state".to_string(), state);
    }

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
                    source_path: Some(wire_path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: project_dir,
                created_at,
                last_active_at,
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn canonical_events_from_wire(
    wire_path: &Path,
    report: &mut MappingReport,
) -> Result<(
    Vec<SessionEvent>,
    Option<chrono::DateTime<Utc>>,
    Option<chrono::DateTime<Utc>>,
)> {
    let file = File::open(wire_path)
        .with_context(|| format!("Failed to open Kimi wire.jsonl: {}", wire_path.display()))?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    let mut pending_user: Option<SessionEvent> = None;
    let mut assistant_blocks: Vec<EventBlock> = Vec::new();
    let mut assistant_raw_parts: Vec<Value> = Vec::new();
    let mut assistant_ts = Utc::now();
    let mut turn_index = 0u32;
    let mut first_ts: Option<chrono::DateTime<Utc>> = None;
    let mut last_ts: Option<chrono::DateTime<Utc>> = None;

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Kimi wire line: {}", error),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };

        let ts = parse_wire_timestamp(&value).unwrap_or_else(Utc::now);
        first_ts = first_ts.or(Some(ts));
        last_ts = Some(ts);
        let msg_type = value
            .get("message")
            .and_then(|message| message.get("type"))
            .and_then(|kind| kind.as_str())
            .unwrap_or("unknown");

        match msg_type {
            "TurnBegin" => {
                flush_kimi_pending_user(&mut events, &mut pending_user);
                flush_kimi_assistant(
                    &mut events,
                    &mut assistant_blocks,
                    &mut assistant_raw_parts,
                    assistant_ts,
                    turn_index,
                );
                let payload = value
                    .get("message")
                    .and_then(|message| message.get("payload"));
                let blocks = kimi_user_input_event_blocks(payload, &value, line_idx + 1, report);
                if !blocks.is_empty() {
                    pending_user = Some(kimi_event(
                        format!("kimi:user:{}:{}", turn_index, line_idx + 1),
                        SessionEventKind::Message,
                        EventRole::User,
                        ts,
                        turn_index,
                        blocks,
                        vec![value.clone()],
                    ));
                }
            }
            "ContentPart" => {
                flush_kimi_pending_user(&mut events, &mut pending_user);
                let payload = value
                    .get("message")
                    .and_then(|message| message.get("payload"));
                if let Some(block) =
                    kimi_content_part_event_block(payload, &value, line_idx + 1, report)
                {
                    assistant_blocks.push(block);
                    assistant_raw_parts.push(value.clone());
                    assistant_ts = ts;
                }
            }
            "TurnEnd" => {
                flush_kimi_pending_user(&mut events, &mut pending_user);
                flush_kimi_assistant(
                    &mut events,
                    &mut assistant_blocks,
                    &mut assistant_raw_parts,
                    assistant_ts,
                    turn_index,
                );
                turn_index += 1;
            }
            other => {
                events.push(kimi_event(
                    format!("kimi:{}:{}", other, line_idx + 1),
                    SessionEventKind::Lifecycle,
                    EventRole::System,
                    ts,
                    turn_index,
                    vec![EventBlock::ProviderPayload {
                        kind: other.to_string(),
                        payload: value.clone(),
                    }],
                    vec![value],
                ));
            }
        }
    }

    flush_kimi_pending_user(&mut events, &mut pending_user);
    flush_kimi_assistant(
        &mut events,
        &mut assistant_blocks,
        &mut assistant_raw_parts,
        assistant_ts,
        turn_index,
    );

    Ok((events, first_ts, last_ts))
}

fn flush_kimi_pending_user(
    events: &mut Vec<SessionEvent>,
    pending_user: &mut Option<SessionEvent>,
) {
    if let Some(event) = pending_user.take() {
        events.push(event);
    }
}

fn flush_kimi_assistant(
    events: &mut Vec<SessionEvent>,
    assistant_blocks: &mut Vec<EventBlock>,
    assistant_raw_parts: &mut Vec<Value>,
    timestamp: chrono::DateTime<Utc>,
    turn_index: u32,
) {
    if assistant_blocks.is_empty() {
        assistant_raw_parts.clear();
        return;
    }
    let blocks = std::mem::take(assistant_blocks);
    let raw_parts = std::mem::take(assistant_raw_parts);
    events.push(kimi_event(
        format!("kimi:assistant:{}", events.len()),
        kimi_event_kind(&blocks),
        EventRole::Assistant,
        timestamp,
        turn_index,
        blocks,
        raw_parts,
    ));
}

fn kimi_event(
    id: String,
    kind: SessionEventKind,
    role: EventRole,
    timestamp: chrono::DateTime<Utc>,
    turn_index: u32,
    blocks: Vec<EventBlock>,
    raw_parts: Vec<Value>,
) -> SessionEvent {
    SessionEvent {
        id,
        kind,
        role,
        timestamp,
        links: EventLinks {
            parent_event_id: None,
            provider_parent_id: None,
            turn_index: Some(turn_index),
            related_event_ids: Vec::new(),
        },
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: Some(
                    match role {
                        EventRole::User => "user",
                        EventRole::Assistant => "assistant",
                        EventRole::Tool => "tool",
                        EventRole::System => "system",
                        EventRole::Developer => "developer",
                        EventRole::Unknown => "unknown",
                    }
                    .to_string(),
                ),
                phase: None,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("kimi_wire_lines".to_string(), Value::Array(raw_parts));
                ext
            },
        },
    }
}

fn kimi_user_input_event_blocks(
    payload: Option<&Value>,
    raw_line: &Value,
    line_number: usize,
    report: &mut MappingReport,
) -> Vec<EventBlock> {
    let Some(inputs) = payload
        .and_then(|payload| payload.get("user_input"))
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };

    inputs
        .iter()
        .enumerate()
        .map(
            |(idx, item)| match item.get("type").and_then(|value| value.as_str()) {
                Some("text") => EventBlock::Text {
                    text: item
                        .get("text")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .to_string(),
                },
                Some("image_url") => EventBlock::Image {
                    mime_type: "image/png".to_string(),
                    data: item
                        .get("image_url")
                        .and_then(|value| value.get("url"))
                        .and_then(|value| value.as_str())
                        .map(str::to_string),
                    path: None,
                },
                Some(kind) => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: MappingDisposition::Preserved,
                        code: "provider_block_preserved".to_string(),
                        message: format!("Preserved unsupported Kimi user input '{}'", kind),
                        path: Some(format!("line:{}:input:{}", line_number, idx)),
                        raw: Some(raw_line.clone()),
                    });
                    EventBlock::ProviderPayload {
                        kind: kind.to_string(),
                        payload: item.clone(),
                    }
                }
                None => EventBlock::Unknown { raw: item.clone() },
            },
        )
        .collect()
}

fn kimi_content_part_event_block(
    payload: Option<&Value>,
    raw_line: &Value,
    line_number: usize,
    report: &mut MappingReport,
) -> Option<EventBlock> {
    let payload = payload?;
    match payload.get("type").and_then(|value| value.as_str()) {
        Some("text") => Some(EventBlock::Text {
            text: payload
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        Some("think") => Some(EventBlock::Thinking {
            text: payload
                .get("think")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            signature: None,
        }),
        Some(kind) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Preserved,
                code: "provider_block_preserved".to_string(),
                message: format!("Preserved unsupported Kimi content part '{}'", kind),
                path: Some(format!("line:{}", line_number)),
                raw: Some(raw_line.clone()),
            });
            Some(EventBlock::ProviderPayload {
                kind: kind.to_string(),
                payload: payload.clone(),
            })
        }
        None => Some(EventBlock::Unknown {
            raw: payload.clone(),
        }),
    }
}

fn kimi_event_kind(blocks: &[EventBlock]) -> SessionEventKind {
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

fn kimi_project_dir_for_session_dir(session_dir: &Path) -> Option<String> {
    let project_hash = session_dir
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())?;
    load_work_dir_map().ok()?.get(project_hash).cloned()
}

fn parse_wire_timestamp(value: &Value) -> Option<chrono::DateTime<Utc>> {
    let ts = value.get("timestamp").and_then(|v| v.as_f64())?;
    let secs = ts as i64;
    let nanos = ((ts - secs as f64) * 1e9).max(0.0) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_canonical_session_preserves_kimi_wire_events_and_state() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let session_dir = temp.path().join("project-hash").join("kimi-session-1");
        std::fs::create_dir_all(&session_dir)?;
        let wire_path = session_dir.join("wire.jsonl");
        let state_path = session_dir.join("state.json");

        std::fs::write(
            &state_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "custom_title": "Kimi Title",
                "archived": false,
                "todos": [{"content": "keep raw state"}]
            }))?,
        )?;
        let mut wire_file = File::create(&wire_path)?;
        writeln!(
            wire_file,
            "{}",
            serde_json::json!({
                "timestamp": 1710000000.0,
                "message": {
                    "type": "metadata",
                    "payload": {"protocol_version": "1.9"}
                }
            })
        )?;
        writeln!(
            wire_file,
            "{}",
            serde_json::json!({
                "timestamp": 1710000001.0,
                "message": {
                    "type": "TurnBegin",
                    "payload": {
                        "user_input": [
                            {"type": "text", "text": "hello"},
                            {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
                        ]
                    }
                }
            })
        )?;
        writeln!(
            wire_file,
            "{}",
            serde_json::json!({
                "timestamp": 1710000002.0,
                "message": {
                    "type": "ContentPart",
                    "payload": {"type": "think", "think": "reasoning"}
                }
            })
        )?;
        writeln!(
            wire_file,
            "{}",
            serde_json::json!({
                "timestamp": 1710000003.0,
                "message": {
                    "type": "ContentPart",
                    "payload": {"type": "text", "text": "answer"}
                }
            })
        )?;
        writeln!(
            wire_file,
            "{}",
            serde_json::json!({
                "timestamp": 1710000004.0,
                "message": {
                    "type": "ContentPart",
                    "payload": {"type": "custom", "payload": {"kept": true}}
                }
            })
        )?;
        writeln!(
            wire_file,
            "{}",
            serde_json::json!({
                "timestamp": 1710000005.0,
                "message": {
                    "type": "TurnEnd",
                    "payload": {}
                }
            })
        )?;

        let imported = import_canonical_session_from_wire(&wire_path)?;

        assert_eq!(imported.session.identity.canonical_id, "kimi-session-1");
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Kimi Title")
        );
        assert!(imported.session.extensions.contains_key("kimi_state"));
        assert!(imported.session.events.iter().any(|event| {
            event.kind == SessionEventKind::Lifecycle
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ProviderPayload { kind, .. }) if kind == "metadata"
                )
        }));
        let user = imported
            .session
            .events
            .iter()
            .find(|event| event.role == EventRole::User)
            .unwrap();
        assert!(matches!(
            user.blocks.first(),
            Some(EventBlock::Text { text }) if text == "hello"
        ));
        assert!(user
            .blocks
            .iter()
            .any(|block| matches!(block, EventBlock::Image { data: Some(data), .. } if data == "data:image/png;base64,abc")));
        let assistant = imported
            .session
            .events
            .iter()
            .find(|event| event.role == EventRole::Assistant)
            .unwrap();
        assert!(assistant.blocks.iter().any(
            |block| matches!(block, EventBlock::Thinking { text, .. } if text == "reasoning")
        ));
        assert!(assistant
            .blocks
            .iter()
            .any(|block| matches!(block, EventBlock::Text { text } if text == "answer")));
        assert!(assistant.blocks.iter().any(
            |block| matches!(block, EventBlock::ProviderPayload { kind, .. } if kind == "custom")
        ));

        Ok(())
    }
}
