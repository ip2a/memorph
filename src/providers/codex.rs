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
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct CodexProvider;

const PROVIDER_ID: &str = "codex";

impl Provider for CodexProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "codex"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_session_management()
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let index_path = get_codex_dir().join("session_index.jsonl");
        if !index_path.exists() {
            return Ok(Vec::new());
        }

        // Build a lookup from id -> cwd from SQLite for fast access
        let cwd_lookup = build_cwd_lookup().unwrap_or_default();

        let file = File::open(&index_path).with_context(|| {
            format!(
                "Failed to open Codex session index: {}",
                index_path.display()
            )
        })?;
        let reader = BufReader::new(file);
        let mut sessions = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let id = value
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let thread_name = value
                .get("thread_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let updated_at = value
                .get("updated_at")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis());

            if let Some(id) = id {
                // Find the actual file path for this session
                let source_path = find_session_file(&id);
                // Get cwd from SQLite lookup, or fall back to scanning file
                let project_dir = cwd_lookup
                    .get(&id)
                    .cloned()
                    .or_else(|| extract_cwd_from_session_file(&id));

                sessions.push(ProviderSessionSummary {
                    session_id: id.clone(),
                    title: thread_name,
                    project_dir,
                    last_active_at: updated_at,
                    source_path: source_path.map(|p| p.to_string_lossy().to_string()),
                });
            }
        }

        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        import_canonical_session(Path::new(source_path))
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
        // Find the session file
        let session_path = find_session_file(session_id)
            .with_context(|| format!("Codex session not found: {}", session_id))?;

        // Remove the JSONL file
        std::fs::remove_file(&session_path).with_context(|| {
            format!("Failed to remove session file: {}", session_path.display())
        })?;

        // Update session_index.jsonl to remove the entry
        let index_path = get_codex_dir().join("session_index.jsonl");
        if index_path.exists() {
            let content = std::fs::read_to_string(&index_path)?;
            let mut new_lines = Vec::new();
            for line in content.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    if v.get("id").and_then(|v| v.as_str()) == Some(session_id) {
                        continue;
                    }
                }
                new_lines.push(line.to_string());
            }
            std::fs::write(&index_path, new_lines.join("\n") + "\n")?;
        }

        // Remove from SQLite
        let db_path = get_codex_dir().join("state_5.sqlite");
        if db_path.exists() {
            let conn = Connection::open(&db_path)?;
            let _ = conn.execute("DELETE FROM threads WHERE id = ?1", [session_id]);
        }

        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        // Update session_index.jsonl
        let index_path = get_codex_dir().join("session_index.jsonl");
        if !index_path.exists() {
            anyhow::bail!("Codex session index not found");
        }

        let content = std::fs::read_to_string(&index_path)?;
        let mut new_lines = Vec::new();
        let mut found = false;

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut v: Value = serde_json::from_str(line)?;
            if v.get("id").and_then(|v| v.as_str()) == Some(session_id) {
                if let Value::Object(ref mut map) = v {
                    map.insert(
                        "thread_name".to_string(),
                        Value::String(new_title.to_string()),
                    );
                    found = true;
                }
                new_lines.push(serde_json::to_string(&v)?);
            } else {
                new_lines.push(line.to_string());
            }
        }

        if !found {
            anyhow::bail!("Codex session not found in index: {}", session_id);
        }

        std::fs::write(&index_path, new_lines.join("\n") + "\n")?;
        Ok(())
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("codex resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        if let Some(path) = find_session_file(session_id) {
            if path.exists() {
                return Ok(std::fs::metadata(path)?.len());
            }
        }
        Ok(0)
    }
}

fn import_canonical_session(path: &Path) -> Result<ImportedSession> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<chrono::DateTime<Utc>> = None;
    let mut last_active_at: Option<chrono::DateTime<Utc>> = None;
    let mut source_title: Option<String> = None;
    let mut extensions = BTreeMap::new();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Codex session line: {}", error),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let timestamp = value
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        last_active_at = Some(timestamp);

        match line_type.as_str() {
            "session_meta" => {
                if let Some(payload) = value.get("payload") {
                    session_id = payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(session_id);
                    project_dir = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(project_dir);
                    created_at = payload
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .or(created_at);

                    if let Some(text) = payload
                        .get("base_instructions")
                        .and_then(|v| v.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        events.push(SessionEvent {
                            id: format!("codex:base_instructions:{}", line_idx + 1),
                            kind: SessionEventKind::Lifecycle,
                            role: EventRole::System,
                            timestamp,
                            links: EventLinks::default(),
                            blocks: vec![
                                EventBlock::Text {
                                    text: text.to_string(),
                                },
                                EventBlock::ProviderPayload {
                                    kind: "session_meta".to_string(),
                                    payload: payload.clone(),
                                },
                            ],
                            metadata: EventMetadata {
                                source: EventSource {
                                    provider_id: PROVIDER_ID.to_string(),
                                    original_id: None,
                                    original_role: Some("developer".to_string()),
                                    phase: None,
                                },
                                model: payload
                                    .get("model")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string),
                                usage: None,
                                fidelity: MappingDisposition::Preserved,
                                provider_ext: {
                                    let mut ext = BTreeMap::new();
                                    ext.insert("codex_raw_line".to_string(), value.clone());
                                    ext
                                },
                            },
                        });
                    } else {
                        events.push(provider_payload_event(
                            format!("codex:session_meta:{}", line_idx + 1),
                            SessionEventKind::Lifecycle,
                            EventRole::System,
                            timestamp,
                            "session_meta",
                            payload.clone(),
                            value.clone(),
                            None,
                        ));
                    }

                    source_title = payload
                        .get("title")
                        .or_else(|| payload.get("thread_name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(source_title);
                    extensions.insert("codex_session_meta".to_string(), payload.clone());
                }
            }
            "turn_context" => {
                if let Some(payload) = value.get("payload") {
                    project_dir = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(project_dir);
                    events.push(provider_payload_event(
                        format!("codex:turn_context:{}", line_idx + 1),
                        SessionEventKind::Lifecycle,
                        EventRole::System,
                        timestamp,
                        "turn_context",
                        payload.clone(),
                        value.clone(),
                        None,
                    ));
                }
            }
            "event_msg" => {
                if let Some(payload) = value.get("payload") {
                    events.push(codex_event_msg_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                    ));
                }
            }
            "response_item" => {
                if let Some(payload) = value.get("payload") {
                    events.push(codex_response_item_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                        &mut report,
                    ));
                }
            }
            other => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Normalized,
                    code: "unknown_codex_line".to_string(),
                    message: format!("Preserved unknown Codex line type '{}'", other),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(value.clone()),
                });
                events.push(provider_payload_event(
                    format!("codex:unknown:{}", line_idx + 1),
                    SessionEventKind::Unknown,
                    EventRole::Unknown,
                    timestamp,
                    other,
                    value.get("payload").cloned().unwrap_or(Value::Null),
                    value,
                    None,
                ));
            }
        }
    }

    let canonical_id = session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: canonical_id.clone(),
                source_title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: canonical_id,
                    source_path: Some(path.to_string_lossy().to_string()),
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

fn codex_response_item_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
    report: &mut MappingReport,
) -> SessionEvent {
    let role_str = payload.get("role").and_then(|v| v.as_str());
    let msg_type = payload.get("type").and_then(|v| v.as_str());
    let phase = payload
        .get("phase")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let event_id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("codex:response_item:{}", line_no));

    if msg_type != Some("message") {
        return provider_payload_event(
            event_id,
            SessionEventKind::Unknown,
            EventRole::Unknown,
            timestamp,
            msg_type.unwrap_or("response_item"),
            payload.clone(),
            raw_line,
            phase,
        );
    }

    let mut blocks = Vec::new();
    if let Some(content_arr) = payload.get("content").and_then(|v| v.as_array()) {
        for (idx, block) in content_arr.iter().enumerate() {
            let Some(block_type) = block.get("type").and_then(|v| v.as_str()) else {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Normalized,
                    code: "codex_block_missing_type".to_string(),
                    message: "Codex content block without a type was preserved as unknown"
                        .to_string(),
                    path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                    raw: Some(block.clone()),
                });
                blocks.push(EventBlock::Unknown { raw: block.clone() });
                continue;
            };
            match block_type {
                "input_text" | "output_text" => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        blocks.push(EventBlock::Text {
                            text: text.to_string(),
                        });
                    }
                }
                "refusal" => {
                    let text = block
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("[refused]");
                    blocks.push(EventBlock::Text {
                        text: text.to_string(),
                    });
                }
                "input_image" => {
                    if let Some(image_block) = codex_image_block(block) {
                        blocks.push(image_block);
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Info,
                            disposition: MappingDisposition::Normalized,
                            code: "codex_input_image_preserved_raw".to_string(),
                            message: "Codex input_image block was preserved as provider payload"
                                .to_string(),
                            path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                            raw: Some(block.clone()),
                        });
                        blocks.push(EventBlock::ProviderPayload {
                            kind: "input_image".to_string(),
                            payload: block.clone(),
                        });
                    }
                }
                other => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: MappingDisposition::Normalized,
                        code: "codex_unknown_block_preserved".to_string(),
                        message: format!("Preserved unknown Codex content block '{}'", other),
                        path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                        raw: Some(block.clone()),
                    });
                    blocks.push(EventBlock::Unknown { raw: block.clone() });
                }
            }
        }
    } else if let Some(text) = payload.get("content").and_then(|v| v.as_str()) {
        blocks.push(EventBlock::Text {
            text: text.to_string(),
        });
    } else {
        blocks.push(EventBlock::ProviderPayload {
            kind: "message_without_content".to_string(),
            payload: payload.clone(),
        });
    }

    if phase.as_deref() == Some("commentary") && blocks.len() == 1 {
        if let EventBlock::Text { text } = &blocks[0] {
            blocks[0] = EventBlock::Thinking {
                text: text.clone(),
                signature: None,
            };
        }
    }

    let role = match role_str {
        Some("user") => EventRole::User,
        Some("assistant") => EventRole::Assistant,
        Some("developer") => EventRole::Developer,
        Some("system") => EventRole::System,
        Some("tool") => EventRole::Tool,
        _ => EventRole::Unknown,
    };

    SessionEvent {
        id: event_id,
        kind: SessionEventKind::Message,
        role,
        timestamp,
        links: EventLinks::default(),
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: role_str.map(str::to_string),
                phase,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("codex_payload".to_string(), payload.clone());
                ext.insert("codex_raw_line".to_string(), raw_line);
                ext
            },
        },
    }
}

fn codex_event_msg_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
) -> SessionEvent {
    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("event_msg");
    let role = match event_type {
        "user_message" => EventRole::User,
        "agent_message" => EventRole::Assistant,
        _ => EventRole::System,
    };

    let mut blocks = Vec::new();
    if let Some(text) = payload.get("message").and_then(|v| v.as_str()) {
        blocks.push(EventBlock::Text {
            text: text.to_string(),
        });
    }
    if let Some(text) = payload.get("last_agent_message").and_then(|v| v.as_str()) {
        if !text.trim().is_empty() {
            blocks.push(EventBlock::Text {
                text: text.to_string(),
            });
        }
    }
    blocks.push(EventBlock::ProviderPayload {
        kind: event_type.to_string(),
        payload: payload.clone(),
    });

    let mut event = provider_payload_event(
        format!("codex:event_msg:{}:{}", event_type, line_no),
        SessionEventKind::Lifecycle,
        role,
        timestamp,
        event_type,
        payload.clone(),
        raw_line,
        payload
            .get("phase")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    );
    event.blocks = blocks;
    event
}

fn codex_image_block(block: &Value) -> Option<EventBlock> {
    let mime_type = block
        .get("mime_type")
        .or_else(|| block.get("mimeType"))
        .and_then(|v| v.as_str())
        .unwrap_or("image/*")
        .to_string();
    let image_url = block
        .get("image_url")
        .or_else(|| block.get("url"))
        .or_else(|| block.get("source"))
        .and_then(|v| v.as_str())?;
    if let Some((mime, data)) = parse_data_uri(image_url) {
        return Some(EventBlock::Image {
            mime_type: mime.to_string(),
            data: Some(data.to_string()),
            path: None,
        });
    }
    Some(EventBlock::Image {
        mime_type,
        data: None,
        path: Some(image_url.to_string()),
    })
}

fn parse_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

fn provider_payload_event(
    id: String,
    kind: SessionEventKind,
    role: EventRole,
    timestamp: chrono::DateTime<Utc>,
    payload_kind: &str,
    payload: Value,
    raw_line: Value,
    phase: Option<String>,
) -> SessionEvent {
    SessionEvent {
        id,
        kind,
        role,
        timestamp,
        links: EventLinks::default(),
        blocks: vec![EventBlock::ProviderPayload {
            kind: payload_kind.to_string(),
            payload: payload.clone(),
        }],
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: None,
                phase,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("codex_payload".to_string(), payload);
                ext.insert("codex_raw_line".to_string(), raw_line);
                ext
            },
        },
    }
}

fn build_cwd_lookup() -> Result<std::collections::HashMap<String, String>> {
    let sqlite_path = get_codex_dir().join("state_5.sqlite");
    if !sqlite_path.exists() {
        return Ok(std::collections::HashMap::new());
    }
    let conn = rusqlite::Connection::open(&sqlite_path)?;
    let mut stmt = conn.prepare("SELECT id, cwd FROM threads")?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let cwd: String = row.get(1)?;
        Ok((id, cwd))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        if let Ok((id, cwd)) = row {
            map.insert(id, cwd);
        }
    }
    Ok(map)
}

fn extract_cwd_from_session_file(id: &str) -> Option<String> {
    let path = find_session_file(id)?;
    let file = File::open(&path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(5) {
        let line = line.ok()?;
        let value: Value = serde_json::from_str(&line).ok()?;
        if value.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
            return value
                .get("payload")
                .and_then(|p| p.get("cwd"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    None
}

fn get_codex_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let timestamp_str = now.format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("rollout-{}-{}.jsonl", timestamp_str, session_id);
    let sessions_dir = get_codex_dir()
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string());
    std::fs::create_dir_all(&sessions_dir)?;

    let file_path = sessions_dir.join(&filename);
    let rollout_path = file_path.to_string_lossy().to_string();
    let mut file = File::create(&file_path)?;
    let git_info = get_git_info(target_dir);
    let codex_version = get_codex_version();
    let codex_model_provider = get_codex_model_provider();
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = canonical_session_title(session);
    let base_instructions = session
        .events
        .iter()
        .find(|event| matches!(event.role, EventRole::System | EventRole::Developer))
        .map(canonical_event_text)
        .filter(|text| !text.trim().is_empty());

    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": now.to_rfc3339(),
            "type": "session_meta",
            "payload": {
                "id": session_id,
                "timestamp": now.to_rfc3339(),
                "cwd": target_dir_str,
                "originator": "memorph-cli",
                "cli_version": codex_version,
                "source": "cli",
                "model_provider": codex_model_provider,
                "title": title,
                "base_instructions": base_instructions.as_ref().map(|text| {
                    serde_json::json!({ "text": text })
                }).unwrap_or(Value::Null),
                "git": {
                    "commit_hash": git_info.as_ref().and_then(|git| git.commit_hash.clone()).unwrap_or_default(),
                    "branch": git_info.as_ref().and_then(|git| git.branch.clone()).unwrap_or_default(),
                }
            }
        }))?
    )?;

    let turn_id = Uuid::new_v4().to_string();
    let first_ts = session
        .events
        .first()
        .map(|event| event.timestamp)
        .unwrap_or(now);
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": first_ts.to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "task_started",
                "turn_id": turn_id,
                "started_at": first_ts.timestamp(),
                "collaboration_mode_kind": "default"
            }
        }))?
    )?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": first_ts.to_rfc3339(),
            "type": "turn_context",
            "payload": {
                "turn_id": turn_id,
                "cwd": target_dir_str,
                "current_date": first_ts.format("%Y-%m-%d").to_string(),
                "timezone": "Asia/Shanghai"
            }
        }))?
    )?;

    let mut wrote_user_event = false;
    let mut last_agent_message = String::new();
    for event in &session.events {
        if event.role == EventRole::System {
            continue;
        }
        let role = match event.role {
            EventRole::Assistant => "assistant",
            EventRole::Developer => "developer",
            EventRole::User | EventRole::Tool | EventRole::Unknown => "user",
            EventRole::System => continue,
        };
        let content = canonical_event_to_codex_content(event);
        if content.is_empty() {
            continue;
        }
        let mut payload = serde_json::json!({
            "type": "message",
            "role": role,
            "content": content,
        });
        if event.role == EventRole::Assistant {
            payload["phase"] = Value::String("final_answer".to_string());
            last_agent_message = canonical_event_text(event);
            writeln!(
                file,
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "timestamp": event.timestamp.to_rfc3339(),
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "message": last_agent_message,
                        "phase": "final_answer",
                        "memory_citation": null
                    }
                }))?
            )?;
        }
        writeln!(
            file,
            "{}",
            serde_json::to_string(&serde_json::json!({
                "timestamp": event.timestamp.to_rfc3339(),
                "type": "response_item",
                "payload": payload,
            }))?
        )?;
        if event.role == EventRole::User && !wrote_user_event {
            let user_text = canonical_event_text(event);
            writeln!(
                file,
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "timestamp": event.timestamp.to_rfc3339(),
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": user_text,
                        "images": [],
                        "local_images": [],
                        "text_elements": []
                    }
                }))?
            )?;
            wrote_user_event = true;
        }
    }

    let last_ts = session
        .events
        .last()
        .map(|event| event.timestamp)
        .unwrap_or(now);
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": last_ts.to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": turn_id,
                "last_agent_message": last_agent_message,
                "completed_at": last_ts.timestamp(),
                "duration_ms": 1000
            }
        }))?
    )?;

    let index_path = get_codex_dir().join("session_index.jsonl");
    let mut index_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&index_path)?;
    writeln!(
        index_file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "id": session_id,
            "thread_name": title,
            "updated_at": now.to_rfc3339(),
        }))?
    )?;
    update_codex_sqlite(&session_id, &rollout_path, target_dir, &title, &now)?;
    Ok(session_id)
}

fn canonical_event_to_codex_content(event: &SessionEvent) -> Vec<Value> {
    event
        .blocks
        .iter()
        .filter_map(|block| match block {
            EventBlock::Text { text } => Some(serde_json::json!({
                "type": if event.role == EventRole::Assistant { "output_text" } else { "input_text" },
                "text": text,
            })),
            EventBlock::Thinking { text, .. } => Some(serde_json::json!({
                "type": "output_text",
                "text": format!("[Thinking]\n{}", text),
            })),
            EventBlock::Image { data: Some(data), .. } if event.role != EventRole::Assistant => {
                Some(serde_json::json!({
                    "type": "input_image",
                    "image_url": data,
                }))
            }
            _ => {
                let text = canonical_block_text(block);
                (!text.trim().is_empty()).then(|| serde_json::json!({
                    "type": if event.role == EventRole::Assistant { "output_text" } else { "input_text" },
                    "text": text,
                }))
            }
        })
        .collect()
}

fn find_session_file(id: &str) -> Option<PathBuf> {
    // Search both active sessions and archived sessions
    let dirs = [
        get_codex_dir().join("sessions"),
        get_codex_dir().join("archived_sessions"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(id))
                .unwrap_or(false)
            {
                return Some(path.to_path_buf());
            }
        }
    }

    None
}

#[derive(Default)]
struct GitInfo {
    commit_hash: Option<String>,
    branch: Option<String>,
}

fn get_git_info(dir: &Path) -> Option<GitInfo> {
    let mut info = GitInfo::default();

    let branch_output = std::process::Command::new("git")
        .args([
            "-C",
            &dir.to_string_lossy(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
        .ok()?;
    if branch_output.status.success() {
        info.branch = Some(
            String::from_utf8_lossy(&branch_output.stdout)
                .trim()
                .to_string(),
        );
    }

    let hash_output = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if hash_output.status.success() {
        info.commit_hash = Some(
            String::from_utf8_lossy(&hash_output.stdout)
                .trim()
                .to_string(),
        );
    }

    Some(info)
}

fn update_codex_sqlite(
    session_id: &str,
    rollout_path: &str,
    cwd: &Path,
    title: &str,
    now: &chrono::DateTime<Utc>,
) -> Result<()> {
    let sqlite_path = get_codex_dir().join("state_5.sqlite");
    if !sqlite_path.exists() {
        // SQLite not present, skip
        return Ok(());
    }

    let conn = rusqlite::Connection::open(&sqlite_path)
        .with_context(|| format!("Failed to open Codex SQLite: {}", sqlite_path.display()))?;

    let created_at = now.timestamp();
    let created_at_ms = now.timestamp_millis();
    let cwd_str = cwd.to_string_lossy().to_string();
    let codex_version = get_codex_version();
    let codex_model_provider = get_codex_model_provider();
    let (codex_model, codex_reasoning) = get_codex_model_config();
    let sandbox_json = format!(
        "{{\"type\":\"workspace-write\",\"writable_roots\":[],\"network_access\":false,\"exclude_tmpdir_env_var\":false,\"exclude_slash_tmp\":false}}"
    );

    conn.execute(
        "INSERT INTO threads (
            id, rollout_path, created_at, updated_at, source, model_provider,
            cwd, title, sandbox_policy, approval_mode, tokens_used, has_user_event,
            archived, cli_version, first_user_message, memory_mode, git_branch,
            model, reasoning_effort, created_at_ms, updated_at_ms
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
        ) ON CONFLICT(id) DO UPDATE SET
            updated_at = excluded.updated_at,
            updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            session_id,
            rollout_path,
            created_at,
            created_at,
            "cli",
            codex_model_provider,
            cwd_str,
            title,
            sandbox_json,
            "on-request",
            0,
            0,
            0,
            codex_version,
            title,
            "enabled",
            get_git_branch(cwd).unwrap_or_else(|| "main".to_string()),
            codex_model,
            codex_reasoning,
            created_at_ms,
            created_at_ms,
        ],
    )
    .with_context(|| "Failed to insert thread into Codex SQLite")?;

    Ok(())
}

fn get_codex_version() -> String {
    let version_path = get_codex_dir().join("version.json");
    if let Ok(content) = std::fs::read_to_string(&version_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(ver) = v.get("latest_version").and_then(|v| v.as_str()) {
                return ver.to_string();
            }
        }
    }
    "0.124.0".to_string()
}

fn get_codex_model_provider() -> String {
    let config_path = get_codex_dir().join("config.toml");
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("model_provider") && !trimmed.starts_with("model_providers") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    return val.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    "openai".to_string()
}

fn get_codex_model_config() -> (String, String) {
    let config_path = get_codex_dir().join("config.toml");
    let mut model = String::new();
    let mut reasoning = String::new();
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if model.is_empty() && trimmed.starts_with("model ") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    model = val.trim().trim_matches('"').to_string();
                }
            }
            if reasoning.is_empty() && trimmed.starts_with("model_reasoning_effort") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    reasoning = val.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    if model.is_empty() {
        model = "gpt-5.3-codex".to_string();
    }
    if reasoning.is_empty() {
        reasoning = "xhigh".to_string();
    }
    (model, reasoning)
}

fn get_git_branch(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &dir.to_string_lossy(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::NamedTempFile;

    #[test]
    fn import_canonical_session_preserves_codex_runtime_and_message_events() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-1",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project",
                    "base_instructions": { "text": "Be careful." },
                    "model": "gpt-5.3-codex"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "turn_context",
                "payload": {
                    "turn_id": "turn-1",
                    "cwd": "/tmp/project",
                    "current_date": "2026-05-21",
                    "timezone": "Asia/Shanghai"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_started",
                    "turn_id": "turn-1",
                    "started_at": 1747821602
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:03Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "developer",
                    "content": [
                        { "type": "input_text", "text": "# AGENTS.md instructions" }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:04Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": [
                        { "type": "output_text", "text": "Thinking out loud" }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:05Z",
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "shell"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:06Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "Done."
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let events = &imported.session.events;

        assert_eq!(imported.session.identity.canonical_id, "session-1");
        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some("/tmp/project")
        );
        assert!(events.iter().any(|event| {
            event.role == EventRole::System
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "Be careful."
                )
        }));
        assert!(events.iter().any(|event| {
            event.role == EventRole::Developer
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "# AGENTS.md instructions"
                )
        }));
        assert!(events.iter().any(|event| {
            event.role == EventRole::Assistant
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Thinking { text, .. }) if text == "Thinking out loud"
                )
        }));
        assert!(events.iter().any(|event| {
            event.id == "codex:response_item:6"
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ProviderPayload { kind, .. }) if kind == "function_call"
                )
        }));
        assert!(events.iter().any(|event| {
            event.id == "codex:event_msg:task_complete:7"
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "Done."
                )
        }));
    }

    #[test]
    fn import_canonical_session_decodes_input_image_data_uri() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-2",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_image",
                            "mime_type": "image/png",
                            "image_url": "data:image/png;base64,QUJD"
                        }
                    ]
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let image_block = imported
            .session
            .events
            .iter()
            .flat_map(|event| event.blocks.iter())
            .find_map(|block| match block {
                EventBlock::Image {
                    mime_type,
                    data,
                    path,
                } => Some((mime_type, data, path)),
                _ => None,
            })
            .expect("expected image block");

        assert_eq!(image_block.0, "image/png");
        assert_eq!(image_block.1.as_deref(), Some("QUJD"));
        assert_eq!(image_block.2, &None);
    }
}
