use crate::model::{
    ContentBlock, MemorphMeta, MemorphMessage, MemorphRole, MemorphSession, SessionInfo, SessionMeta,
};
use crate::provider::Provider;
use crate::utils::{encode_project_dir, extract_text, parse_timestamp_to_ms, path_basename, truncate_summary};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct ClaudeProvider;

const PROVIDER_ID: &str = "claude";
const TITLE_MAX_CHARS: usize = 80;

impl Provider for ClaudeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Claude Code"
    }

    fn scan_sessions(&self) -> Result<Vec<SessionMeta>> {
        let root = get_claude_config_dir().join("projects");
        if !root.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in WalkDir::new(&root)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // Skip agent sub-sessions
            if path.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("agent-"))
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(meta) = parse_session(path) {
                sessions.push(meta);
            }
        }

        Ok(sessions)
    }

    fn load_session(&self, source_path: &str) -> Result<MemorphSession> {
        let path = Path::new(source_path);
        let file = File::open(path)
            .with_context(|| format!("Failed to open Claude session: {}", path.display()))?;
        let reader = BufReader::new(file);

        let mut messages = Vec::new();
        let mut session_title: Option<String> = None;
        let mut project_dir: Option<String> = None;
        let mut session_id: Option<String> = None;
        let mut created_at: Option<i64> = None;
        let mut last_active_at: Option<i64> = None;

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Extract session-level metadata from early lines
            if session_id.is_none() {
                session_id = value.get("sessionId").and_then(|v| v.as_str()).map(|s| s.to_string());
            }
            if project_dir.is_none() {
                project_dir = value.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string());
            }
            if created_at.is_none() {
                created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
            }
            if last_active_at.is_none() {
                if let Some(ts) = value.get("timestamp").and_then(parse_timestamp_to_ms) {
                    last_active_at = Some(ts);
                }
            }
            // custom-title
            if value.get("type").and_then(|v| v.as_str()) == Some("custom-title") {
                if let Some(title) = value.get("customTitle").and_then(|v| v.as_str()) {
                    session_title = Some(title.to_string());
                }
            }

            // Skip meta lines
            if value.get("isMeta").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            if value.get("type").and_then(|v| v.as_str()) == Some("permission-mode") {
                continue;
            }
            if value.get("type").and_then(|v| v.as_str()) == Some("file-history-snapshot") {
                continue;
            }

            let msg_type = value.get("type").and_then(|v| v.as_str());
            let message = match value.get("message") {
                Some(m) => m,
                None => continue,
            };

            let role = match msg_type {
                Some("user") => {
                    let mut r = MemorphRole::User;
                    // Check for tool_result-only content -> reclassify as tool
                    if let Some(Value::Array(items)) = message.get("content") {
                        let all_tool_results = !items.is_empty()
                            && items.iter().all(|item| {
                                item.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                            });
                        if all_tool_results {
                            r = MemorphRole::Tool;
                        }
                    }
                    r
                }
                Some("assistant") => MemorphRole::Assistant,
                _ => {
                    if let Some(role_str) = message.get("role").and_then(|v| v.as_str()) {
                        match role_str {
                            "user" => MemorphRole::User,
                            "assistant" => MemorphRole::Assistant,
                            _ => continue,
                        }
                    } else {
                        continue;
                    }
                }
            };

            // Extract content blocks
            let content = match message.get("content") {
                Some(Value::Array(arr)) => {
                    arr.iter().filter_map(parse_content_block).collect()
                }
                Some(Value::String(s)) => {
                    vec![ContentBlock::text(s)]
                }
                _ => continue,
            };

            if content.is_empty() {
                continue;
            }

            let ts = value
                .get("timestamp")
                .and_then(parse_timestamp_to_ms)
                .map(|ms| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now))
                .unwrap_or_else(Utc::now);

            let msg_id = value
                .get("uuid")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            let parent_id = value
                .get("parentUuid")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            messages.push(MemorphMessage {
                id: msg_id,
                role,
                content,
                timestamp: ts,
                metadata: None,
                parent_id,
                turn_index: None,
            });
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
            title: session_title,
            project_dir,
            created_at: created_at.map(|ms| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now)),
            last_active_at: last_active_at.map(|ms| chrono::DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now)),
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
        let encoded_dir = encode_project_dir(&target_dir.to_string_lossy());
        let claude_projects_dir = get_claude_config_dir().join("projects").join(&encoded_dir);
        std::fs::create_dir_all(&claude_projects_dir)?;

        let file_path = claude_projects_dir.join(format!("{}.jsonl", session_id));
        let sidecar_dir = claude_projects_dir.join(&session_id);
        std::fs::create_dir_all(&sidecar_dir)?;
        std::fs::create_dir_all(sidecar_dir.join("tool-results"))?;
        std::fs::create_dir_all(sidecar_dir.join("subagents"))?;

        let mut file = File::create(&file_path)?;

        // Write permission mode line
        let perm_line = serde_json::json!({
            "type": "permission-mode",
            "permissionMode": "bypassPermissions",
            "sessionId": session_id,
        });
        writeln!(file, "{}", serde_json::to_string(&perm_line)?)?;

        // Write custom-title line so Claude Code shows the correct title
        let title = session.session.title.clone().or_else(|| {
            session.messages.iter()
                .find(|m| m.role == MemorphRole::User)
                .and_then(|m| m.content.iter().find_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                }))
        }).unwrap_or_else(|| "Imported session".to_string());
        let title_line = serde_json::json!({
            "type": "custom-title",
            "customTitle": title,
            "sessionId": session_id,
        });
        writeln!(file, "{}", serde_json::to_string(&title_line)?)?;

        let project_dir_str = target_dir.to_string_lossy().to_string();
        let version = "2.1.116"; // TODO: detect actual version
        let git_branch = get_git_branch(target_dir).unwrap_or_else(|| "main".to_string());

        let mut prev_uuid: Option<String> = None;

        for (_idx, msg) in session.messages.iter().enumerate() {
            let msg_uuid = Uuid::new_v4().to_string();
            let timestamp = msg.timestamp.to_rfc3339();

            match msg.role {
                MemorphRole::User | MemorphRole::Tool | MemorphRole::System | MemorphRole::Developer => {
                    // Build content: if tool role, wrap as tool_result block; otherwise extract text
                    let content = if msg.role == MemorphRole::Tool {
                        // Try to find matching tool_use_id from metadata or use generic
                        let tool_use_id = msg
                            .metadata
                            .as_ref()
                            .and_then(|m| m.extra.get("tool_use_id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("toolu_{}", Uuid::new_v4().to_string().replace("-", "").chars().take(20).collect::<String>()));
                        let text = msg.content.iter()
                            .filter_map(|b| match b {
                                ContentBlock::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        serde_json::json!([{
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": text,
                        }])
                    } else {
                        let text = msg.content.iter()
                            .filter_map(|b| match b {
                                ContentBlock::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        serde_json::Value::String(text)
                    };

                    let line = serde_json::json!({
                        "parentUuid": prev_uuid,
                        "isSidechain": false,
                        "promptId": Uuid::new_v4().to_string(),
                        "type": "user",
                        "message": {
                            "role": "user",
                            "content": content,
                        },
                        "uuid": msg_uuid,
                        "timestamp": timestamp,
                        "permissionMode": "bypassPermissions",
                        "userType": "external",
                        "entrypoint": "cli",
                        "cwd": project_dir_str,
                        "sessionId": session_id,
                        "version": version,
                        "gitBranch": git_branch,
                    });
                    writeln!(file, "{}", serde_json::to_string(&line)?)?;
                }
                MemorphRole::Assistant => {
                    let mut content_blocks = Vec::new();
                    let mut has_tool_use = false;

                    for block in &msg.content {
                        match block {
                            ContentBlock::Text { text } => {
                                content_blocks.push(serde_json::json!({
                                    "type": "text",
                                    "text": text,
                                }));
                            }
                            ContentBlock::Thinking { thinking, signature } => {
                                let mut t = serde_json::json!({
                                    "type": "thinking",
                                    "thinking": thinking,
                                });
                                if let Some(sig) = signature {
                                    t["signature"] = serde_json::Value::String(sig.clone());
                                }
                                content_blocks.push(t);
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                has_tool_use = true;
                                let mut t = serde_json::json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                });
                                if let Some(inp) = input {
                                    t["input"] = inp.clone();
                                }
                                content_blocks.push(t);
                            }
                            ContentBlock::ToolResult { tool_use_id, content, .. } => {
                                // tool_result in assistant? skip, should be in user message
                                let _ = (tool_use_id, content);
                            }
                            _ => {}
                        }
                    }

                    // Fallback: if no blocks, add empty text
                    if content_blocks.is_empty() {
                        content_blocks.push(serde_json::json!({
                            "type": "text",
                            "text": "",
                        }));
                    }

                    let stop_reason = if has_tool_use { "tool_use" } else { "end_turn" };

                    let line = serde_json::json!({
                        "parentUuid": prev_uuid,
                        "isSidechain": false,
                        "message": {
                            "id": format!("msg_{}", Uuid::new_v4().to_string().replace("-", "").chars().take(20).collect::<String>()),
                            "type": "message",
                            "role": "assistant",
                            "content": content_blocks,
                            "model": msg.metadata.as_ref().and_then(|m| m.model.clone()).unwrap_or_else(|| "claude-sonnet-4-1".to_string()),
                            "stop_reason": stop_reason,
                            "usage": msg.metadata.as_ref().and_then(|m| m.usage.as_ref().map(|u| {
                                serde_json::json!({
                                    "input_tokens": u.input_tokens,
                                    "output_tokens": u.output_tokens,
                                })
                            })).unwrap_or(serde_json::json!({})),
                        },
                        "type": "assistant",
                        "uuid": msg_uuid,
                        "timestamp": timestamp,
                        "userType": "external",
                        "entrypoint": "cli",
                        "cwd": project_dir_str,
                        "sessionId": session_id,
                        "version": version,
                        "gitBranch": git_branch,
                    });
                    writeln!(file, "{}", serde_json::to_string(&line)?)?;
                }
            }

            prev_uuid = Some(msg_uuid);
        }

        Ok(session_id)
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let projects_dir = get_claude_config_dir().join("projects");
        if !projects_dir.exists() {
            anyhow::bail!("Claude projects directory not found");
        }

        // Find the session file
        let mut found = false;
        for entry in WalkDir::new(&projects_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                // Remove the JSONL file
                std::fs::remove_file(path)
                    .with_context(|| format!("Failed to remove session file: {}", path.display()))?;
                // Remove sidecar directory if exists
                if let Some(parent) = path.parent() {
                    let sidecar = parent.join(session_id);
                    if sidecar.exists() {
                        std::fs::remove_dir_all(&sidecar)
                            .with_context(|| format!("Failed to remove sidecar directory: {}", sidecar.display()))?;
                    }
                }
                found = true;
                break;
            }
        }

        if !found {
            anyhow::bail!("Claude session not found: {}", session_id);
        }

        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let projects_dir = get_claude_config_dir().join("projects");
        if !projects_dir.exists() {
            anyhow::bail!("Claude projects directory not found");
        }

        // Find the session file
        let mut found_path: Option<PathBuf> = None;
        for entry in WalkDir::new(&projects_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                found_path = Some(path.to_path_buf());
                break;
            }
        }

        let path = found_path.with_context(|| format!("Claude session not found: {}", session_id))?;

        // Rewrite the JSONL with updated custom-title line
        let content = std::fs::read_to_string(&path)?;
        let mut new_lines = Vec::new();
        let mut title_updated = false;

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut value: Value = serde_json::from_str(line)?;
            if value.get("type").and_then(|v| v.as_str()) == Some("custom-title") {
                if let Value::Object(ref mut map) = value {
                    map.insert("customTitle".to_string(), Value::String(new_title.to_string()));
                    title_updated = true;
                }
                new_lines.push(serde_json::to_string(&value)?);
            } else {
                new_lines.push(line.to_string());
            }
        }

        // If no custom-title line found, prepend one after permission-mode
        if !title_updated {
            let title_line = serde_json::json!({
                "type": "custom-title",
                "customTitle": new_title,
                "sessionId": session_id,
            });
            // Insert after permission-mode line if present, else at beginning
            let mut inserted = false;
            for (i, line) in new_lines.iter().enumerate() {
                let v: Value = serde_json::from_str(line)?;
                if v.get("type").and_then(|v| v.as_str()) == Some("permission-mode") {
                    new_lines.insert(i + 1, serde_json::to_string(&title_line)?);
                    inserted = true;
                    break;
                }
            }
            if !inserted {
                new_lines.insert(0, serde_json::to_string(&title_line)?);
            }
        }

        std::fs::write(&path, new_lines.join("\n") + "\n")?;
        Ok(())
    }
}

fn get_claude_config_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".claude"))
        .unwrap_or_else(|| PathBuf::from(".claude"))
}

fn get_git_branch(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn parse_content_block(value: &Value) -> Option<ContentBlock> {
    let block_type = value.get("type").and_then(|v| v.as_str())?;
    match block_type {
        "text" => {
            let text = value.get("text").and_then(|v| v.as_str())?;
            Some(ContentBlock::text(text))
        }
        "thinking" => {
            let thinking = value.get("thinking").and_then(|v| v.as_str())?;
            let signature = value.get("signature").and_then(|v| v.as_str()).map(|s| s.to_string());
            Some(ContentBlock::Thinking {
                thinking: thinking.to_string(),
                signature,
            })
        }
        "tool_use" => {
            let id = value.get("id").and_then(|v| v.as_str())?;
            let name = value.get("name").and_then(|v| v.as_str())?;
            let input = value.get("input").cloned();
            Some(ContentBlock::ToolUse {
                id: id.to_string(),
                name: name.to_string(),
                input,
            })
        }
        "tool_result" => {
            let tool_use_id = value.get("tool_use_id")
                .and_then(|v| v.as_str())?;
            let content = value.get("content")
                .map(|v| v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string()))
                .unwrap_or_default();
            let is_error = value.get("is_error").and_then(|v| v.as_bool());
            Some(ContentBlock::ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content,
                is_error,
            })
        }
        _ => None,
    }
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().filter_map(Result::ok).collect();

    let head = lines.iter().take(20).collect::<Vec<_>>();
    let tail = lines.iter().rev().take(30).collect::<Vec<_>>();

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut first_user_message: Option<String> = None;
    let mut custom_title: Option<String> = None;

    for line in &head {
        let value: Value = serde_json::from_str(line).ok()?;
        if session_id.is_none() {
            session_id = value.get("sessionId").and_then(|v| v.as_str()).map(|s| s.to_string());
        }
        if project_dir.is_none() {
            project_dir = value.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string());
        }
        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if first_user_message.is_none() {
            let is_user = value.get("type").and_then(|v| v.as_str()) == Some("user")
                || value
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|v| v.as_str())
                    == Some("user");
            if is_user {
                if let Some(message) = value.get("message") {
                    let text = extract_text(message.get("content")?);
                    let trimmed = text.trim();
                    if !trimmed.is_empty()
                        && !trimmed.contains("<local-command-caveat>")
                        && !trimmed.starts_with("<command-name>")
                    {
                        first_user_message = Some(trimmed.to_string());
                    }
                }
            }
        }
        if session_id.is_some()
            && project_dir.is_some()
            && created_at.is_some()
            && first_user_message.is_some()
        {
            break;
        }
    }

    let mut last_active_at: Option<i64> = None;
    let mut summary: Option<String> = None;

    for line in &tail {
        let value: Value = serde_json::from_str(line).ok()?;
        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if custom_title.is_none()
            && value.get("type").and_then(|v| v.as_str()) == Some("custom-title")
        {
            custom_title = value
                .get("customTitle")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if summary.is_none() {
            if value.get("isMeta").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            if let Some(message) = value.get("message") {
                let text = extract_text(message.get("content")?);
                if !text.trim().is_empty() {
                    summary = Some(text);
                }
            }
        }
        if last_active_at.is_some() && summary.is_some() && custom_title.is_some() {
            break;
        }
    }

    let session_id = session_id.or_else(|| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem.to_string())
    })?;

    let title = custom_title
        .map(|t| truncate_summary(&t, TITLE_MAX_CHARS))
        .or_else(|| first_user_message.map(|t| truncate_summary(&t, TITLE_MAX_CHARS)))
        .or_else(|| {
            project_dir
                .as_deref()
                .and_then(path_basename)
                .map(|v| v.to_string())
        });

    let _summary = summary.map(|text| truncate_summary(&text, 160));

    Some(SessionMeta {
        session_id: session_id.clone(),
        title,
        project_dir,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
    })
}
