use crate::model::{
    ContentBlock, MemorphMessage, MemorphMeta, MemorphRole, MemorphSession, SessionInfo,
    SessionMeta,
};
use crate::provider::{Provider, ProviderCapabilities};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct DeepseekProvider;

const PROVIDER_ID: &str = "deepseek";

impl Provider for DeepseekProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "DeepSeek"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_session_management()
    }

    fn scan_sessions(&self) -> Result<Vec<SessionMeta>> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(Vec::new());
        }
        let conn = Connection::open(&db_path).with_context(|| {
            format!("failed to open DeepSeek state db at {}", db_path.display())
        })?;

        let mut stmt = conn.prepare(
            "SELECT id, preview, cwd, title, created_at, updated_at FROM threads WHERE archived = 0 ORDER BY updated_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let preview: String = row.get(1)?;
            let cwd: String = row.get(2)?;
            let title: Option<String> = row.get(3)?;
            let _created: i64 = row.get(4)?;
            let updated: i64 = row.get(5)?;
            Ok(SessionMeta {
                session_id: id.clone(),
                title: title.or_else(|| {
                    let p = preview.trim();
                    if p.is_empty() {
                        None
                    } else {
                        Some(p.to_string())
                    }
                }),
                project_dir: Some(cwd),
                last_active_at: Some(updated),
                source_path: Some(id),
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

    fn load_session(&self, source_path: &str) -> Result<MemorphSession> {
        let db_path = get_state_db_path();
        let conn = Connection::open(&db_path).with_context(|| {
            format!("failed to open DeepSeek state db at {}", db_path.display())
        })?;

        // Load thread metadata
        let thread = conn
            .query_row(
                "SELECT id, preview, cwd, title, created_at, updated_at, model_provider FROM threads WHERE id = ?1",
                [source_path],
                |row| {
                    Ok(ThreadRow {
                        id: row.get(0)?,
                        preview: row.get(1)?,
                        cwd: row.get(2)?,
                        title: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        model_provider: row.get(6)?,
                    })
                },
            )
            .optional()
            .context("failed to read thread")?;

        let thread = thread.with_context(|| format!("thread not found: {}", source_path))?;

        // Load messages
        let mut stmt = conn.prepare(
            "SELECT id, role, content, item_json, created_at FROM messages WHERE thread_id = ?1 ORDER BY created_at ASC"
        )?;
        let rows = stmt.query_map([source_path], |row| {
            Ok(MessageRow {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                item_json: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            let msg = row?;
            if msg.role == "history" {
                // Skip internal history markers (fork/resume metadata)
                continue;
            }
            let role = match msg.role.as_str() {
                "user" => MemorphRole::User,
                "assistant" => MemorphRole::Assistant,
                "tool" => MemorphRole::Tool,
                "system" => MemorphRole::System,
                "developer" => MemorphRole::Developer,
                _ => MemorphRole::User,
            };

            let ts = chrono::DateTime::from_timestamp(msg.created_at, 0).unwrap_or_else(Utc::now);

            let mut content_blocks = Vec::new();

            // Try to parse item_json for structured content (tool calls, etc.)
            if let Some(item_str) = &msg.item_json {
                if let Ok(item) = serde_json::from_str::<Value>(item_str) {
                    // If item has tool-related fields, convert accordingly
                    if let Some(tool_name) = item.get("tool_name").and_then(|v| v.as_str()) {
                        let tool_id = item
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| msg.id.to_string());
                        content_blocks.push(ContentBlock::ToolUse {
                            id: tool_id,
                            name: tool_name.to_string(),
                            input: item.get("arguments").cloned(),
                        });
                    } else if let Some(output) = item.get("output").and_then(|v| v.as_str()) {
                        let tool_use_id = item
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| msg.id.to_string());
                        let is_error = item.get("is_error").and_then(|v| v.as_bool());
                        content_blocks.push(ContentBlock::ToolResult {
                            tool_use_id,
                            content: output.to_string(),
                            is_error,
                        });
                    } else {
                        // Fallback: treat as text with the original content
                        content_blocks.push(ContentBlock::text(msg.content.clone()));
                    }
                } else {
                    content_blocks.push(ContentBlock::text(msg.content.clone()));
                }
            } else {
                content_blocks.push(ContentBlock::text(msg.content.clone()));
            }

            let mut extra = serde_json::Map::new();
            if let Ok(item) = msg
                .item_json
                .as_deref()
                .map_or(Ok(Value::Null), serde_json::from_str::<Value>)
            {
                if !item.is_null() {
                    extra.insert("item".to_string(), item);
                }
            }

            messages.push(MemorphMessage {
                id: msg.id.to_string(),
                role,
                content: content_blocks,
                timestamp: ts,
                metadata: Some(crate::model::MessageMetadata {
                    source: Some(crate::model::SourceMetadata {
                        provider: thread.model_provider.clone(),
                        original_id: Some(msg.id.to_string()),
                        original_role: Some(msg.role.clone()),
                    }),
                    model: None,
                    usage: None,
                    extra: Value::Object(extra),
                }),
                parent_id: None,
                turn_index: None,
            });
        }

        let meta = MemorphMeta {
            version: "1.0".to_string(),
            converted_from: PROVIDER_ID.to_string(),
            converted_at: Utc::now(),
            memorph_version: env!("CARGO_PKG_VERSION").to_string(),
            source_session_id: thread.id.clone(),
            source_provider: PROVIDER_ID.to_string(),
            converted_by: Some("memorph-cli".to_string()),
        };

        let session = SessionInfo {
            id: thread.id,
            title: thread.title.or_else(|| {
                let p = thread.preview.trim();
                if p.is_empty() {
                    None
                } else {
                    Some(p.to_string())
                }
            }),
            project_dir: Some(thread.cwd),
            created_at: chrono::DateTime::from_timestamp(thread.created_at, 0),
            last_active_at: chrono::DateTime::from_timestamp(thread.updated_at, 0),
            tags: None,
        };

        Ok(MemorphSession {
            meta,
            session,
            messages,
        })
    }

    fn write_session(&self, session: &MemorphSession, target_dir: &Path) -> Result<String> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            anyhow::bail!(
                "DeepSeek state database does not exist at {}. Please launch DeepSeek TUI once to initialize storage before importing.",
                db_path.display()
            );
        }
        let mut conn = Connection::open(&db_path).with_context(|| {
            format!("failed to open DeepSeek state db at {}", db_path.display())
        })?;

        let thread_id = format!("thread-{}", Uuid::new_v4());
        let now = Utc::now().timestamp();
        let cwd = target_dir.to_string_lossy().to_string();
        let title = session
            .session
            .title
            .clone()
            .unwrap_or_else(|| "Imported session".to_string());
        let preview = title.clone();

        let tx = conn.transaction()?;

        tx.execute(
            "INSERT INTO threads (id, preview, ephemeral, model_provider, created_at, updated_at, status, cwd, cli_version, source, title, archived) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            [
                &thread_id,
                &preview,
                "0",
                "deepseek",
                &now.to_string(),
                &now.to_string(),
                "idle",
                &cwd,
                env!("CARGO_PKG_VERSION"),
                "interactive",
                &title,
                "0",
            ],
        )
        .context("failed to insert thread")?;

        for msg in &session.messages {
            let role_str = match msg.role {
                MemorphRole::User => "user",
                MemorphRole::Assistant => "assistant",
                MemorphRole::Tool => "assistant",
                MemorphRole::System => "system",
                MemorphRole::Developer => "developer",
            };

            let content = msg
                .content
                .iter()
                .map(|block| match block {
                    ContentBlock::Text { text } => text.clone(),
                    ContentBlock::Thinking { thinking, .. } => thinking.clone(),
                    ContentBlock::ToolUse { id, name, input } => {
                        format!(
                            "[Tool: {} id={}]\n{}",
                            name,
                            id,
                            input.as_ref().map(|v| v.to_string()).unwrap_or_default()
                        )
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => {
                        format!("[Tool Result: {}]\n{}", tool_use_id, content)
                    }
                    ContentBlock::Image { mime_type, data } => {
                        format!("[Image: {} len={}]", mime_type, data.len())
                    }
                    ContentBlock::File { path, .. } => {
                        format!("[File: {}]", path)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            let item_json = if msg
                .metadata
                .as_ref()
                .map_or(false, |m| !m.extra.is_null() && m.extra != Value::Null)
            {
                msg.metadata.as_ref().and_then(|m| {
                    if m.extra.is_object() && m.extra.as_object().map_or(false, |o| !o.is_empty()) {
                        serde_json::to_string(&m.extra).ok()
                    } else {
                        None
                    }
                })
            } else {
                None
            };

            let msg_created = msg.timestamp.timestamp();

            tx.execute(
                "INSERT INTO messages (thread_id, role, content, item_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                [
                    &thread_id,
                    role_str,
                    &content,
                    item_json.as_deref().unwrap_or("null"),
                    &msg_created.to_string(),
                ],
            )
            .context("failed to insert message")?;
        }

        tx.commit()?;

        // Append to session_index.jsonl
        append_session_index(&thread_id, Some(&title), now, None)?;

        Ok(thread_id)
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(());
        }
        let conn = Connection::open(&db_path)?;
        conn.execute("DELETE FROM messages WHERE thread_id = ?1", [session_id])?;
        conn.execute("DELETE FROM checkpoints WHERE thread_id = ?1", [session_id])?;
        conn.execute(
            "DELETE FROM thread_dynamic_tools WHERE thread_id = ?1",
            [session_id],
        )?;
        conn.execute("DELETE FROM threads WHERE id = ?1", [session_id])?;
        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(());
        }
        let conn = Connection::open(&db_path)?;
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE threads SET title = ?1, preview = ?1, updated_at = ?2 WHERE id = ?3",
            [new_title, &now.to_string(), session_id],
        )?;

        // Also update session_index.jsonl
        append_session_index(session_id, Some(new_title), now, None)?;
        Ok(())
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("deepseek resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(0);
        }
        let conn = Connection::open(&db_path)?;
        let mut total: u64 = 0;

        // Thread row size
        if let Ok(size) = conn.query_row(
            "SELECT COALESCE(length(id), 0) + COALESCE(length(preview), 0) + COALESCE(length(cwd), 0) + COALESCE(length(title), 0) + COALESCE(length(status), 0) + COALESCE(length(model_provider), 0) + COALESCE(length(cli_version), 0) + COALESCE(length(source), 0) FROM threads WHERE id = ?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        ) {
            total += size as u64;
        }

        // Messages size
        let mut stmt = conn.prepare("SELECT COALESCE(length(role), 0) + COALESCE(length(content), 0) + COALESCE(length(item_json), 0) FROM messages WHERE thread_id = ?1")?;
        let rows = stmt.query_map([session_id], |row| row.get::<_, i64>(0))?;
        for row in rows {
            if let Ok(size) = row {
                total += size as u64;
            }
        }

        Ok(total)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_state_db_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".deepseek").join("state.db"))
        .unwrap_or_else(|| PathBuf::from(".deepseek").join("state.db"))
}

#[derive(Debug)]
struct ThreadRow {
    id: String,
    preview: String,
    cwd: String,
    title: Option<String>,
    created_at: i64,
    updated_at: i64,
    model_provider: String,
}

#[derive(Debug)]
struct MessageRow {
    id: i64,
    role: String,
    content: String,
    item_json: Option<String>,
    created_at: i64,
}

fn append_session_index(
    thread_id: &str,
    thread_name: Option<&str>,
    updated_at: i64,
    rollout_path: Option<&Path>,
) -> Result<()> {
    let index_path = dirs::home_dir()
        .map(|h| h.join(".deepseek").join("session_index.jsonl"))
        .unwrap_or_else(|| PathBuf::from(".deepseek").join("session_index.jsonl"));

    if let Some(parent) = index_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let entry = serde_json::json!({
        "thread_id": thread_id,
        "thread_name": thread_name,
        "updated_at": updated_at,
        "rollout_path": rollout_path.map(|p| p.to_string_lossy().to_string()),
    });

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&index_path)?;
    use std::io::Write;
    writeln!(file, "{}", entry)?;
    Ok(())
}
