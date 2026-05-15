use crate::model::{
    ContentBlock, MemorphMessage, MemorphMeta, MemorphRole, MemorphSession, SessionInfo,
    SessionMeta,
};
use crate::provider::{Provider, ProviderCapabilities};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde_json::Value;
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

    fn scan_sessions(&self) -> Result<Vec<SessionMeta>> {
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

                sessions.push(SessionMeta {
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

    fn load_session(&self, source_path: &str) -> Result<MemorphSession> {
        let path = Path::new(source_path);
        let file = File::open(path)
            .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;
        let reader = BufReader::new(file);

        let mut messages = Vec::new();
        let mut session_id: Option<String> = None;
        let mut project_dir: Option<String> = None;
        let mut created_at: Option<i64> = None;
        let mut base_instructions: Option<String> = None;
        let mut model: Option<String> = None;

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let line_type = value.get("type").and_then(|v| v.as_str());

            match line_type {
                Some("session_meta") => {
                    if let Some(payload) = value.get("payload") {
                        session_id = payload
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        project_dir = payload
                            .get("cwd")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        created_at = payload
                            .get("timestamp")
                            .and_then(|v| v.as_str())
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.timestamp_millis());
                        base_instructions = payload
                            .get("base_instructions")
                            .and_then(|v| v.get("text"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        model = payload
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                }
                Some("response_item") => {
                    if let Some(payload) = value.get("payload") {
                        let role = payload.get("role").and_then(|v| v.as_str());
                        let msg_type = payload.get("type").and_then(|v| v.as_str());

                        // Skip non-message items (like function calls handled separately)
                        if msg_type != Some("message") {
                            continue;
                        }

                        let memorph_role = match role {
                            Some("user") => MemorphRole::User,
                            Some("assistant") => MemorphRole::Assistant,
                            Some("developer") => {
                                // Skip Codex runtime-injected developer messages
                                // (permissions, skills, collaboration_mode, etc.)
                                // base_instructions are already extracted separately
                                // from session_meta as a system message.
                                continue;
                            }
                            Some("system") => MemorphRole::System,
                            Some("tool") => MemorphRole::Tool,
                            _ => continue,
                        };

                        let content = if let Some(content_arr) =
                            payload.get("content").and_then(|v| v.as_array())
                        {
                            content_arr
                                .iter()
                                .filter_map(|block| {
                                    let block_type = block.get("type").and_then(|v| v.as_str())?;
                                    match block_type {
                                        "input_text" | "output_text" => {
                                            let text =
                                                block.get("text").and_then(|v| v.as_str())?;
                                            Some(ContentBlock::text(text))
                                        }
                                        "input_image" => {
                                            // TODO: handle images
                                            None
                                        }
                                        "refusal" => {
                                            let text = block
                                                .get("text")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("[refused]");
                                            Some(ContentBlock::text(text))
                                        }
                                        _ => None,
                                    }
                                })
                                .collect::<Vec<_>>()
                        } else if let Some(text) = payload.get("content").and_then(|v| v.as_str()) {
                            vec![ContentBlock::text(text)]
                        } else {
                            continue;
                        };

                        if content.is_empty() {
                            continue;
                        }

                        // Skip Codex runtime-injected context messages that are
                        // wrapped as user role (AGENTS.md, environment_context, etc.)
                        if memorph_role == MemorphRole::User {
                            let full_text: String = content
                                .iter()
                                .filter_map(|b| match b {
                                    crate::model::ContentBlock::Text { text } => {
                                        Some(text.as_str())
                                    }
                                    _ => None,
                                })
                                .collect();
                            let is_injected = full_text.contains("<INSTRUCTIONS>")
                                || full_text.contains("<environment_context>")
                                || full_text.contains("<permissions instructions>")
                                || full_text.contains("<skills_instructions>")
                                || full_text.contains("<collaboration_mode>")
                                || full_text.starts_with("# AGENTS.md instructions for");
                            if is_injected {
                                continue;
                            }
                        }

                        let ts = value
                            .get("timestamp")
                            .and_then(|v| v.as_str())
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                            .map(|dt| dt.with_timezone(&chrono::Utc))
                            .unwrap_or_else(Utc::now);

                        let msg_id = Uuid::new_v4().to_string();

                        // Check if this is a commentary/thinking phase
                        let phase = payload.get("phase").and_then(|v| v.as_str());
                        let is_commentary = phase == Some("commentary");

                        let final_content = if is_commentary && content.len() == 1 {
                            // Wrap commentary as thinking block
                            if let ContentBlock::Text { text } = &content[0] {
                                vec![ContentBlock::Thinking {
                                    thinking: text.clone(),
                                    signature: None,
                                }]
                            } else {
                                content
                            }
                        } else {
                            content
                        };

                        messages.push(MemorphMessage {
                            id: msg_id,
                            role: memorph_role,
                            content: final_content,
                            timestamp: ts,
                            metadata: Some(crate::model::MessageMetadata {
                                source: Some(crate::model::SourceMetadata {
                                    provider: PROVIDER_ID.to_string(),
                                    original_id: None,
                                    original_role: role.map(|s| s.to_string()),
                                }),
                                model: model.clone(),
                                usage: None,
                                extra: {
                                    let mut extra = serde_json::Map::new();
                                    if let Some(phase) = phase {
                                        extra.insert(
                                            "phase".to_string(),
                                            serde_json::Value::String(phase.to_string()),
                                        );
                                    }
                                    serde_json::Value::Object(extra)
                                },
                            }),
                            parent_id: None,
                            turn_index: None,
                        });
                    }
                }
                _ => {}
            }
        }

        // Add base_instructions as first system/developer message if present
        if let Some(instr) = base_instructions {
            messages.insert(
                0,
                MemorphMessage {
                    id: Uuid::new_v4().to_string(),
                    role: MemorphRole::System,
                    content: vec![ContentBlock::text(instr)],
                    timestamp: created_at
                        .map(|ms| {
                            chrono::DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now)
                        })
                        .unwrap_or_else(Utc::now),
                    metadata: Some(crate::model::MessageMetadata {
                        source: Some(crate::model::SourceMetadata {
                            provider: PROVIDER_ID.to_string(),
                            original_id: None,
                            original_role: Some("developer".to_string()),
                        }),
                        model: None,
                        usage: None,
                        extra: serde_json::json!({"type": "base_instructions"}),
                    }),
                    parent_id: None,
                    turn_index: Some(0),
                },
            );
        }

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
            title: None,
            project_dir,
            created_at: created_at
                .map(|ms| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now)),
            last_active_at: created_at
                .map(|ms| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now)),
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
        let now = Utc::now();
        let timestamp_str = now.format("%Y-%m-%dT%H-%M-%S").to_string();
        let filename = format!("rollout-{}-{}.jsonl", timestamp_str, session_id);

        // Codex stores sessions in dated subdirectories
        let year = now.format("%Y").to_string();
        let month = now.format("%m").to_string();
        let day = now.format("%d").to_string();
        let sessions_dir = get_codex_dir()
            .join("sessions")
            .join(&year)
            .join(&month)
            .join(&day);
        std::fs::create_dir_all(&sessions_dir)?;

        let file_path = sessions_dir.join(&filename);
        let rollout_path = file_path.to_string_lossy().to_string();

        let mut file = File::create(&file_path)?;

        // Build base instructions from first system message
        let base_instructions = session
            .messages
            .iter()
            .find(|m| m.role == MemorphRole::System)
            .and_then(|m| {
                m.content.iter().find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            });

        // Write session_meta line
        let git_info = get_git_info(target_dir);
        let codex_version = get_codex_version();
        let codex_model_provider = get_codex_model_provider();
        let session_meta = serde_json::json!({
            "timestamp": now.to_rfc3339(),
            "type": "session_meta",
            "payload": {
                "id": session_id,
                "timestamp": now.to_rfc3339(),
                "cwd": target_dir.to_string_lossy().to_string(),
                "originator": "memorph-cli",
                "cli_version": codex_version,
                "source": "cli",
                "model_provider": codex_model_provider,
                "base_instructions": base_instructions.as_ref().map(|text| {
                    serde_json::json!({ "text": text })
                }).unwrap_or(serde_json::Value::Null),
                "git": {
                    "commit_hash": git_info.as_ref().and_then(|g| g.commit_hash.clone()).unwrap_or_default(),
                    "branch": git_info.as_ref().and_then(|g| g.branch.clone()).unwrap_or_default(),
                }
            }
        });
        writeln!(file, "{}", serde_json::to_string(&session_meta)?)?;

        // Group non-system messages into turns (each user message starts a new turn)
        let mut turns: Vec<Vec<&MemorphMessage>> = Vec::new();
        for msg in &session.messages {
            if msg.role == MemorphRole::System {
                continue;
            }
            if msg.role == MemorphRole::User && !turns.is_empty() {
                turns.push(Vec::new());
            }
            if turns.is_empty() {
                turns.push(Vec::new());
            }
            turns.last_mut().unwrap().push(msg);
        }

        let target_dir_str = target_dir.to_string_lossy().to_string();

        for turn in &turns {
            let turn_id = Uuid::new_v4().to_string();
            let first_msg = turn.first().unwrap();
            let last_msg = turn.last().unwrap();
            let started_at = first_msg.timestamp.timestamp();
            let completed_at = last_msg.timestamp.timestamp();
            let turn_ts = first_msg.timestamp.to_rfc3339();

            // task_started
            // NOTE: model_context_window is intentionally omitted.
            // It is Option<i64> in Codex's protocol. Hard-coding it can trigger
            // a known bug (GitHub #16068) where the token counter gets poisoned
            // and auto-compaction never fires, leading to repeated context-window
            // crashes. Let Codex derive the window size from its own config.
            let task_started = serde_json::json!({
                "timestamp": turn_ts,
                "type": "event_msg",
                "payload": {
                    "type": "task_started",
                    "turn_id": turn_id,
                    "started_at": started_at,
                    "collaboration_mode_kind": "default"
                }
            });
            writeln!(file, "{}", serde_json::to_string(&task_started)?)?;

            // turn_context
            // NOTE: Only preserve history-specific info (turn_id, cwd, date, timezone).
            // model/effort/sandbox_policy/collaboration_mode/approval_policy etc.
            // Runtime configs are intentionally omitted; Codex reads them from config.toml.
            let current_date = first_msg.timestamp.format("%Y-%m-%d").to_string();
            let turn_context = serde_json::json!({
                "timestamp": turn_ts,
                "type": "turn_context",
                "payload": {
                    "turn_id": turn_id,
                    "cwd": target_dir_str,
                    "current_date": current_date,
                    "timezone": "Asia/Shanghai"
                }
            });
            writeln!(file, "{}", serde_json::to_string(&turn_context)?)?;

            let last_assistant_idx = turn.iter().rposition(|m| m.role == MemorphRole::Assistant);
            let mut user_message_written = false;

            for (idx, msg) in turn.iter().enumerate() {
                let timestamp = msg.timestamp.to_rfc3339();
                let role_str = match msg.role {
                    MemorphRole::User => "user",
                    MemorphRole::Assistant => "assistant",
                    MemorphRole::Tool => "user",
                    MemorphRole::Developer => "developer",
                    MemorphRole::System => continue,
                };

                let content_blocks: Vec<serde_json::Value> = match msg.role {
                    MemorphRole::Tool => {
                        let text = msg.content.iter()
                            .filter_map(|b| match b {
                                ContentBlock::Text { text } => Some(text.as_str()),
                                ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        vec![serde_json::json!({
                            "type": "input_text",
                            "text": format!("[Tool Result]\n{}", text),
                        })]
                    }
                    MemorphRole::Assistant => {
                        msg.content.iter()
                            .filter_map(|b| match b {
                                ContentBlock::Text { text } => Some(serde_json::json!({
                                    "type": "output_text",
                                    "text": text,
                                })),
                                ContentBlock::Thinking { thinking, .. } => Some(serde_json::json!({
                                    "type": "output_text",
                                    "text": format!("[Thinking]\n{}", thinking),
                                })),
                                ContentBlock::ToolUse { id, name, input } => Some(serde_json::json!({
                                    "type": "output_text",
                                    "text": format!("[Tool Use: {} (id={})]\nInput: {}", name, id, input.as_ref().map(|v| v.to_string()).unwrap_or_default()),
                                })),
                                _ => None,
                            })
                            .collect()
                    }
                    _ => {
                        let text = msg.content.iter()
                            .filter_map(|b| match b {
                                ContentBlock::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        vec![serde_json::json!({
                            "type": "input_text",
                            "text": text,
                        })]
                    }
                };

                if content_blocks.is_empty() {
                    continue;
                }

                let mut payload = serde_json::json!({
                    "type": "message",
                    "role": role_str,
                    "content": content_blocks,
                });
                if msg.role == MemorphRole::Assistant {
                    payload["phase"] = serde_json::Value::String("final_answer".to_string());
                }
                // agent_message event before the last assistant response_item
                if Some(idx) == last_assistant_idx {
                    let agent_text = msg
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    let agent_event = serde_json::json!({
                        "timestamp": timestamp,
                        "type": "event_msg",
                        "payload": {
                            "type": "agent_message",
                            "message": agent_text,
                            "phase": "final_answer",
                            "memory_citation": null
                        }
                    });
                    writeln!(file, "{}", serde_json::to_string(&agent_event)?)?;
                }

                let response_item = serde_json::json!({
                    "timestamp": timestamp,
                    "type": "response_item",
                    "payload": payload,
                });
                writeln!(file, "{}", serde_json::to_string(&response_item)?)?;

                // user_message event after the first original user message
                if msg.role == MemorphRole::User && !user_message_written {
                    let user_text = msg
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    let user_event = serde_json::json!({
                        "timestamp": timestamp,
                        "type": "event_msg",
                        "payload": {
                            "type": "user_message",
                            "message": user_text,
                            "images": [],
                            "local_images": [],
                            "text_elements": []
                        }
                    });
                    writeln!(file, "{}", serde_json::to_string(&user_event)?)?;
                    user_message_written = true;
                }
            }

            // task_complete
            let last_agent_message = last_assistant_idx
                .and_then(|idx| turn.get(idx))
                .map(|msg| {
                    msg.content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();

            let task_complete = serde_json::json!({
                "timestamp": last_msg.timestamp.to_rfc3339(),
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": turn_id,
                    "last_agent_message": last_agent_message,
                    "completed_at": completed_at,
                    "duration_ms": 1000
                }
            });
            writeln!(file, "{}", serde_json::to_string(&task_complete)?)?;
        }

        // Update session_index.jsonl
        let index_path = get_codex_dir().join("session_index.jsonl");
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

        let index_entry = serde_json::json!({
            "id": session_id,
            "thread_name": title,
            "updated_at": now.to_rfc3339(),
        });

        let mut index_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&index_path)?;
        writeln!(index_file, "{}", serde_json::to_string(&index_entry)?)?;

        // Update state_5.sqlite
        update_codex_sqlite(&session_id, &rollout_path, target_dir, &title, &now)?;

        Ok(session_id)
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
            conn.execute("DELETE FROM thread WHERE id = ?1", [session_id])?;
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
