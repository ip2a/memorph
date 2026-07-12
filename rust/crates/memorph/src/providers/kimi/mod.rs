pub mod adapter;
pub mod hook;

use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::provider::{
    canonical_event_visible_message_role, canonical_event_visible_message_text,
    canonical_export_result, canonical_session_title, canonical_visible_block_text, Provider,
    ProviderBackupSupport, ProviderCapabilities, ProviderSessionBackup, ProviderSessionSummary,
    ProviderSourceMutation,
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

mod backup;

pub struct KimiProvider;

const PROVIDER_ID: &str = "kimi";
const TITLE_MAX_CHARS: usize = 80;

#[cfg(test)]
static TEST_KIMI_SESSIONS_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_KIMI_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

impl Provider for KimiProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Kimi"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
            },
            ..ProviderCapabilities::full_session_management()
        }
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

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        let dir = match find_session_dir(session_id) {
            Some(d) => d,
            None => return Ok(None),
        };

        let state_path = dir.join("state.json");
        let (title, archived) = if state_path.exists() {
            match read_state_json(&state_path) {
                Ok(s) => (s.custom_title, s.archived),
                Err(_) => (None, false),
            }
        } else {
            (None, false)
        };

        if archived {
            return Ok(None);
        }

        let project_hash = dir
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let dir_map = load_work_dir_map().unwrap_or_default();
        let project_dir = dir_map.get(&project_hash).cloned();

        let wire_path = dir.join("wire.jsonl");
        let last_active_at = wire_last_timestamp(&wire_path);

        Ok(Some(ProviderSessionSummary {
            session_id: session_id.to_string(),
            title: title.filter(|t| !t.is_empty()),
            project_dir,
            last_active_at,
            source_path: Some(wire_path.to_string_lossy().to_string()),
        }))
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
            session,
            self.capabilities(),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let dir = backup::validate_mutation_source(ProviderSourceMutation::Delete, session_id)?;
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("Failed to delete Kimi session dir: {}", dir.display()))?;
        fail_kimi_mutation_after_write(ProviderSourceMutation::Delete)?;
        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let dir = backup::validate_mutation_source(ProviderSourceMutation::Rename, session_id)?;
        let state_path = dir.join("state.json");

        let raw = std::fs::read_to_string(&state_path)?;
        let mut state: Value = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse state.json: {}", state_path.display()))?;

        if let Some(obj) = state.as_object_mut() {
            obj.insert(
                "custom_title".to_string(),
                Value::String(new_title.to_string()),
            );
        }

        let updated = serde_json::to_vec_pretty(&state)?;
        write_kimi_state_atomically(&state_path, &updated)?;
        fail_kimi_mutation_after_write(ProviderSourceMutation::Rename)?;
        Ok(())
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        backup::create_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        backup::restore_session_backup(backup)
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

    fn data_source_paths(&self) -> Vec<PathBuf> {
        vec![get_kimi_sessions_dir()]
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_kimi_sessions_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_KIMI_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi sessions dir lock")
        .clone()
    {
        return path;
    }

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

fn write_kimi_state_atomically(state_path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = state_path
        .parent()
        .context("Kimi state.json has no parent directory")?;
    let temporary_path = parent.join(format!(".state.json.memorph-{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<()> {
        let mut file = File::create(&temporary_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary_path, state_path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    write_result
}

#[cfg(test)]
fn set_test_kimi_sessions_dir(path: Option<PathBuf>) {
    *TEST_KIMI_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi sessions dir lock") = path;
}

#[cfg(test)]
fn set_test_kimi_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_KIMI_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi mutation failure lock") = mutation;
}

#[cfg(test)]
fn fail_kimi_mutation_after_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_KIMI_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected Kimi mutation failure after provider write");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_kimi_mutation_after_write(_mutation: ProviderSourceMutation) -> Result<()> {
    Ok(())
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
        let Some(visible_role) = canonical_event_visible_message_role(event) else {
            continue;
        };
        let ts = event.timestamp.timestamp_millis() as f64 / 1000.0;
        match visible_role {
            EventRole::Assistant => {
                let Some(text) = canonical_event_visible_message_text(event) else {
                    continue;
                };
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
                        "content": text
                    })
                )?;
            }
            _ => {
                let Some(text) = canonical_event_visible_message_text(event) else {
                    continue;
                };
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
        _ => canonical_visible_block_text(block).map(|text| {
            serde_json::json!({
                "type": "text",
                "text": text
            })
        }),
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
    use crate::{core::session_management, storage::local_store};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    static TEST_KIMI_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    struct TestKimiSessionsGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestKimiSessionsGuard {
        fn drop(&mut self) {
            crate::cache::global_cache().invalidate(PROVIDER_ID);
            backup::set_test_backup_failure(false);
            set_test_kimi_mutation_failure(None);
            set_test_kimi_sessions_dir(None);
        }
    }

    fn use_test_kimi_sessions_dir(path: PathBuf) -> TestKimiSessionsGuard {
        let lock = TEST_KIMI_TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set_test_kimi_sessions_dir(Some(path));
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        TestKimiSessionsGuard { _lock: lock }
    }

    fn write_native_kimi_fixture(root: &Path, project: &str, session_id: &str) -> PathBuf {
        let session_dir = root.join(project).join(session_id);
        std::fs::create_dir_all(session_dir.join("nested")).unwrap();
        std::fs::write(
            session_dir.join("state.json"),
            b"{\n  \"version\": 1,\n  \"custom_title\": \"Before\",\n  \"archived\": false,\n  \"native\": {\"keep\": true}\n}\n",
        )
        .unwrap();
        std::fs::write(
            session_dir.join("wire.jsonl"),
            b"{\"timestamp\":1710000000.0,\"message\":{\"type\":\"metadata\"}}\n",
        )
        .unwrap();
        std::fs::write(
            session_dir.join("context.jsonl"),
            b"{\"role\":\"user\",\"content\":\"hello\"}\n",
        )
        .unwrap();
        std::fs::write(
            session_dir.join("nested").join("native.bin"),
            [0_u8, 1, 127, 128, 255],
        )
        .unwrap();
        session_dir
    }

    fn session_tree_bytes(session_dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        WalkDir::new(session_dir)
            .min_depth(1)
            .into_iter()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                (
                    entry
                        .path()
                        .strip_prefix(session_dir)
                        .unwrap()
                        .to_path_buf(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn delete_backup_restores_exact_kimi_directory_and_preserves_unrelated_sessions() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-delete";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let unrelated_dir = write_native_kimi_fixture(&root, "project-b", "kimi-other");
        let original = session_tree_bytes(&session_dir);
        let backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-delete",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        KimiProvider.delete_session(session_id).unwrap();
        assert!(!session_dir.exists());
        std::fs::write(unrelated_dir.join("wire.jsonl"), b"changed concurrently\n").unwrap();

        KimiProvider.restore_session_backup(&backup).unwrap();
        KimiProvider.restore_session_backup(&backup).unwrap();

        assert_eq!(session_tree_bytes(&session_dir), original);
        assert_eq!(
            std::fs::read(unrelated_dir.join("wire.jsonl")).unwrap(),
            b"changed concurrently\n"
        );
    }

    #[test]
    fn rename_backup_restores_exact_state_only_and_preserves_other_changes() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-rename";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let original_state = std::fs::read(session_dir.join("state.json")).unwrap();
        let backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-kimi-rename",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        KimiProvider.rename_session(session_id, "After").unwrap();
        std::fs::write(
            session_dir.join("wire.jsonl"),
            b"wire changed concurrently\n",
        )
        .unwrap();
        std::fs::write(session_dir.join("concurrent.txt"), b"keep me").unwrap();

        KimiProvider.restore_session_backup(&backup).unwrap();
        KimiProvider.restore_session_backup(&backup).unwrap();

        assert_eq!(
            std::fs::read(session_dir.join("state.json")).unwrap(),
            original_state
        );
        assert_eq!(
            std::fs::read(session_dir.join("wire.jsonl")).unwrap(),
            b"wire changed concurrently\n"
        );
        assert_eq!(
            std::fs::read(session_dir.join("concurrent.txt")).unwrap(),
            b"keep me"
        );
    }

    #[test]
    fn rename_restore_does_not_recreate_concurrently_deleted_state() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-concurrent-delete";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-kimi-concurrent-delete",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        KimiProvider.rename_session(session_id, "After").unwrap();
        std::fs::remove_file(session_dir.join("state.json")).unwrap();

        KimiProvider.restore_session_backup(&backup).unwrap();

        assert!(!session_dir.join("state.json").exists());
    }

    #[test]
    fn kimi_backup_contract_and_capabilities_are_truthful() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-contract";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-contract",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        let capabilities = KimiProvider.capabilities();
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert_eq!(backup.source_path, session_dir.canonicalize().unwrap());
        assert_eq!(backup.format, "kimi-session-backup-v1");
        assert_eq!(
            backup.mime_type,
            "application/vnd.memorph.kimi-session-backup"
        );
        assert!(backup.backup_path.join("metadata.json").is_file());
        assert!(backup.backup_path.join("session/state.json").is_file());
        assert!(backup
            .backup_path
            .join("session/nested/native.bin")
            .is_file());
    }

    #[test]
    fn backup_registration_failure_prevents_kimi_provider_write() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-registration-failure";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let mut artifact_conn = rusqlite::Connection::open_in_memory().unwrap();

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-kimi-registration".to_string()],
            &dir.path().join("backups"),
            &mut artifact_conn,
        );

        assert!(results[0]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("Delete cancelled before provider write"));
        assert!(session_dir.exists());
        assert!(dir
            .path()
            .join("backups/kimi/operation-kimi-registration")
            .exists());
    }

    #[test]
    fn partial_kimi_delete_and_rename_failures_restore_registered_backups() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let delete_id = "kimi-partial-delete";
        let rename_id = "kimi-partial-rename";
        let delete_dir = write_native_kimi_fixture(&root, "project-a", delete_id);
        let rename_dir = write_native_kimi_fixture(&root, "project-a", rename_id);
        let delete_original = session_tree_bytes(&delete_dir);
        let rename_original = std::fs::read(rename_dir.join("state.json")).unwrap();
        let mut artifact_conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();

        set_test_kimi_mutation_failure(Some(ProviderSourceMutation::Delete));
        let delete_results = session_management::delete_sessions(
            PROVIDER_ID,
            &[delete_id],
            &["operation-kimi-partial-delete".to_string()],
            &dir.path().join("backups"),
            &mut artifact_conn,
        );
        assert!(delete_results[0]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(session_tree_bytes(&delete_dir), delete_original);

        set_test_kimi_mutation_failure(Some(ProviderSourceMutation::Rename));
        let rename_error = session_management::rename_session(
            PROVIDER_ID,
            rename_id,
            "After",
            "operation-kimi-partial-rename",
            &dir.path().join("backups"),
            &mut artifact_conn,
        )
        .unwrap_err();
        assert!(rename_error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            std::fs::read(rename_dir.join("state.json")).unwrap(),
            rename_original
        );
    }

    #[test]
    fn kimi_backup_rejects_ambiguous_and_unsafe_sources_before_mutation() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-ambiguous";
        let first = write_native_kimi_fixture(&root, "project-a", session_id);
        write_native_kimi_fixture(&root, "project-b", session_id);

        let error = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-ambiguous",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err();
        assert!(error.to_string().contains("not found or ambiguous"));
        assert!(first.exists());

        std::fs::remove_dir_all(root.join("project-b").join(session_id)).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(first.join("wire.jsonl"), first.join("unsafe-wire-link"))
                .unwrap();
            let error = KimiProvider
                .create_session_backup(
                    ProviderSourceMutation::Delete,
                    "operation-kimi-unsafe",
                    session_id,
                    &dir.path().join("backups"),
                )
                .unwrap_err();
            assert!(error.to_string().contains("unsupported filesystem entry"));
        }
    }

    #[test]
    fn kimi_restore_rejects_metadata_and_content_tampering() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-tamper";
        write_native_kimi_fixture(&root, "project-a", session_id);
        let backup_root = dir.path().join("backups");

        let content_backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-content-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        std::fs::write(
            content_backup.backup_path.join("session/state.json"),
            b"tampered",
        )
        .unwrap();
        assert!(KimiProvider
            .restore_session_backup(&content_backup)
            .unwrap_err()
            .to_string()
            .contains("does not match its manifest"));

        let metadata_backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-kimi-metadata-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        let metadata_path = metadata_backup.backup_path.join("metadata.json");
        let mut metadata: Value =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        metadata["provider_session_id"] = Value::String("other-session".to_string());
        std::fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        assert!(KimiProvider
            .restore_session_backup(&metadata_backup)
            .unwrap_err()
            .to_string()
            .contains("does not match the registered restore context"));
    }

    #[test]
    fn failed_kimi_backup_creation_removes_operation_directory() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-backup-failure";
        write_native_kimi_fixture(&root, "project-a", session_id);
        backup::set_test_backup_failure(true);

        let error = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-backup-failure",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err();

        assert!(error.to_string().contains("injected Kimi backup failure"));
        assert!(!dir
            .path()
            .join("backups/kimi/operation-kimi-backup-failure")
            .exists());
    }

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

    #[test]
    fn compressed_segment_exports_as_portable_kimi_text_part() {
        let block = EventBlock::Compressed {
            source_provider_id: "opencode".to_string(),
            summary: "compressed summary".to_string(),
            source_event_ids: vec![
                "old-event-1".to_string(),
                "old-event-2".to_string(),
                "old-event-3".to_string(),
            ],
            source_event_count: None,
            archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
        };

        let part = canonical_block_to_kimi_content_part(&block).expect("kimi text part");
        let text = part
            .get("text")
            .and_then(Value::as_str)
            .expect("portable compressed text");

        assert_eq!(part.get("type").and_then(Value::as_str), Some("text"));
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
    fn provider_payload_block_is_skipped_in_kimi_text_part_export() {
        let block = EventBlock::ProviderPayload {
            kind: "custom".to_string(),
            payload: serde_json::json!({"kept": true}),
        };

        assert!(canonical_block_to_kimi_content_part(&block).is_none());
    }
}
