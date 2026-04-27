use crate::model::{ContentBlock, MemorphMessage, MemorphMeta, MemorphSession, SessionInfo};
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
        let line = line
            .with_context(|| format!("Failed to read line {} from {}", idx + 1, path.display()))?;
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(&line).with_context(|| {
            format!(
                "Failed to parse JSON at line {} in {}",
                idx + 1,
                path.display()
            )
        })?;

        let line_type = value.get("type").and_then(|v| v.as_str());

        match line_type {
            Some("meta") => {
                let m: MemorphMeta =
                    serde_json::from_value(value.get("_memorph").cloned().unwrap_or(Value::Null))?;
                let s: SessionInfo =
                    serde_json::from_value(value.get("session").cloned().unwrap_or(Value::Null))?;
                meta = Some(m);
                session_info = Some(s);
            }
            Some("message") => {
                let msg = parse_message_line(value).with_context(|| {
                    format!(
                        "Failed to parse message at line {} in {}",
                        idx + 1,
                        path.display()
                    )
                })?;
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

pub fn write_markdown(path: &Path, session: &MemorphSession) -> Result<()> {
    let mut out = String::new();
    let title = session
        .session
        .title
        .as_deref()
        .unwrap_or("Untitled Session");
    out.push_str("# ");
    out.push_str(&escape_markdown_text(title));
    out.push_str("\n\n");
    out.push_str("| Field | Value |\n|---|---|\n");
    out.push_str(&format!("| Session ID | `{}` |\n", session.session.id));
    out.push_str(&format!(
        "| Source Provider | `{}` |\n",
        session.meta.source_provider
    ));
    if let Some(project_dir) = &session.session.project_dir {
        out.push_str(&format!("| Workspace | `{}` |\n", project_dir));
    }
    out.push_str(&format!("| Messages | {} |\n\n", session.messages.len()));

    for message in &session.messages {
        out.push_str("## ");
        out.push_str(&message.role.to_string());
        out.push_str(" - ");
        out.push_str(&message.timestamp.to_rfc3339());
        out.push_str("\n\n");
        for block in &message.content {
            out.push_str(&content_block_markdown(block));
            out.push_str("\n\n");
        }
    }

    out.push_str("---\n\n");
    out.push_str("<!-- memorph-session-json -->\n\n");
    out.push_str("```json memorph-session-json\n");
    out.push_str(&serde_json::to_string_pretty(session)?);
    out.push_str("\n```\n");

    std::fs::write(path, out)
        .with_context(|| format!("Failed to write markdown file: {}", path.display()))
}

pub fn write_html(path: &Path, session: &MemorphSession) -> Result<()> {
    let title = session
        .session
        .title
        .as_deref()
        .unwrap_or("Untitled Session");
    let mut out = String::new();
    out.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    out.push_str("<title>");
    out.push_str(&html_escape(title));
    out.push_str("</title><style>body{font-family:ui-sans-serif,system-ui;margin:32px;line-height:1.55;color:#111}article{max-width:920px;margin:auto}pre{white-space:pre-wrap;border:1px solid #111;padding:12px}code{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}.meta{display:grid;grid-template-columns:max-content 1fr;gap:6px 12px;border:1px solid #111;padding:12px}.message{border-top:1px solid #111;padding-top:18px;margin-top:18px}.role{text-transform:uppercase;font-weight:700}</style></head><body><article>");
    out.push_str("<h1>");
    out.push_str(&html_escape(title));
    out.push_str("</h1><section class=\"meta\"><strong>Session ID</strong><code>");
    out.push_str(&html_escape(&session.session.id));
    out.push_str("</code><strong>Source Provider</strong><code>");
    out.push_str(&html_escape(&session.meta.source_provider));
    out.push_str("</code>");
    if let Some(project_dir) = &session.session.project_dir {
        out.push_str("<strong>Workspace</strong><code>");
        out.push_str(&html_escape(project_dir));
        out.push_str("</code>");
    }
    out.push_str("<strong>Messages</strong><span>");
    out.push_str(&session.messages.len().to_string());
    out.push_str("</span></section>");

    for message in &session.messages {
        out.push_str("<section class=\"message\"><p><span class=\"role\">");
        out.push_str(&html_escape(&message.role.to_string()));
        out.push_str("</span> <time>");
        out.push_str(&html_escape(&message.timestamp.to_rfc3339()));
        out.push_str("</time></p>");
        for block in &message.content {
            out.push_str(&content_block_html(block));
        }
        out.push_str("</section>");
    }

    out.push_str("<script id=\"memorph-session-json\" type=\"application/json\">");
    out.push_str(&html_escape(&serde_json::to_string(session)?));
    out.push_str("</script></article></body></html>\n");

    std::fs::write(path, out)
        .with_context(|| format!("Failed to write html file: {}", path.display()))
}

pub fn read_markdown(path: &Path) -> Result<MemorphSession> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read markdown file: {}", path.display()))?;
    let json = extract_markdown_session_json(&raw)
        .context("Markdown file does not contain a memorph-session-json block")?;
    serde_json::from_str(json).with_context(|| {
        format!(
            "Failed to parse embedded session JSON in {}",
            path.display()
        )
    })
}

pub fn read_html(path: &Path) -> Result<MemorphSession> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read html file: {}", path.display()))?;
    let json = extract_html_session_json(&raw)
        .context("HTML file does not contain a memorph-session-json script block")?;
    let unescaped = html_unescape(json);
    serde_json::from_str(&unescaped).with_context(|| {
        format!(
            "Failed to parse embedded session JSON in {}",
            path.display()
        )
    })
}

/// Manually parse a message line, handling the content.blocks wrapper structure
fn parse_message_line(value: Value) -> anyhow::Result<MemorphMessage> {
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
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

    let parent_id = value
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let turn_index = value
        .get("turn_index")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

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

fn content_block_markdown(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text { text } => text.clone(),
        ContentBlock::Thinking { thinking, .. } => {
            format!("```text\n[Thinking]\n{}\n```", thinking)
        }
        ContentBlock::ToolUse { id, name, input } => format!(
            "```json\n{{\n  \"tool_use_id\": {},\n  \"name\": {},\n  \"input\": {}\n}}\n```",
            serde_json::to_string(id).unwrap_or_else(|_| "null".to_string()),
            serde_json::to_string(name).unwrap_or_else(|_| "null".to_string()),
            input
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_else(|| "null".to_string())
        ),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => format!(
            "```text\n[Tool Result: {}{}]\n{}\n```",
            tool_use_id,
            if is_error.unwrap_or(false) {
                " error"
            } else {
                ""
            },
            content
        ),
        ContentBlock::Image { mime_type, .. } => format!("[Image: {}]", mime_type),
        ContentBlock::File { path, content } => content
            .as_ref()
            .map(|value| format!("### File `{}`\n\n```text\n{}\n```", path, value))
            .unwrap_or_else(|| format!("[File: {}]", path)),
    }
}

fn content_block_html(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text { text } => {
            format!("<p>{}</p>", html_escape(text).replace('\n', "<br>"))
        }
        ContentBlock::Thinking { thinking, .. } => {
            format!("<pre>[Thinking]\n{}</pre>", html_escape(thinking))
        }
        ContentBlock::ToolUse { id, name, input } => format!(
            "<pre>[Tool Use: {} ({})]\n{}</pre>",
            html_escape(name),
            html_escape(id),
            html_escape(
                &input
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_else(|| "null".to_string())
            )
        ),
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => format!(
            "<pre>[Tool Result: {}{}]\n{}</pre>",
            html_escape(tool_use_id),
            if is_error.unwrap_or(false) {
                " error"
            } else {
                ""
            },
            html_escape(content)
        ),
        ContentBlock::Image { mime_type, data } => format!(
            "<p><img alt=\"{}\" src=\"data:{};base64,{}\"></p>",
            html_escape(mime_type),
            html_escape(mime_type),
            html_escape(data)
        ),
        ContentBlock::File { path, content } => content
            .as_ref()
            .map(|value| {
                format!(
                    "<h3>File <code>{}</code></h3><pre>{}</pre>",
                    html_escape(path),
                    html_escape(value)
                )
            })
            .unwrap_or_else(|| format!("<p>[File: {}]</p>", html_escape(path))),
    }
}

fn extract_markdown_session_json(raw: &str) -> Option<&str> {
    let marker = "```json memorph-session-json";
    let start = raw.find(marker)? + marker.len();
    let rest = raw[start..].strip_prefix('\n').unwrap_or(&raw[start..]);
    let end = rest.find("\n```")?;
    Some(&rest[..end])
}

fn extract_html_session_json(raw: &str) -> Option<&str> {
    let marker = "<script id=\"memorph-session-json\" type=\"application/json\">";
    let start = raw.find(marker)? + marker.len();
    let rest = &raw[start..];
    let end = rest.find("</script>")?;
    Some(&rest[..end])
}

fn escape_markdown_text(value: &str) -> String {
    value.replace('\n', " ")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}
