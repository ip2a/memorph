use crate::model::{
    ContentBlock, MemorphMeta, MemorphMessage, MemorphRole, MemorphSession, SessionInfo, SessionMeta,
};
use crate::provider::Provider;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct OpenCodeProvider;

const PROVIDER_ID: &str = "opencode";
const OPENCODE_VERSION: &str = "1.3.17";

impl Provider for OpenCodeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "OpenCode"
    }

    fn scan_sessions(&self) -> Result<Vec<SessionMeta>> {
        let mut sessions = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // 1. Scan SQLite DB (primary source for recent sessions)
        if let Ok(db_sessions) = scan_sessions_from_db() {
            for s in db_sessions {
                seen.insert(s.session_id.clone());
                sessions.push(s);
            }
        }

        // 2. Scan filesystem (fallback for older sessions)
        let storage_dir = get_opencode_dir().join("storage").join("session");
        if storage_dir.exists() {
            for entry in WalkDir::new(&storage_dir)
                .max_depth(3)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Some(meta) = parse_session_file(path) {
                    if !seen.contains(&meta.session_id) {
                        sessions.push(meta);
                    }
                }
            }
        }

        Ok(sessions)
    }

    fn load_session(&self, source_path: &str) -> Result<MemorphSession> {
        let session_id = source_path;

        // Try DB first, fallback to filesystem
        let (session_json, messages, parts) =
            if let Ok(data) = load_session_from_db(session_id) {
                data
            } else {
                load_session_from_filesystem(session_id)?
            };

        // Build messages linearly: for each user message, then its assistant children
        let mut memorph_messages = Vec::new();

        // Sort messages by creation time
        let mut msg_list: Vec<(i64, Value, Vec<Value>)> = messages
            .into_iter()
            .map(|(created, msg_json)| {
                let msg_id = msg_json
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let msg_parts: Vec<Value> = parts
                    .get(&msg_id)
                    .cloned()
                    .unwrap_or_default();
                (created, msg_json, msg_parts)
            })
            .collect();
        msg_list.sort_by_key(|(created, _, _)| *created);

        for (_, msg_json, msg_parts) in msg_list {
            let role_str = msg_json
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user");
            let role = match role_str {
                "user" => MemorphRole::User,
                "assistant" => MemorphRole::Assistant,
                _ => continue,
            };

            let msg_id = msg_json
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let parent_id = msg_json
                .get("parentID")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let created = msg_json
                .get("time")
                .and_then(|v| v.get("created"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| Utc::now().timestamp_millis());
            let ts = chrono::DateTime::from_timestamp_millis(created)
                .unwrap_or_else(Utc::now);

            let mut content_blocks = Vec::new();

            for part in msg_parts {
                let part_type = part.get("type").and_then(|v| v.as_str());
                match part_type {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            content_blocks.push(ContentBlock::text(text));
                        }
                    }
                    Some("reasoning") => {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            content_blocks.push(ContentBlock::Thinking {
                                thinking: text.to_string(),
                                signature: None,
                            });
                        }
                    }
                    Some("tool") => {
                        let call_id = part
                            .get("callID")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let state = part.get("state").cloned().unwrap_or_default();
                        let status = state
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("completed");
                        let output = state
                            .get("output")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let is_error = status == "error";
                        content_blocks.push(ContentBlock::ToolResult {
                            tool_use_id: call_id,
                            content: output.to_string(),
                            is_error: Some(is_error),
                        });
                    }
                    Some("file") => {
                        let mime = part
                            .get("mime")
                            .and_then(|v| v.as_str())
                            .unwrap_or("application/octet-stream");
                        let filename = part
                            .get("filename")
                            .and_then(|v| v.as_str())
                            .unwrap_or("file");
                        let url = part
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if mime.starts_with("image/") && url.starts_with("data:") {
                            // Parse data URI
                            if let Some((mime_type, data)) = parse_data_uri(url) {
                                content_blocks.push(ContentBlock::Image {
                                    mime_type: mime_type.to_string(),
                                    data: data.to_string(),
                                });
                            }
                        } else if !url.is_empty() {
                            content_blocks.push(ContentBlock::File {
                                path: filename.to_string(),
                                content: Some(url.to_string()),
                            });
                        }
                    }
                    Some("patch") => {
                        let files = part
                            .get("files")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        let hash = part
                            .get("hash")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        content_blocks.push(ContentBlock::text(format!(
                            "[Patch: {} (files: {})]",
                            hash, files
                        )));
                    }
                    Some("step-start") | Some("step-finish") | Some("compaction") => {
                        // Skip metadata parts
                    }
                    _ => {}
                }
            }

            if content_blocks.is_empty() {
                continue;
            }

            let model = msg_json
                .get("modelID")
                .or_else(|| msg_json.get("model").and_then(|m| m.get("modelID")))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let provider = msg_json
                .get("providerID")
                .or_else(|| msg_json.get("model").and_then(|m| m.get("providerID")))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let usage = msg_json.get("tokens").and_then(|t| {
                Some(crate::model::TokenUsage {
                    input_tokens: t.get("input").and_then(|v| v.as_u64()),
                    output_tokens: t.get("output").and_then(|v| v.as_u64()),
                    total_tokens: t.get("total").and_then(|v| v.as_u64()),
                })
            });

            let finish = msg_json
                .get("finish")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let cost = msg_json.get("cost").and_then(|v| v.as_f64());
            let agent = msg_json
                .get("agent")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let mode = msg_json
                .get("mode")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let mut extra = serde_json::Map::new();
            if let Some(f) = finish {
                extra.insert("finish".to_string(), Value::String(f));
            }
            if let Some(c) = cost {
                extra.insert("cost".to_string(), Value::from(c));
            }
            if let Some(a) = agent {
                extra.insert("agent".to_string(), Value::String(a));
            }
            if let Some(m) = mode {
                extra.insert("mode".to_string(), Value::String(m));
            }

            memorph_messages.push(MemorphMessage {
                id: msg_id,
                role,
                content: content_blocks,
                timestamp: ts,
                metadata: Some(crate::model::MessageMetadata {
                    source: Some(crate::model::SourceMetadata {
                        provider: provider.unwrap_or_else(|| PROVIDER_ID.to_string()),
                        original_id: None,
                        original_role: Some(role_str.to_string()),
                    }),
                    model,
                    usage,
                    extra: Value::Object(extra),
                }),
                parent_id,
                turn_index: None,
            });
        }

        let session_id_val = session_json
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(session_id)
            .to_string();
        let title = session_json
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let project_dir = session_json
            .get("directory")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let created = session_json
            .get("time")
            .and_then(|v| v.get("created"))
            .and_then(|v| v.as_i64());
        let updated = session_json
            .get("time")
            .and_then(|v| v.get("updated"))
            .and_then(|v| v.as_i64());

        let meta = MemorphMeta {
            version: "1.0".to_string(),
            converted_from: PROVIDER_ID.to_string(),
            converted_at: Utc::now(),
            memorph_version: env!("CARGO_PKG_VERSION").to_string(),
            source_session_id: session_id_val.clone(),
            source_provider: PROVIDER_ID.to_string(),
            converted_by: Some("memorph-cli".to_string()),
        };

        let session = SessionInfo {
            id: session_id_val,
            title: title.clone(),
            project_dir,
            created_at: created.and_then(|ms| chrono::DateTime::from_timestamp_millis(ms)),
            last_active_at: updated.and_then(|ms| chrono::DateTime::from_timestamp_millis(ms)),
            tags: None,
        };

        Ok(MemorphSession {
            meta,
            session,
            messages: memorph_messages,
        })
    }

    fn write_session(&self, session: &MemorphSession, target_dir: &Path) -> Result<String> {
        let now = Utc::now().timestamp_millis();
        let session_id = generate_opencode_id("ses");
        let project_id = find_or_create_project(target_dir)?;
        let slug = generate_slug();
        let target_dir_str = target_dir.to_string_lossy().to_string();
        let title = session
            .session
            .title
            .clone()
            .unwrap_or_else(|| "Imported session".to_string());

        // Build session JSON
        let session_json = serde_json::json!({
            "id": &session_id,
            "slug": &slug,
            "version": OPENCODE_VERSION,
            "projectID": &project_id,
            "directory": &target_dir_str,
            "title": &title,
            "time": {
                "created": now,
                "updated": now
            }
        });

        // Convert memorph messages to OpenCode messages + parts
        let mut oc_messages: Vec<(String, i64, Value)> = Vec::new();
        let mut oc_parts: Vec<(String, String, i64, Value)> = Vec::new();
        let mut last_user_msg_id: Option<String> = None;

        for msg in &session.messages {
            let msg_id = generate_opencode_id("msg");
            let msg_created = msg.timestamp.timestamp_millis();

            let (role, parent_id) = match msg.role {
                MemorphRole::User => {
                    last_user_msg_id = Some(msg_id.clone());
                    ("user", None)
                }
                MemorphRole::Assistant => ("assistant", last_user_msg_id.clone()),
                MemorphRole::Tool => ("user", last_user_msg_id.clone()),
                MemorphRole::System => {
                    // System messages become user messages in OpenCode
                    last_user_msg_id = Some(msg_id.clone());
                    ("user", None)
                }
                MemorphRole::Developer => {
                    last_user_msg_id = Some(msg_id.clone());
                    ("user", None)
                }
            };

            let mut msg_json = serde_json::Map::new();
            msg_json.insert("id".to_string(), Value::String(msg_id.clone()));
            msg_json.insert("sessionID".to_string(), Value::String(session_id.clone()));
            msg_json.insert("role".to_string(), Value::String(role.to_string()));
            msg_json.insert(
                "time".to_string(),
                serde_json::json!({"created": msg_created}),
            );
            if let Some(pid) = parent_id {
                msg_json.insert("parentID".to_string(), Value::String(pid));
            }

            // Extract model/provider from metadata
            if let Some(meta) = &msg.metadata {
                if let Some(m) = &meta.model {
                    msg_json.insert("modelID".to_string(), Value::String(m.clone()));
                }
                if let Some(source) = &meta.source {
                    msg_json.insert(
                        "providerID".to_string(),
                        Value::String(source.provider.clone()),
                    );
                }
            }

            oc_messages.push((msg_id.clone(), msg_created, Value::Object(msg_json)));

            // Convert content blocks to parts
            for block in &msg.content {
                let part_id = generate_opencode_id("prt");
                let part_created = msg_created + 1;

                let part_json = match block {
                    ContentBlock::Text { text } => {
                        serde_json::json!({
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": msg_id,
                            "type": "text",
                            "text": text,
                        })
                    }
                    ContentBlock::Thinking { thinking, .. } => {
                        serde_json::json!({
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": msg_id,
                            "type": "reasoning",
                            "text": thinking,
                        })
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        // OpenCode doesn't have explicit tool_use parts.
                        // Store as a text annotation.
                        serde_json::json!({
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": msg_id,
                            "type": "text",
                            "text": format!(
                                "[Tool Use: {} (id={})]\nInput: {}",
                                name,
                                id,
                                input.as_ref().map(|v| v.to_string()).unwrap_or_default()
                            ),
                        })
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        serde_json::json!({
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": msg_id,
                            "type": "tool",
                            "callID": tool_use_id,
                            "tool": "unknown",
                            "state": {
                                "status": if *is_error == Some(true) { "error" } else { "completed" },
                                "input": {},
                                "output": content,
                                "title": "Tool result",
                                "metadata": {},
                                "time": {
                                    "start": part_created,
                                    "end": part_created
                                }
                            }
                        })
                    }
                    ContentBlock::Image { mime_type, data } => {
                        serde_json::json!({
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": msg_id,
                            "type": "file",
                            "mime": mime_type,
                            "filename": "image.png",
                            "url": format!("data:{};base64,{}" , mime_type, data),
                        })
                    }
                    ContentBlock::File { path, content } => {
                        serde_json::json!({
                            "id": part_id,
                            "sessionID": session_id,
                            "messageID": msg_id,
                            "type": "file",
                            "mime": "text/plain",
                            "filename": path,
                            "url": content.as_deref().unwrap_or(""),
                        })
                    }
                };

                oc_parts.push((part_id, msg_id.clone(), part_created, part_json));
            }
        }

        // Write to SQLite DB
        if let Err(e) = write_to_db(
            &session_id,
            &project_id,
            &slug,
            &target_dir_str,
            &title,
            now,
            &oc_messages,
            &oc_parts,
        ) {
            eprintln!("Warning: failed to write to OpenCode DB: {}. Falling back to filesystem only.", e);
        }

        // Write to filesystem
        write_to_filesystem(
            &session_id,
            &project_id,
            &session_json,
            &oc_messages,
            &oc_parts,
        )?;

        Ok(session_id)
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let db_path = get_db_path();
        let storage_dir = get_opencode_dir().join("storage");

        // Delete from DB
        if db_path.exists() {
            let conn = Connection::open(&db_path)?;
            conn.execute("DELETE FROM part WHERE session_id = ?1", [session_id])?;
            conn.execute("DELETE FROM message WHERE session_id = ?1", [session_id])?;
            conn.execute("DELETE FROM session WHERE id = ?1", [session_id])?;
        }

        // Delete from filesystem
        // Remove session file
        for entry in WalkDir::new(storage_dir.join("session"))
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                let _ = std::fs::remove_file(path);
                // Try to remove empty parent dirs
                if let Some(parent) = path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
                break;
            }
        }

        // Remove message directory
        let msg_dir = storage_dir.join("message").join(session_id);
        if msg_dir.exists() {
            let _ = std::fs::remove_dir_all(&msg_dir);
        }

        // Remove part directories for this session's messages
        let parts_dir = storage_dir.join("part");
        if parts_dir.exists() {
            for entry in std::fs::read_dir(&parts_dir)? {
                let entry = entry?;
                let msg_dir = entry.path();
                if !msg_dir.is_dir() {
                    continue;
                }
                // Check if this message belongs to our session by looking at the first part
                let mut belongs = false;
                for part_entry in std::fs::read_dir(&msg_dir)? {
                    let part_entry = part_entry?;
                    let part_path = part_entry.path();
                    if part_path.extension().and_then(|e| e.to_str()) == Some("json") {
                        if let Ok(content) = std::fs::read_to_string(&part_path) {
                            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                                if v.get("sessionID").and_then(|v| v.as_str()) == Some(session_id) {
                                    belongs = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                if belongs {
                    let _ = std::fs::remove_dir_all(&msg_dir);
                }
            }
        }

        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let db_path = get_db_path();
        let storage_dir = get_opencode_dir().join("storage");
        let now = Utc::now().timestamp_millis();

        // Update DB
        if db_path.exists() {
            let conn = Connection::open(&db_path)?;
            conn.execute(
                "UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3",
                [new_title, &now.to_string(), session_id],
            )?;
        }

        // Update filesystem
        for entry in WalkDir::new(storage_dir.join("session"))
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                if let Ok(content) = std::fs::read_to_string(path) {
                    if let Ok(mut v) = serde_json::from_str::<Value>(&content) {
                        if let Value::Object(ref mut map) = v {
                            map.insert("title".to_string(), Value::String(new_title.to_string()));
                            if let Some(Value::Object(ref mut time)) = map.get_mut("time") {
                                time.insert("updated".to_string(), Value::Number(now.into()));
                            }
                            let _ = std::fs::write(path, serde_json::to_string_pretty(&v)?);
                        }
                    }
                }
                break;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_opencode_dir() -> PathBuf {
    // OpenCode uses ~/.local/share/opencode even on macOS
    dirs::home_dir()
        .map(|h| h.join(".local/share/opencode"))
        .unwrap_or_else(|| PathBuf::from(".local/share/opencode"))
}

fn generate_opencode_id(prefix: &str) -> String {
    let uuid = Uuid::new_v4().to_string().replace("-", "");
    format!("{}_{}", prefix, &uuid[..24.min(uuid.len())])
}

fn generate_slug() -> String {
    let adjectives = [
        "bright", "calm", "swift", "keen", "bold", "warm", "cool", "sharp", "clear", "steady",
    ];
    let nouns = [
        "river", "forest", "mountain", "ocean", "sky", "star", "path", "garden", "valley",
        "horizon",
    ];
    let idx1 = (Uuid::new_v4().as_u128() % adjectives.len() as u128) as usize;
    let idx2 = (Uuid::new_v4().as_u128() % nouns.len() as u128) as usize;
    format!("{}-{}", adjectives[idx1], nouns[idx2])
}

fn parse_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

// ---------------------------------------------------------------------------
// DB operations
// ---------------------------------------------------------------------------

fn get_db_path() -> PathBuf {
    get_opencode_dir().join("opencode.db")
}

fn scan_sessions_from_db() -> Result<Vec<SessionMeta>> {
    let db_path = get_db_path();
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open(&db_path)?;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, directory, title, time_created, time_updated FROM session WHERE time_archived IS NULL ORDER BY time_updated DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        let session_id: String = row.get(0)?;
        let _project_id: String = row.get(1)?;
        let directory: String = row.get(2)?;
        let title: String = row.get(3)?;
        let _created: i64 = row.get(4)?;
        let updated: i64 = row.get(5)?;
        Ok(SessionMeta {
            session_id: session_id.clone(),
            title: Some(title),
            project_dir: Some(directory),
            last_active_at: Some(updated),
            source_path: Some(session_id),
        })
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        if let Ok(s) = row {
            sessions.push(s);
        }
    }
    Ok(sessions)
}

fn load_session_from_db(
    session_id: &str,
) -> Result<(Value, Vec<(i64, Value)>, HashMap<String, Vec<Value>>)> {
    let db_path = get_db_path();
    let conn = Connection::open(&db_path)?;

    // Load session
    let session_json: Value = conn.query_row(
        "SELECT id, project_id, parent_id, slug, directory, title, version, share_url, summary_additions, summary_deletions, summary_files, summary_diffs, revert, permission, time_created, time_updated, time_compacting, time_archived, workspace_id FROM session WHERE id = ?1",
        [session_id],
        |row| {
            let mut obj = serde_json::Map::new();
            obj.insert("id".to_string(), Value::String(row.get(0)?));
            obj.insert("projectID".to_string(), Value::String(row.get(1)?));
            if let Ok(Some(v)) = row.get::<_, Option<String>>(2) {
                obj.insert("parentID".to_string(), Value::String(v));
            }
            obj.insert("slug".to_string(), Value::String(row.get(3)?));
            obj.insert("directory".to_string(), Value::String(row.get(4)?));
            obj.insert("title".to_string(), Value::String(row.get(5)?));
            obj.insert("version".to_string(), Value::String(row.get(6)?));
            if let Ok(Some(v)) = row.get::<_, Option<String>>(7) {
                obj.insert("shareURL".to_string(), Value::String(v));
            }
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(8) {
                obj.insert("summaryAdditions".to_string(), Value::Number(v.into()));
            }
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(9) {
                obj.insert("summaryDeletions".to_string(), Value::Number(v.into()));
            }
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(10) {
                obj.insert("summaryFiles".to_string(), Value::Number(v.into()));
            }
            if let Ok(Some(v)) = row.get::<_, Option<String>>(11) {
                obj.insert("summaryDiffs".to_string(), Value::String(v));
            }
            if let Ok(Some(v)) = row.get::<_, Option<String>>(12) {
                obj.insert("revert".to_string(), Value::String(v));
            }
            if let Ok(Some(v)) = row.get::<_, Option<String>>(13) {
                obj.insert("permission".to_string(), Value::String(v));
            }
            let created: i64 = row.get(14)?;
            let updated: i64 = row.get(15)?;
            let mut time = serde_json::Map::new();
            time.insert("created".to_string(), Value::Number(created.into()));
            time.insert("updated".to_string(), Value::Number(updated.into()));
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(16) {
                time.insert("compacting".to_string(), Value::Number(v.into()));
            }
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(17) {
                time.insert("archived".to_string(), Value::Number(v.into()));
            }
            obj.insert("time".to_string(), Value::Object(time));
            if let Ok(Some(v)) = row.get::<_, Option<String>>(18) {
                obj.insert("workspaceID".to_string(), Value::String(v));
            }
            Ok(Value::Object(obj))
        },
    )?;

    // Load messages
    let mut stmt = conn.prepare(
        "SELECT id, session_id, time_created, time_updated, data FROM message WHERE session_id = ?1 ORDER BY time_created"
    )?;
    let rows = stmt.query_map([session_id], |row| {
        let msg_id: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let created: i64 = row.get(2)?;
        let _updated: i64 = row.get(3)?;
        let data_str: String = row.get(4)?;
        let mut data: Value = serde_json::from_str(&data_str).unwrap_or_default();
        if let Value::Object(ref mut map) = data {
            map.insert("id".to_string(), Value::String(msg_id));
            map.insert("sessionID".to_string(), Value::String(session_id));
        }
        Ok((created, data))
    })?;

    let mut messages = Vec::new();
    for row in rows {
        if let Ok(r) = row {
            messages.push(r);
        }
    }

    // Load parts
    let mut parts_map: HashMap<String, Vec<Value>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, message_id, session_id, time_created, time_updated, data FROM part WHERE session_id = ?1"
    )?;
    let rows = stmt.query_map([session_id], |row| {
        let part_id: String = row.get(0)?;
        let message_id: String = row.get(1)?;
        let session_id: String = row.get(2)?;
        let _created: i64 = row.get(3)?;
        let _updated: i64 = row.get(4)?;
        let data_str: String = row.get(5)?;
        let mut data: Value = serde_json::from_str(&data_str).unwrap_or_default();
        if let Value::Object(ref mut map) = data {
            map.insert("id".to_string(), Value::String(part_id));
            map.insert("messageID".to_string(), Value::String(message_id.clone()));
            map.insert("sessionID".to_string(), Value::String(session_id));
        }
        Ok((message_id, data))
    })?;

    for row in rows {
        if let Ok((msg_id, part)) = row {
            parts_map.entry(msg_id).or_default().push(part);
        }
    }

    Ok((session_json, messages, parts_map))
}

fn load_session_from_filesystem(
    session_id: &str,
) -> Result<(Value, Vec<(i64, Value)>, HashMap<String, Vec<Value>>)> {
    let storage_dir = get_opencode_dir().join("storage");

    // Find session file
    let mut session_path: Option<PathBuf> = None;
    for entry in WalkDir::new(storage_dir.join("session"))
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
            session_path = Some(path.to_path_buf());
            break;
        }
    }

    let session_path = session_path.context("Session not found in filesystem")?;
    let session_json: Value = serde_json::from_reader(File::open(&session_path)?)?;

    // Load messages from filesystem
    let mut messages = Vec::new();
    let msg_dir = storage_dir.join("message").join(session_id);
    if msg_dir.exists() {
        for entry in std::fs::read_dir(&msg_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let msg_json: Value = serde_json::from_reader(File::open(&path)?)?;
            let created = msg_json
                .get("time")
                .and_then(|v| v.get("created"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            messages.push((created, msg_json));
        }
    }

    // Load parts from filesystem
    let mut parts_map: HashMap<String, Vec<Value>> = HashMap::new();
    let parts_dir = storage_dir.join("part");
    if parts_dir.exists() {
        for entry in std::fs::read_dir(&parts_dir)? {
            let entry = entry?;
            let msg_id = entry.file_name().to_string_lossy().to_string();
            let msg_parts_dir = entry.path();
            if !msg_parts_dir.is_dir() {
                continue;
            }
            for part_entry in std::fs::read_dir(&msg_parts_dir)? {
                let part_entry = part_entry?;
                let part_path = part_entry.path();
                if part_path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let part_json: Value = serde_json::from_reader(File::open(&part_path)?)?;
                parts_map.entry(msg_id.clone()).or_default().push(part_json);
            }
        }
    }

    Ok((session_json, messages, parts_map))
}

fn parse_session_file(path: &Path) -> Option<SessionMeta> {
    let file = File::open(path).ok()?;
    let json: Value = serde_json::from_reader(file).ok()?;

    let session_id = json.get("id").and_then(|v| v.as_str())?.to_string();
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let directory = json
        .get("directory")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let _created = json
        .get("time")
        .and_then(|v| v.get("created"))
        .and_then(|v| v.as_i64());
    let updated = json
        .get("time")
        .and_then(|v| v.get("updated"))
        .and_then(|v| v.as_i64());

    Some(SessionMeta {
        session_id: session_id.clone(),
        title,
        project_dir: directory,
        last_active_at: updated,
        source_path: Some(session_id),
    })
}

fn find_or_create_project(target_dir: &Path) -> Result<String> {
    let db_path = get_db_path();
    if !db_path.exists() {
        // Generate a deterministic project ID from path
        return Ok(generate_project_id(target_dir));
    }

    let conn = Connection::open(&db_path)?;
    let target_dir_str = target_dir.to_string_lossy().to_string();

    // Try to find existing project
    let existing: Result<String, _> = conn.query_row(
        "SELECT id FROM project WHERE worktree = ?1 LIMIT 1",
        [&target_dir_str],
        |row| row.get(0),
    );

    if let Ok(id) = existing {
        return Ok(id);
    }

    // Create new project
    let project_id = generate_project_id(target_dir);
    let now = Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO project (id, worktree, vcs, name, time_created, time_updated, sandboxes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        [
            &project_id,
            &target_dir_str,
            &get_git_remote(target_dir).unwrap_or_default(),
            &target_dir.file_name().and_then(|n| n.to_str()).unwrap_or("project").to_string(),
            &now.to_string(),
            &now.to_string(),
            "{}",
        ],
    )?;

    Ok(project_id)
}

fn generate_project_id(target_dir: &Path) -> String {
    // Use a SHA256 of the absolute path for determinism
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let path_str = target_dir.to_string_lossy().to_string();
    let mut hasher = DefaultHasher::new();
    path_str.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{:016x}{:024x}", hash, hash)
}

fn get_git_remote(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn write_to_db(
    session_id: &str,
    project_id: &str,
    slug: &str,
    directory: &str,
    title: &str,
    now: i64,
    messages: &[(String, i64, Value)],
    parts: &[(String, String, i64, Value)],
) -> Result<()> {
    let db_path = get_db_path();
    let mut conn = Connection::open(&db_path)?;

    let tx = conn.transaction()?;

    // Insert session
    tx.execute(
        "INSERT INTO session (id, project_id, slug, directory, title, version, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        [session_id, project_id, slug, directory, title, OPENCODE_VERSION, &now.to_string(), &now.to_string()],
    )?;

    // Insert messages
    for (msg_id, created, data) in messages {
        // Strip id/sessionID from JSON for DB storage (they're in columns)
        let mut db_data = data.clone();
        if let Value::Object(ref mut map) = db_data {
            map.remove("id");
            map.remove("sessionID");
        }
        let data_str = serde_json::to_string(&db_data)?;
        tx.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            [msg_id, session_id, &created.to_string(), &created.to_string(), &data_str],
        )?;
    }

    // Insert parts
    for (part_id, msg_id, created, data) in parts {
        let mut db_data = data.clone();
        if let Value::Object(ref mut map) = db_data {
            map.remove("id");
            map.remove("messageID");
            map.remove("sessionID");
        }
        let data_str = serde_json::to_string(&db_data)?;
        tx.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            [part_id, msg_id, session_id, &created.to_string(), &created.to_string(), &data_str],
        )?;
    }

    tx.commit()?;
    Ok(())
}

fn write_to_filesystem(
    session_id: &str,
    project_id: &str,
    session_json: &Value,
    messages: &[(String, i64, Value)],
    parts: &[(String, String, i64, Value)],
) -> Result<()> {
    let storage_dir = get_opencode_dir().join("storage");

    // Write session
    let session_dir = storage_dir.join("session").join(project_id);
    std::fs::create_dir_all(&session_dir)?;
    let session_file = session_dir.join(format!("{}.json", session_id));
    std::fs::write(&session_file, serde_json::to_string_pretty(session_json)?)?;

    // Write messages
    let msg_dir = storage_dir.join("message").join(session_id);
    std::fs::create_dir_all(&msg_dir)?;
    for (msg_id, _created, data) in messages {
        let msg_file = msg_dir.join(format!("{}.json", msg_id));
        std::fs::write(&msg_file, serde_json::to_string_pretty(data)?)?;
    }

    // Write parts
    let parts_base = storage_dir.join("part");
    for (part_id, msg_id, _created, data) in parts {
        let part_dir = parts_base.join(msg_id);
        std::fs::create_dir_all(&part_dir)?;
        let part_file = part_dir.join(format!("{}.json", part_id));
        std::fs::write(&part_file, serde_json::to_string_pretty(data)?)?;
    }

    Ok(())
}
