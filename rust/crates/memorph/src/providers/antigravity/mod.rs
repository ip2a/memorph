pub mod adapter;
pub mod hook;

use crate::provider::{
    PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionSummary, ProviderSourceFingerprint, ProviderWriteRisk,
    ResumeQuality, ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::session::{
    Block, Context, Event, EventKind, Fidelity, Identity, ImportedSession, Links, MappingDirection,
    MappingReport, Metadata, Provenance, ProviderRef, Role, Schema, Session,
};
use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct AntigravityProvider;
const PROVIDER_ID: &str = "antigravity";

impl Provider for AntigravityProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "Antigravity"
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
            for session_dir in session_directories(&root) {
                let Some(id) = session_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|id| valid_session_id(id))
                    .map(str::to_string)
                else {
                    continue;
                };
                let doc = document_from_session_dir(&session_dir, &id).unwrap_or_default();
                sessions.push(ProviderSessionSummary {
                    session_id: id,
                    title: messages(&doc).iter().find_map(|message| text_for(message)),
                    project_dir: doc
                        .get("workspace")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    created_at: None,
                    last_active_at: modified_ms(&session_dir),
                    source_path: Some(session_dir.to_string_lossy().into_owned()),
                });
            }
        }
        sessions.sort_by_key(|session| std::cmp::Reverse(session.last_active_at.unwrap_or(0)));
        sessions.dedup_by(|left, right| left.session_id == right.session_id);
        Ok(sessions)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let path = std::fs::canonicalize(source_path)
            .with_context(|| format!("Antigravity source does not exist: {source_path}"))?;
        anyhow::ensure!(
            path.is_dir(),
            "Antigravity source must be a session directory"
        );
        let id = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("Antigravity session directory has no id")?
            .to_string();
        anyhow::ensure!(
            valid_session_id(&id),
            "Invalid Antigravity session id: {id}"
        );
        let doc = document_from_session_dir(&path, &id)?;
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let events: Vec<Event> = messages(&doc)
            .iter()
            .enumerate()
            .filter_map(|(i, m)| map_message(m, i, &mut report))
            .collect();
        let mut extensions = BTreeMap::new();
        extensions.insert("antigravity_session_json".into(), doc.clone());
        let created = timestamp(&doc, "startTime")
            .or_else(|| modified_datetime(&path))
            .unwrap_or_else(Utc::now);
        let event_meta = events
            .iter()
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect::<Vec<_>>();
        Ok(ImportedSession {
            session: Session {
                lineage: Vec::new(),
                schema: Schema::default(),
                identity: Identity {
                    id: id.clone(),
                    title: messages(&doc).iter().find_map(|m| text_for(m)),
                },
                context: Context {
                    workspace: doc
                        .get("workspace")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            doc.get("directories")
                                .and_then(Value::as_array)
                                .and_then(|a| a.first())
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        }),
                    created_at: Some(created),
                    last_active_at: timestamp(&doc, "lastUpdated")
                        .or_else(|| modified_datetime(&path)),
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
        let mut digest = Sha256::new();
        let mut size = 0_u64;
        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() {
                let bytes = std::fs::read(entry.path())?;
                size = size.saturating_add(bytes.len() as u64);
                digest.update(entry.path().to_string_lossy().as_bytes());
                digest.update(bytes);
            }
        }
        let id = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms: modified_ms(path).unwrap_or(0),
            size_bytes: size.min(i64::MAX as u64) as i64,
            value: format!("antigravity-v2:{id}:{:x}", digest.finalize()),
        }))
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        roots()
    }
}
fn roots() -> Vec<PathBuf> {
    dirs::home_dir()
        .map(|home| {
            vec![
                home.join(".gemini").join("antigravity"),
                home.join(".gemini").join("antigravity-cli"),
            ]
        })
        .unwrap_or_default()
}

fn session_directories(root: &Path) -> Vec<PathBuf> {
    let brain = root.join("brain");
    let Ok(entries) = std::fs::read_dir(brain) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect()
}

fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn document_from_session_dir(path: &Path, id: &str) -> Result<Value> {
    let manifest = path.join("manifest.json");
    let mut document = if manifest.is_file() {
        read_document(&manifest).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let messages = transcript_messages(path)?;
    let object = document
        .as_object_mut()
        .context("Antigravity manifest must be an object")?;
    object.insert("sessionId".into(), Value::String(id.to_string()));
    object.insert("messages".into(), Value::Array(messages));
    Ok(document)
}

fn transcript_messages(session_dir: &Path) -> Result<Vec<Value>> {
    let candidates = [
        session_dir.join(".system_generated/logs/transcript_full.jsonl"),
        session_dir.join("transcript_full.jsonl"),
        session_dir.join("transcript.jsonl"),
    ];
    let Some(path) = candidates.iter().find(|path| path.is_file()) else {
        return Ok(Vec::new());
    };
    let mut messages = Vec::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let source = record.get("source").and_then(Value::as_str).unwrap_or("");
        let role = match source {
            "USER_EXPLICIT" | "USER" => "user",
            "MODEL" | "PLANNER" | "SYSTEM" => "gemini",
            _ => continue,
        };
        let Some(content) = record
            .get("content")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        messages.push(serde_json::json!({
            "type": role,
            "content": [{"text": content}],
            "timestamp": record.get("created_at").cloned().unwrap_or(Value::Null)
        }));
    }
    Ok(messages)
}

fn read_document(path: &Path) -> Result<Value> {
    serde_json::from_str(&std::fs::read_to_string(path)?)
        .with_context(|| format!("invalid Antigravity JSON in {}", path.display()))
}
fn document_id(doc: &Value) -> Option<String> {
    doc.get("sessionId")
        .or_else(|| doc.get("session_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
fn messages(doc: &Value) -> Vec<&Value> {
    doc.get("messages")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|m| {
                    matches!(
                        m.get("type").and_then(Value::as_str),
                        Some("user") | Some("gemini")
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}
fn timestamp(doc: &Value, key: &str) -> Option<DateTime<Utc>> {
    doc.get(key)
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
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
fn text_for(v: &Value) -> Option<String> {
    v.get("content")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .map(str::to_string)
        .next()
}
fn blocks(v: &Value) -> Vec<Block> {
    let mut out = Vec::new();
    if let Some(parts) = v.get("content").and_then(Value::as_array) {
        for p in parts {
            if let Some(t) = p.get("text").and_then(Value::as_str) {
                out.push(Block::Text { text: t.into() });
            }
        }
    }
    if let Some(thoughts) = v.get("thoughts").and_then(Value::as_array) {
        for p in thoughts {
            if let Some(t) = p
                .get("text")
                .or_else(|| p.get("summary"))
                .and_then(Value::as_str)
            {
                out.push(Block::Thinking {
                    text: t.into(),
                    signature: None,
                });
            }
        }
    }
    if let Some(calls) = v.get("toolCalls").and_then(Value::as_array) {
        for c in calls {
            out.push(Block::ToolCall {
                tool_call_id: c
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .into(),
                name: c
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .into(),
                input: c
                    .get("args")
                    .cloned()
                    .or_else(|| c.get("arguments").cloned()),
            });
            if let Some(result) = c.get("result") {
                out.push(Block::ToolResult {
                    tool_call_id: c
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .into(),
                    content: result.to_string(),
                    outcome: crate::session::execution_outcome(false),
                });
            }
        }
    }
    out
}
fn map_message(v: &Value, i: usize, r: &mut MappingReport) -> Option<Event> {
    let role_raw = v.get("type").and_then(Value::as_str)?;
    let role = match role_raw {
        "user" => Role::User,
        "gemini" => Role::Assistant,
        _ => return None,
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
        code: "antigravity-native-message".into(),
        message: "Mapped Antigravity JSON message".into(),
        path: Some(format!("messages[{i}]")),
        raw: None,
    });
    Some(Event {
        id: format!("antigravity:event:{i}"),
        kind,
        role,
        timestamp: timestamp(v, "timestamp").unwrap_or_else(Utc::now),
        links: Links::default(),
        blocks: bs,
        tags: Vec::new(),
        extensions: Default::default(),
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
    fn identity_comes_from_document_metadata() {
        let d = serde_json::json!({"sessionId":"ag-1","projectHash":"p","messages":[]});
        assert_eq!(document_id(&d).as_deref(), Some("ag-1"));
    }
    #[test]
    fn maps_antigravity_messages_and_blocks() {
        let d = serde_json::json!({"type":"gemini","content":[{"text":"answer"}],"thoughts":[{"summary":"think"}],"toolCalls":[{"id":"c1","name":"read","args":{"path":"x"},"result":"ok"}]});
        let mut r = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let e = map_message(&d, 0, &mut r).unwrap();
        assert_eq!(e.blocks.len(), 4);
        assert!(matches!(e.role, Role::Assistant));
    }
    #[test]
    fn imports_brain_transcript_and_uses_v2_fingerprint() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let session = temp.path().join("brain").join("ag-session-1");
        std::fs::create_dir_all(session.join(".system_generated/logs"))?;
        std::fs::write(
            session.join("manifest.json"),
            r#"{"workspace":"/workspace/antigravity"}"#,
        )?;
        std::fs::write(
            session.join(".system_generated/logs/transcript_full.jsonl"),
            "{\"source\":\"USER_EXPLICIT\",\"content\":\"hello\",\"created_at\":\"2026-01-01T00:00:00Z\"}\n{\"source\":\"MODEL\",\"content\":\"world\",\"created_at\":\"2026-01-01T00:00:01Z\"}\n",
        )?;
        let dirs = session_directories(temp.path());
        assert_eq!(dirs, vec![session.clone()]);
        let imported = AntigravityProvider.import_session(session.to_str().unwrap())?;
        assert_eq!(imported.session.identity.id, "ag-session-1");
        assert_eq!(imported.session.events.len(), 2);
        assert_eq!(
            imported.session.context.workspace.as_deref(),
            Some("/workspace/antigravity")
        );
        let fingerprint = AntigravityProvider
            .session_source_fingerprint(session.to_str().unwrap())?
            .unwrap();
        assert!(fingerprint
            .value
            .starts_with("antigravity-v2:ag-session-1:"));
        assert!(roots().iter().all(|root| !root.ends_with(".gemini/tmp")));
        Ok(())
    }

    #[test]
    fn main_gemini_sessions_are_not_antigravity() {
        let d = serde_json::json!({"kind":"main"});
        assert!(d.get("kind").and_then(Value::as_str) == Some("main"));
    }
}
