pub mod adapter;
pub mod hook;

use crate::session::{
    Block, Context, Event, EventKind, Fidelity, Identity, ImportedSession, Links, MappingDirection,
    MappingReport, Metadata, Provenance, ProviderRef, Role, Schema, Session,
};
use crate::provider::{
    PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionSummary, ProviderSourceFingerprint, ProviderWriteRisk,
    ResumeQuality, ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct CodeBuddyProvider;
const PROVIDER_ID: &str = "codebuddy";

impl Provider for CodeBuddyProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "CodeBuddy"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            storage_shape: StorageShape::Directory,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::FullImport,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Preserved),
                tool_result: Some(Fidelity::Preserved),
                provider_payload: Some(Fidelity::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            resume_quality: ResumeQuality::None,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: false,
                sidecar_files: true,
                index_repair: false,
            },
            backup_support: ProviderBackupSupport {
                before_write: false,
                restore: false,
                sync_only: false,
            },
            activity_support: ProviderActivitySupport {
                hook_events: false,
                runtime_endpoint: false,
                session_activity: false,
            },
            ..ProviderCapabilities::default()
        }
    }
    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let mut sessions = Vec::new();
        for root in roots() {
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
                let Some(id) = session_id(path) else {
                    continue;
                };
                let events = read_events(path).unwrap_or_default();
                let title = events
                    .iter()
                    .find_map(|e| text_for(e).filter(|s| !s.trim().is_empty()));
                sessions.push(ProviderSessionSummary {
                    session_id: id,
                    title,
                    project_dir: events.iter().find_map(workspace),
                    created_at: None,
                    last_active_at: modified_ms(path),
                    source_path: Some(path.to_string_lossy().into_owned()),
                });
            }
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
        sessions.dedup_by(|a, b| a.session_id == b.session_id);
        Ok(sessions)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let path = std::fs::canonicalize(source_path)
            .with_context(|| format!("CodeBuddy source does not exist: {source_path}"))?;
        let id = session_id(&path)
            .context("CodeBuddy source must be .codebuddy/projects/<project>/<session>.jsonl")?;
        let raw = read_events(&path)?;
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let events: Vec<Event> = raw
            .iter()
            .enumerate()
            .filter_map(|(i, e)| map_event(e, i, &mut report))
            .collect();
        let event_meta = events
            .iter()
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect::<Vec<_>>();
        let mut extensions = BTreeMap::new();
        extensions.insert("codebuddy_session_jsonl".into(), Value::Array(raw.clone()));
        let created = raw
            .iter()
            .find_map(timestamp)
            .or_else(|| modified_datetime(&path))
            .unwrap_or_else(Utc::now);
        Ok(ImportedSession {
            session: Session {
                schema: Schema::default(),
                identity: Identity {
                    id: id.clone(),
                    title: raw.iter().find_map(text_for),
                },
                context: Context {
                    workspace: raw.iter().find_map(workspace),
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
                    provider_id: PROVIDER_ID.into(),
                    session_id: id,
                    source_path: Some(path.to_string_lossy().into_owned()),
                },
                aliases: Vec::new(),
            },
            event_meta,
            report,
        })
    }
    fn session_source_fingerprint(
        &self,
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
                "codebuddy-session-v1:{}:{digest:x}",
                session_id(path).unwrap_or_default()
            ),
        }))
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        roots()
    }
}
fn roots() -> Vec<PathBuf> {
    dirs::home_dir()
        .into_iter()
        .map(|h| h.join(".codebuddy/projects"))
        .collect()
}
fn session_id(path: &Path) -> Option<String> {
    path.file_stem()?
        .to_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
fn read_events(path: &Path) -> Result<Vec<Value>> {
    std::fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l)
                .with_context(|| format!("invalid CodeBuddy event in {}", path.display()))
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
fn timestamp(v: &Value) -> Option<DateTime<Utc>> {
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
fn workspace(v: &Value) -> Option<String> {
    [
        "cwd",
        "workspace",
        "workspaceDirectory",
        "projectDir",
        "rootPath",
    ]
    .iter()
    .find_map(|k| v.get(*k).and_then(Value::as_str).map(str::to_string))
    .or_else(|| v.get("message").and_then(workspace))
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
                    is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                }),
                _ => None,
            },
        )
        .collect()
}
fn map_event(v: &Value, i: usize, r: &mut MappingReport) -> Option<Event> {
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
    let kind = if bs.iter().any(|b| matches!(b, Block::ToolCall { .. })) {
        EventKind::Action
    } else if bs.iter().any(|b| matches!(b, Block::ToolResult { .. })) {
        EventKind::Observation
    } else {
        EventKind::Message
    };
    r.push_issue(crate::session::MappingIssue {
        level: crate::session::MappingIssueLevel::Info,
        disposition: Fidelity::Preserved,
        code: "codebuddy-native-event".into(),
        message: "Mapped Factory/CodeBuddy JSONL event".into(),
        path: Some(format!("events[{i}]")),
        raw: None,
    });
    Some(Event {
        id: format!("codebuddy:event:{i}"),
        kind,
        role,
        timestamp: timestamp(v).unwrap_or_else(Utc::now),
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_comes_from_transcript_filename() {
        assert_eq!(
            session_id(Path::new("/tmp/.codebuddy/projects/project/abc.jsonl")).as_deref(),
            Some("abc")
        );
    }
    #[test]
    fn maps_claude_style_codebuddy_event() {
        let v = serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}});
        let mut r = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let e = map_event(&v, 0, &mut r).unwrap();
        assert!(matches!(e.blocks[0],Block::Text{ref text} if text=="ok"));
    }
}
