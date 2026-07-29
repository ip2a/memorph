use crate::session::{
    Artifact, Block, Context, Event, EventKind, Identity, Provenance, Role, Schema, Session,
};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
struct MorphMetaLine {
    #[serde(default)]
    schema: Schema,
    identity: Identity,
    provenance: Provenance,
    #[serde(default)]
    context: Context,
    #[serde(default)]
    artifacts: Vec<Artifact>,
    #[serde(default)]
    extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MorphEventLine {
    event: Event,
}

pub fn read_session(path: &Path) -> Result<Session> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open morph file: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut meta: Option<MorphMetaLine> = None;
    let mut events = Vec::new();

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

        match value.get("type").and_then(|v| v.as_str()) {
            Some("meta") => {
                meta = Some(serde_json::from_value(value).with_context(|| {
                    format!(
                        "Failed to parse meta line {} in {}",
                        idx + 1,
                        path.display()
                    )
                })?);
            }
            Some("event") => {
                let line: MorphEventLine = serde_json::from_value(value).with_context(|| {
                    format!(
                        "Failed to parse event line {} in {}",
                        idx + 1,
                        path.display()
                    )
                })?;
                events.push(line.event);
            }
            _ => {}
        }
    }

    let meta = meta.context("Missing meta line in morph file")?;
    Ok(Session {
        schema: meta.schema,
        identity: meta.identity,
        provenance: meta.provenance,
        context: meta.context,
        events,
        artifacts: meta.artifacts,
        extensions: meta.extensions,
    })
}

pub fn write_session(path: &Path, session: &Session) -> Result<()> {
    let mut file = File::create(path)
        .with_context(|| format!("Failed to create morph file: {}", path.display()))?;

    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "type": "meta",
            "schema": session.schema,
            "identity": session.identity,
            "provenance": session.provenance,
            "context": session.context,
            "artifacts": session.artifacts,
            "extensions": session.extensions,
        }))?
    )?;

    for event in &session.events {
        writeln!(
            file,
            "{}",
            serde_json::to_string(&serde_json::json!({
                "type": "event",
                "event": event,
            }))?
        )?;
    }

    Ok(())
}

pub fn write_markdown(path: &Path, session: &Session) -> Result<()> {
    let mut out = String::new();
    let title = session_title(session);
    out.push_str("# ");
    out.push_str(&escape_markdown_text(title));
    out.push_str("\n\n");
    out.push_str("| Field | Value |\n|---|---|\n");
    out.push_str(&format!(
        "| Canonical ID | `{}` |\n",
        session.identity.canonical_id
    ));
    out.push_str(&format!(
        "| Source Provider | `{}` |\n",
        session.provenance.primary_source.provider_id
    ));
    out.push_str(&format!(
        "| Source Session | `{}` |\n",
        session.provenance.primary_source.session_id
    ));
    if let Some(workspace) = &session.context.workspace_dir {
        out.push_str(&format!("| Workspace | `{}` |\n", workspace));
    }
    out.push_str(&format!("| Events | {} |\n", session.events.len()));
    out.push_str(&format!("| Artifacts | {} |\n\n", session.artifacts.len()));

    for event in &session.events {
        out.push_str("## ");
        out.push_str(event_role_label(event.role));
        out.push_str(" / ");
        out.push_str(event_kind_label(event.kind));
        out.push_str(" - ");
        out.push_str(&event.timestamp.to_rfc3339());
        out.push_str("\n\n");
        for block in &event.blocks {
            out.push_str(&event_block_markdown(block));
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

pub fn write_html(path: &Path, session: &Session) -> Result<()> {
    let title = session_title(session);
    let mut out = String::new();
    out.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    out.push_str("<title>");
    out.push_str(&html_escape(title));
    out.push_str("</title><style>body{font-family:ui-sans-serif,system-ui;margin:32px;line-height:1.55;color:#111}article{max-width:960px;margin:auto}pre{white-space:pre-wrap;border:1px solid #111;padding:12px;overflow:auto}code{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}.meta{display:grid;grid-template-columns:max-content 1fr;gap:6px 12px;border:1px solid #111;padding:12px}.event{border-top:1px solid #111;padding-top:18px;margin-top:18px}.label{text-transform:uppercase;font-weight:700}</style></head><body><article>");
    out.push_str("<h1>");
    out.push_str(&html_escape(title));
    out.push_str("</h1><section class=\"meta\"><strong>Canonical ID</strong><code>");
    out.push_str(&html_escape(&session.identity.canonical_id));
    out.push_str("</code><strong>Source Provider</strong><code>");
    out.push_str(&html_escape(&session.provenance.primary_source.provider_id));
    out.push_str("</code><strong>Source Session</strong><code>");
    out.push_str(&html_escape(&session.provenance.primary_source.session_id));
    out.push_str("</code>");
    if let Some(workspace) = &session.context.workspace_dir {
        out.push_str("<strong>Workspace</strong><code>");
        out.push_str(&html_escape(workspace));
        out.push_str("</code>");
    }
    out.push_str("<strong>Events</strong><span>");
    out.push_str(&session.events.len().to_string());
    out.push_str("</span><strong>Artifacts</strong><span>");
    out.push_str(&session.artifacts.len().to_string());
    out.push_str("</span></section>");

    for event in &session.events {
        out.push_str("<section class=\"event\"><p><span class=\"label\">");
        out.push_str(&html_escape(event_role_label(event.role)));
        out.push_str("</span> / <span class=\"label\">");
        out.push_str(&html_escape(event_kind_label(event.kind)));
        out.push_str("</span> <time>");
        out.push_str(&html_escape(&event.timestamp.to_rfc3339()));
        out.push_str("</time></p>");
        for block in &event.blocks {
            out.push_str(&event_block_html(block));
        }
        out.push_str("</section>");
    }

    out.push_str("<script id=\"memorph-session-json\" type=\"application/json\">");
    out.push_str(&html_escape(&serde_json::to_string(session)?));
    out.push_str("</script></article></body></html>\n");

    std::fs::write(path, out)
        .with_context(|| format!("Failed to write html file: {}", path.display()))
}

pub fn read_markdown(path: &Path) -> Result<Session> {
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

pub fn read_html(path: &Path) -> Result<Session> {
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

fn session_title(session: &Session) -> &str {
    session
        .primary_title()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(&session.identity.canonical_id)
}

fn event_role_label(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::System => "system",
        Role::Developer => "developer",
        _ => "unknown",
    }
}

fn event_kind_label(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Message => "message",
        EventKind::ToolCall => "tool_call",
        EventKind::ToolResult => "tool_result",
        EventKind::Command => "command",
        EventKind::CommandResult => "command_result",
        EventKind::Patch => "patch",
        EventKind::Lifecycle => "lifecycle",
        EventKind::Artifact => "artifact",
        _ => "unknown",
    }
}

fn event_block_markdown(block: &Block) -> String {
    match block {
        Block::Text { text } => text.clone(),
        Block::Thinking { text, .. } => format!("```text\n[Thinking]\n{}\n```", text),
        Block::ToolCall {
            tool_call_id,
            name,
            input,
        } => json_block_markdown(&serde_json::json!({
            "tool_call_id": tool_call_id,
            "name": name,
            "input": input,
        })),
        Block::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => format!(
            "```text\n[Tool Result: {}{}]\n{}\n```",
            tool_call_id,
            if *is_error { " error" } else { "" },
            content
        ),
        Block::Patch {
            summary,
            diff_text,
            files,
            hash,
        } => {
            let mut body = String::new();
            if let Some(summary) = summary {
                body.push_str(summary);
                body.push('\n');
            }
            if !files.is_empty() {
                body.push_str("Files:\n");
                for file in files {
                    body.push_str("- ");
                    body.push_str(file);
                    body.push('\n');
                }
            }
            if let Some(hash) = hash {
                body.push_str("Hash: ");
                body.push_str(hash);
                body.push('\n');
            }
            if let Some(diff_text) = diff_text {
                body.push('\n');
                body.push_str(diff_text);
            }
            format!("```diff\n{}\n```", body.trim_end())
        }
        Block::Command { command, argv, cwd } => json_block_markdown(&serde_json::json!({
            "command": command,
            "argv": argv,
            "cwd": cwd,
        })),
        Block::CommandResult {
            command,
            exit_code,
            stdout,
            stderr,
        } => {
            let mut body = String::new();
            if let Some(command) = command {
                body.push_str("Command: ");
                body.push_str(command);
                body.push('\n');
            }
            if let Some(exit_code) = exit_code {
                body.push_str("Exit: ");
                body.push_str(&exit_code.to_string());
                body.push('\n');
            }
            if let Some(stdout) = stdout {
                body.push_str("\nstdout:\n");
                body.push_str(stdout);
                body.push('\n');
            }
            if let Some(stderr) = stderr {
                body.push_str("\nstderr:\n");
                body.push_str(stderr);
                body.push('\n');
            }
            format!("```text\n{}\n```", body.trim_end())
        }
        Block::File {
            path,
            content,
            mime_type,
        } => match content {
            Some(content) => format!(
                "### File `{}`{}\n\n```text\n{}\n```",
                path,
                mime_type
                    .as_deref()
                    .map(|mime| format!(" ({})", mime))
                    .unwrap_or_default(),
                content
            ),
            None => format!(
                "[File: {}{}]",
                path,
                mime_type
                    .as_deref()
                    .map(|mime| format!(" ({})", mime))
                    .unwrap_or_default()
            ),
        },
        Block::Image {
            mime_type,
            data,
            path,
        } => format!(
            "[Image: {}{}{}]",
            mime_type,
            path.as_deref()
                .map(|value| format!(", path={}", value))
                .unwrap_or_default(),
            if data.is_some() { ", embedded" } else { "" }
        ),
        Block::ProviderPayload { kind, payload } => json_block_markdown(&serde_json::json!({
            "kind": kind,
            "payload": payload,
        })),
        Block::Compressed {
            source_provider_id,
            summary,
            source_event_ids,
            source_event_count,
            archive_ref,
        } => {
            let mut out = String::from("> Compressed session segment\n\n");
            out.push_str(&format!(
                "- Source provider: `{}`\n",
                escape_markdown_text(source_provider_id)
            ));
            let count = source_event_count.unwrap_or(source_event_ids.len());
            if count > 0 {
                out.push_str(&format!(
                    "- Source event count: `{}`\n",
                    escape_markdown_text(&count.to_string())
                ));
            }
            if let Some(archive_ref) = archive_ref {
                out.push_str(&format!(
                    "- Archive: `{}`\n",
                    escape_markdown_text(archive_ref)
                ));
            }
            out.push('\n');
            out.push_str(&escape_markdown_text(summary));
            out
        }
        Block::Other { raw } => json_block_markdown(raw),
        // ponytail: oasf 标了 #[non_exhaustive],未来新增 Block 变体在此兜底为空串。
        _ => String::new(),
    }
}

fn event_block_html(block: &Block) -> String {
    match block {
        Block::Text { text } => format!("<p>{}</p>", html_escape(text).replace('\n', "<br>")),
        Block::Thinking { text, .. } => {
            format!("<pre>[Thinking]\n{}</pre>", html_escape(text))
        }
        Block::ToolCall {
            tool_call_id,
            name,
            input,
        } => json_block_html(&serde_json::json!({
            "tool_call_id": tool_call_id,
            "name": name,
            "input": input,
        })),
        Block::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => format!(
            "<pre>[Tool Result: {}{}]\n{}</pre>",
            html_escape(tool_call_id),
            if *is_error { " error" } else { "" },
            html_escape(content)
        ),
        Block::Patch {
            summary,
            diff_text,
            files,
            hash,
        } => {
            let payload = serde_json::json!({
                "summary": summary,
                "diff_text": diff_text,
                "files": files,
                "hash": hash,
            });
            json_block_html(&payload)
        }
        Block::Command { command, argv, cwd } => json_block_html(&serde_json::json!({
            "command": command,
            "argv": argv,
            "cwd": cwd,
        })),
        Block::CommandResult {
            command,
            exit_code,
            stdout,
            stderr,
        } => json_block_html(&serde_json::json!({
            "command": command,
            "exit_code": exit_code,
            "stdout": stdout,
            "stderr": stderr,
        })),
        Block::File {
            path,
            content,
            mime_type,
        } => match content {
            Some(content) => format!(
                "<h3>File <code>{}</code>{}</h3><pre>{}</pre>",
                html_escape(path),
                mime_type
                    .as_deref()
                    .map(|mime| format!(" ({})", html_escape(mime)))
                    .unwrap_or_default(),
                html_escape(content)
            ),
            None => format!(
                "<p>[File: {}{}]</p>",
                html_escape(path),
                mime_type
                    .as_deref()
                    .map(|mime| format!(" ({})", html_escape(mime)))
                    .unwrap_or_default()
            ),
        },
        Block::Image {
            mime_type,
            data,
            path,
        } => match data {
            Some(data) => format!(
                "<p><img alt=\"{}\" src=\"data:{};base64,{}\"></p>",
                html_escape(mime_type),
                html_escape(mime_type),
                html_escape(data)
            ),
            None => format!(
                "<p>[Image: {}{}]</p>",
                html_escape(mime_type),
                path.as_deref()
                    .map(|value| format!(" path={}", html_escape(value)))
                    .unwrap_or_default()
            ),
        },
        Block::ProviderPayload { kind, payload } => json_block_html(&serde_json::json!({
            "kind": kind,
            "payload": payload,
        })),
        Block::Compressed {
            source_provider_id,
            summary,
            source_event_ids,
            source_event_count,
            archive_ref,
        } => {
            let mut out =
                String::from("<blockquote><p>Compressed session segment</p></blockquote>");
            out.push_str("<p><strong>Source provider:</strong> <code>");
            out.push_str(&html_escape(source_provider_id));
            out.push_str("</code></p>");
            let count = source_event_count.unwrap_or(source_event_ids.len());
            if count > 0 {
                out.push_str("<p><strong>Source event count:</strong> <code>");
                out.push_str(&html_escape(&count.to_string()));
                out.push_str("</code></p>");
            }
            if let Some(archive_ref) = archive_ref {
                out.push_str("<p><strong>Archive:</strong> <code>");
                out.push_str(&html_escape(archive_ref));
                out.push_str("</code></p>");
            }
            out.push_str("<pre>");
            out.push_str(&html_escape(summary));
            out.push_str("</pre>");
            out
        }
        Block::Other { raw } => json_block_html(raw),
        _ => String::new(),
    }
}

fn json_block_markdown(value: &Value) -> String {
    format!(
        "```json\n{}\n```",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string())
    )
}

fn json_block_html(value: &Value) -> String {
    format!(
        "<pre>{}</pre>",
        html_escape(&serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string()))
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Fidelity, ProviderRef};
    use chrono::Utc;
    use tempfile::tempdir;

    fn sample_session() -> Session {
        Session {
            schema: Schema::default(),
            identity: Identity {
                canonical_id: "session-1".to_string(),
                source_title: Some("Session Title".to_string()),
            },
            provenance: Provenance {
                imported_at: Utc::now(),
                imported_by: Some("test".to_string()),
                primary_source: ProviderRef {
                    provider_id: "codex".to_string(),
                    session_id: "source-1".to_string(),
                    source_path: Some("/tmp/source.jsonl".to_string()),
                },
                aliases: Vec::new(),
            },
            context: Context {
                workspace_dir: Some("/tmp/project".to_string()),
                created_at: Some(Utc::now()),
                last_active_at: Some(Utc::now()),
                tags: vec!["demo".to_string()],
            },
            events: vec![Event {
                id: "event-1".to_string(),
                kind: EventKind::Message,
                role: Role::Assistant,
                timestamp: Utc::now(),
                links: Default::default(),
                blocks: vec![
                    Block::Text {
                        text: "hello".to_string(),
                    },
                    Block::ToolCall {
                        tool_call_id: "call-1".to_string(),
                        name: "exec".to_string(),
                        input: Some(serde_json::json!({"cmd":"ls"})),
                    },
                ],
                metadata: crate::session::Metadata {
                    source: crate::session::Source {
                        provider_id: "codex".to_string(),
                        original_id: Some("event-1".to_string()),
                        original_role: Some("assistant".to_string()),
                        phase: Some("final_answer".to_string()),
                    },
                    model: Some("gpt-5.3-codex".to_string()),
                    usage: None,
                    fidelity: Fidelity::Preserved,
                    provider_ext: BTreeMap::new(),
                },
            }],
            artifacts: vec![Artifact {
                id: "artifact-1".to_string(),
                kind: crate::session::ArtifactKind::Patch,
                path: None,
                mime_type: None,
                content: Some("@@ -1 +1 @@".to_string()),
                metadata: BTreeMap::new(),
            }],
            extensions: {
                let mut extensions = BTreeMap::new();
                extensions.insert("source".to_string(), serde_json::json!({"kind":"test"}));
                extensions
            },
        }
    }

    #[test]
    fn morph_round_trip_preserves_canonical_session() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.morph");
        let session = sample_session();

        write_session(&path, &session).unwrap();
        let round_trip = read_session(&path).unwrap();

        assert_eq!(
            serde_json::to_value(&round_trip).unwrap(),
            serde_json::to_value(&session).unwrap()
        );
    }

    #[test]
    fn markdown_round_trip_preserves_embedded_canonical_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.md");
        let session = sample_session();

        write_markdown(&path, &session).unwrap();
        let round_trip = read_markdown(&path).unwrap();

        assert_eq!(
            serde_json::to_value(&round_trip).unwrap(),
            serde_json::to_value(&session).unwrap()
        );
    }

    #[test]
    fn html_round_trip_preserves_embedded_canonical_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.html");
        let session = sample_session();

        write_html(&path, &session).unwrap();
        let round_trip = read_html(&path).unwrap();

        assert_eq!(
            serde_json::to_value(&round_trip).unwrap(),
            serde_json::to_value(&session).unwrap()
        );
    }
}
