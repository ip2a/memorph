//! Shared parsing for the pi-ai JSONL session family (Pi, Oh My Pi).
//!
//! Both agents write `<ts>_<uuid>.jsonl` transcripts under
//! `<home>/<agent>/agent/sessions/<encoded-cwd>/`. Oh My Pi files may carry a
//! leading `{"type":"title"}` row, so the session header is located by type,
//! not by position.

use crate::provider::{ProviderSessionSummary, ProviderSourceFingerprint};
use crate::session::{
    Block, Context, Event, EventKind, Fidelity, Identity, ImportedSession, Links, MappingDirection,
    MappingReport, Metadata, Provenance, ProviderRef, Role, Schema, Session,
};
use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub(crate) fn scan(roots: &[PathBuf]) -> Result<Vec<ProviderSessionSummary>> {
    let mut sessions = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(records) = read_records(path) else {
                continue;
            };
            let Some(header) = records
                .iter()
                .find(|v| v.get("type").and_then(Value::as_str) == Some("session"))
            else {
                continue;
            };
            let Some(id) = session_id(header) else {
                continue;
            };
            let events: Vec<&Value> = records
                .iter()
                .filter(|v| v.get("type").and_then(Value::as_str) == Some("message"))
                .collect();
            let title = events
                .iter()
                .find_map(|e| text_for(e).filter(|s| !s.trim().is_empty()));
            sessions.push(ProviderSessionSummary {
                archived: false,
                session_id: id,
                title,
                project_dir: header
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                created_at: timestamp(header),
                last_active_at: modified_ms(path),
                source_path: Some(path.to_string_lossy().into_owned()),
            });
        }
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
    sessions.dedup_by(|a, b| a.session_id == b.session_id);
    Ok(sessions)
}

pub(crate) fn import(provider_id: &str, source_path: &str) -> Result<ImportedSession> {
    let path = std::fs::canonicalize(source_path)
        .with_context(|| format!("{provider_id} source does not exist: {source_path}"))?;
    let raw = read_records(&path)?;
    let header = raw
        .iter()
        .find(|v| v.get("type").and_then(Value::as_str) == Some("session"))
        .context("source must contain a session header")?;
    let id = session_id(header).context("session header must contain id")?;
    let mut report = MappingReport::new(provider_id, MappingDirection::Import);
    let events: Vec<Event> = raw
        .iter()
        .enumerate()
        .filter_map(|(i, e)| map_event(provider_id, e, i, &mut report))
        .collect();
    let event_meta = events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(provider_id))
        .collect::<Vec<_>>();
    let mut extensions = std::collections::BTreeMap::new();
    extensions.insert("pi_session_jsonl".into(), Value::Array(raw.clone()));
    let created = raw
        .iter()
        .find_map(rfc3339_ms)
        .or_else(|| modified_datetime(&path))
        .unwrap_or_else(Utc::now);
    Ok(ImportedSession {
        session: Session {
            lineage: Vec::new(),
            schema: Schema::default(),
            identity: Identity {
                id: id.clone(),
                title: raw.iter().find_map(text_for),
            },
            context: Context {
                workspace: header
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                created_at: Some(created),
                last_active_at: modified_datetime(&path),
                tags: Vec::new(),
            },
            events,
            extensions,
        },
        provenance: Provenance {
            imported_at: Utc::now(),
            imported_by: Some("memorph-cli".into()),
            primary_source: ProviderRef {
                provider_id: provider_id.into(),
                session_id: id.clone(),
                source_path: Some(path.to_string_lossy().into_owned()),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    })
}

pub(crate) fn fingerprint(
    provider_id: &str,
    source_path: &str,
) -> Result<Option<ProviderSourceFingerprint>> {
    let path = Path::new(source_path);
    if !path.exists() {
        return Ok(None);
    }
    let digest = Sha256::digest(std::fs::read(path)?);
    let size = std::fs::metadata(path)?.len();
    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms: modified_ms(path).unwrap_or(0),
        size_bytes: size.min(i64::MAX as u64) as i64,
        value: format!(
            "{provider_id}-session-v1:{}:{digest:x}",
            read_records(path)?
                .iter()
                .find(|v| v.get("type").and_then(Value::as_str) == Some("session"))
                .and_then(session_id)
                .unwrap_or_default()
        ),
    }))
}

fn session_id(header: &Value) -> Option<String> {
    header
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn read_records(path: &Path) -> Result<Vec<Value>> {
    std::fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).with_context(|| {
                format!("invalid session record in {}", path.display())
            })
        })
        .collect()
}

fn modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
}

fn modified_datetime(path: &Path) -> Option<DateTime<Utc>> {
    modified_ms(path).and_then(DateTime::<Utc>::from_timestamp_millis)
}

fn timestamp(v: &Value) -> Option<i64> {
    v.get("timestamp")
        .or_else(|| v.get("createdAt"))
        .and_then(|x| {
            x.as_str()
                .and_then(|s| {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|d| d.with_timezone(&Utc))
                })
                .or_else(|| x.as_i64().and_then(DateTime::<Utc>::from_timestamp_millis))
        })
        .map(|d| d.timestamp_millis())
}

fn text_for(v: &Value) -> Option<String> {
    let m = v.get("message").unwrap_or(v);
    let c = m.get("content").unwrap_or(m);
    c.as_str().map(str::to_string).or_else(|| {
        c.as_array()?
            .iter()
            .find_map(|b| b.get("text").and_then(Value::as_str).map(str::to_string))
    })
}

fn blocks(v: &Value) -> Vec<Block> {
    let m = v.get("message").unwrap_or(v);
    let c = m.get("content").unwrap_or(m);
    if let Some(s) = c.as_str() {
        return vec![Block::Text { text: s.into() }];
    };
    c.as_array()
        .into_iter()
        .flatten()
        .filter_map(
            |b| match b.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" | "input_text" | "output_text" => b
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|s| Block::Text { text: s.into() }),
                "thinking" => b
                    .get("thinking")
                    .and_then(Value::as_str)
                    .map(|s| Block::Thinking {
                        text: s.into(),
                        signature: None,
                    }),
                "tool_use" | "toolCall" => Some(Block::ToolCall {
                    tool_call_id: b
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .into(),
                    name: b
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .into(),
                    input: b.get("input").or_else(|| b.get("arguments")).cloned(),
                }),
                "tool_result" | "toolResult" => Some(Block::ToolResult {
                    tool_call_id: b
                        .get("tool_use_id")
                        .or_else(|| b.get("toolCallId"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .into(),
                    content: b.get("content").map(|x| x.to_string()).unwrap_or_default(),
                    outcome: crate::session::execution_outcome(
                        b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    ),
                }),
                _ => None,
            },
        )
        .collect()
}

fn map_event(provider_id: &str, v: &Value, i: usize, r: &mut MappingReport) -> Option<Event> {
    if v.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let m = v.get("message").unwrap_or(v);
    let role_raw = m
        .get("role")
        .or_else(|| v.get("role"))
        .or_else(|| v.get("type"))?
        .as_str()?;
    let role = match role_raw.to_ascii_lowercase().as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        "system" => Role::System,
        _ => Role::Other,
    };
    let bs = blocks(v);
    if bs.is_empty() {
        return None;
    };
    let kind = if bs.iter().any(|b| matches!(b, Block::ToolResult { .. })) {
        EventKind::Observation
    } else if bs.iter().any(|b| matches!(b, Block::ToolCall { .. })) {
        EventKind::Action
    } else {
        EventKind::Message
    };
    r.push_issue(crate::session::MappingIssue {
        level: crate::session::MappingIssueLevel::Info,
        disposition: Fidelity::Preserved,
        code: "pi-native-event".into(),
        message: "Mapped pi-ai JSONL event".into(),
        path: Some(format!("events[{i}]")),
        raw: None,
    });
    Some(Event {
        id: format!("{provider_id}:event:{i}"),
        kind,
        role,
        timestamp: rfc3339_ms(v).unwrap_or_else(Utc::now),
        links: Links::default(),
        blocks: bs,
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    })
}

fn rfc3339_ms(v: &Value) -> Option<DateTime<Utc>> {
    v.get("timestamp")
        .or_else(|| v.get("createdAt"))
        .and_then(|x| {
            x.as_str()
                .and_then(|s| {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|d| d.with_timezone(&Utc))
                })
                .or_else(|| x.as_i64().and_then(DateTime::<Utc>::from_timestamp_millis))
        })
}
