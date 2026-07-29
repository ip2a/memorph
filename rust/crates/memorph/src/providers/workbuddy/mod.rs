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

pub struct WorkBuddyProvider;
const PROVIDER_ID: &str = "workbuddy";

impl Provider for WorkBuddyProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "WorkBuddy"
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
                if !is_trace(path) {
                    continue;
                }
                let Ok(doc) = read_doc(path) else {
                    continue;
                };
                let Some(id) = doc_id(&doc, path) else {
                    continue;
                };
                sessions.push(ProviderSessionSummary {
                    session_id: id,
                    title: doc_title(&doc),
                    project_dir: None,
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
            .with_context(|| format!("WorkBuddy source does not exist: {source_path}"))?;
        let doc = read_doc(&path)?;
        let id = doc_id(&doc, &path)
            .context("WorkBuddy source must be .workbuddy/traces/trace_<id>.json")?;
        let spans = doc
            .get("spans")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let events: Vec<Event> = spans
            .iter()
            .enumerate()
            .filter_map(|(i, e)| map_event(e, i, &mut report))
            .collect();
        let event_meta = events
            .iter()
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect::<Vec<_>>();
        let mut extensions = BTreeMap::new();
        extensions.insert("workbuddy_trace_json".into(), doc.clone());
        let created = spans
            .iter()
            .find_map(timestamp)
            .or_else(|| modified_datetime(&path))
            .unwrap_or_else(Utc::now);
        Ok(ImportedSession {
            session: Session {
                schema: Schema::default(),
                identity: Identity {
                    id: id.clone(),
                    title: doc_title(&doc),
                },
                context: Context {
                    workspace: None,
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
                "workbuddy-trace-v1:{}:{digest:x}",
                doc_id(&read_doc(path)?, path).unwrap_or_default()
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
        .map(|h| h.join(".workbuddy/traces"))
        .collect()
}
fn is_trace(path: &Path) -> bool {
    path.file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.starts_with("trace_") && v.ends_with(".json"))
        .unwrap_or(false)
}
fn read_doc(path: &Path) -> Result<Value> {
    serde_json::from_str(&std::fs::read_to_string(path)?)
        .with_context(|| format!("invalid WorkBuddy trace: {}", path.display()))
}
fn doc_id(value: &Value, path: &Path) -> Option<String> {
    value
        .pointer("/trace/traceId")
        .or_else(|| value.get("traceId"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()?
                .to_str()?
                .strip_prefix("trace_")
                .map(str::to_string)
        })
}
fn doc_title(value: &Value) -> Option<String> {
    value
        .pointer("/trace/title")
        .or_else(|| value.get("title"))
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string)
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
fn text_for(v: &Value) -> Option<String> {
    let m = v.get("message").unwrap_or(v);
    let c = m.get("content").unwrap_or(m);
    c.as_str().map(str::to_string).or_else(|| {
        c.as_array()?
            .iter()
            .find_map(|b| b.get("text").and_then(Value::as_str).map(str::to_string))
    })
}
fn map_event(v: &Value, i: usize, r: &mut MappingReport) -> Option<Event> {
    let kind_raw = v.get("type").and_then(Value::as_str)?.to_ascii_lowercase();
    let (role, kind, blocks) = match kind_raw.as_str() {
        "user" | "human" | "user_message" => (
            Role::User,
            EventKind::Message,
            vec![Block::Text { text: text_for(v)? }],
        ),
        "generation" | "assistant" | "agent" => (
            Role::Assistant,
            EventKind::Message,
            vec![Block::Text {
                text: text_for(v).unwrap_or_else(|| {
                    v.get("model")
                        .and_then(Value::as_str)
                        .unwrap_or("assistant")
                        .into()
                }),
            }],
        ),
        "tool" | "tool_call" | "function" => (
            Role::Tool,
            EventKind::Action,
            vec![Block::ToolCall {
                tool_call_id: format!("workbuddy:{i}"),
                name: v
                    .get("toolName")
                    .or_else(|| v.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .into(),
                input: v.get("toolInput").or_else(|| v.get("input")).cloned(),
            }],
        ),
        "tool_result" | "tool_output" => (
            Role::Tool,
            EventKind::Observation,
            vec![Block::ToolResult {
                tool_call_id: format!("workbuddy:{i}"),
                content: text_for(v).unwrap_or_default(),
                is_error: false,
            }],
        ),
        _ => return None,
    };
    r.push_issue(crate::session::MappingIssue {
        level: crate::session::MappingIssueLevel::Info,
        disposition: Fidelity::Preserved,
        code: "workbuddy-native-span".into(),
        message: "Mapped WorkBuddy trace span".into(),
        path: Some(format!("spans[{i}]")),
        raw: None,
    });
    Some(Event {
        id: format!("workbuddy:span:{i}"),
        kind,
        role,
        timestamp: timestamp(v).unwrap_or_else(Utc::now),
        links: Links::default(),
        blocks,
        metadata: Metadata {
            model: v.get("model").and_then(Value::as_str).map(str::to_string),
            usage: None,
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_comes_from_trace_document_or_filename() {
        assert_eq!(
            doc_id(
                &serde_json::json!({}),
                Path::new("/tmp/.workbuddy/traces/trace_abc.json")
            )
            .as_deref(),
            Some("abc")
        );
    }
    #[test]
    fn maps_generation_span() {
        let v = serde_json::json!({"type":"generation","content":"ok"});
        let mut r = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let e = map_event(&v, 0, &mut r).unwrap();
        assert!(matches!(e.blocks[0],Block::Text{ref text} if text=="ok"));
    }
}
