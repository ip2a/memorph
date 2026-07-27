pub mod adapter;
pub mod hook;

use crate::canonical::{
    Block, Context, Event, EventKind, Fidelity, Identity, ImportedSession, Links, MappingDirection,
    MappingReport, Metadata, Provenance, ProviderRef, Role, Schema, Session, Source,
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

pub struct CopilotProvider;
const PROVIDER_ID: &str = "copilot";
const EVENTS_FILE: &str = "events.jsonl";

impl Provider for CopilotProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "GitHub Copilot"
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
                provider_payload: Some(Fidelity::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            resume_quality: ResumeQuality::None,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: false,
                sidecar_files: false,
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
        let mut out = Vec::new();
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
                if path.file_name().and_then(|v| v.to_str()) != Some(EVENTS_FILE) {
                    continue;
                }
                let Some(id) = session_id(path) else { continue };
                let events = read_events(path).unwrap_or_default();
                let title =
                    events
                        .iter()
                        .find_map(|e| match e.get("type").and_then(Value::as_str) {
                            Some("user.message") => e
                                .get("data")
                                .and_then(|d| d.get("content"))
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            _ => None,
                        });
                out.push(ProviderSessionSummary {
                    session_id: id,
                    title,
                    project_dir: session_cwd(&events),
                    created_at: None,
                    last_active_at: file_modified_ms(path),
                    source_path: Some(path.to_string_lossy().into_owned()),
                });
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
        out.dedup_by(|a, b| a.session_id == b.session_id);
        Ok(out)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let path = std::fs::canonicalize(source_path)
            .with_context(|| format!("Copilot source does not exist: {source_path}"))?;
        let id =
            session_id(&path).context("Copilot source must be session-state/<id>/events.jsonl")?;
        let events = read_events(&path)?;
        let timestamp = events
            .iter()
            .find_map(event_time)
            .or_else(|| file_modified_datetime(&path))
            .unwrap_or_else(Utc::now);
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let canonical_events = events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| map_event(e, i, &mut report))
            .collect();
        let title = events
            .iter()
            .find_map(|e| {
                if e.get("type").and_then(Value::as_str) != Some("user.message") {
                    return None;
                }
                e.get("data")
                    .and_then(|d| d.get("content"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .filter(|s| !s.trim().is_empty());
        let mut extensions = BTreeMap::new();
        extensions.insert("copilot_events_jsonl".into(), Value::Array(events));
        Ok(ImportedSession {
            session: Session {
                schema: Schema::default(),
                identity: Identity {
                    canonical_id: id.clone(),
                    source_title: title,
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
                context: Context {
                    workspace_dir: session_cwd_from_value(&extensions["copilot_events_jsonl"]),
                    created_at: Some(timestamp),
                    last_active_at: file_modified_datetime(&path),
                    tags: Vec::new(),
                },
                events: canonical_events,
                artifacts: Vec::new(),
                extensions,
            },
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
        let metadata = std::fs::metadata(path)?;
        let digest = Sha256::digest(std::fs::read(path)?);
        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms: file_modified_ms(path).unwrap_or(0),
            size_bytes: metadata.len().min(i64::MAX as u64) as i64,
            value: format!(
                "copilot-events-v1:{}:{digest:x}",
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
        .map(|h| h.join(".copilot/session-state"))
        .collect()
}
fn session_id(path: &Path) -> Option<String> {
    path.parent()?
        .file_name()?
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
                .with_context(|| format!("invalid Copilot event in {}", path.display()))
        })
        .collect()
}
fn file_modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
}
fn file_modified_datetime(path: &Path) -> Option<DateTime<Utc>> {
    file_modified_ms(path).and_then(DateTime::<Utc>::from_timestamp_millis)
}
fn event_time(event: &Value) -> Option<DateTime<Utc>> {
    event
        .get("timestamp")
        .or_else(|| event.get("ts"))
        .or_else(|| event.get("data")?.get("timestamp"))
        .and_then(|v| {
            v.as_str()
                .and_then(|s| {
                    DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|d| d.with_timezone(&Utc))
                })
                .or_else(|| v.as_i64().and_then(DateTime::<Utc>::from_timestamp_millis))
        })
}
fn session_cwd(events: &[Value]) -> Option<String> {
    events.iter().find_map(|e| {
        e.get("type")
            .and_then(Value::as_str)
            .filter(|t| *t == "session.start")
            .and_then(|_| {
                e.get("data")?
                    .get("context")?
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
    })
}
fn session_cwd_from_value(value: &Value) -> Option<String> {
    value.as_array().and_then(|a| {
        a.iter().find_map(|e| {
            e.get("type")
                .and_then(Value::as_str)
                .filter(|t| *t == "session.start")
                .and_then(|_| {
                    e.get("data")?
                        .get("context")?
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
        })
    })
}
fn map_event(event: &Value, index: usize, report: &mut MappingReport) -> Option<Event> {
    let kind = event.get("type").and_then(Value::as_str)?;
    let (role, content) = match kind {
        "user.message" => (
            Role::User,
            event.get("data")?.get("content")?.as_str()?.to_string(),
        ),
        "assistant.message" => (
            Role::Assistant,
            event.get("data")?.get("content")?.as_str()?.to_string(),
        ),
        _ => return None,
    };
    report.push_issue(crate::canonical::MappingIssue {
        level: crate::canonical::MappingIssueLevel::Info,
        disposition: Fidelity::Preserved,
        code: "copilot-native-message".into(),
        message: "Mapped Copilot CLI message event".into(),
        path: Some(format!("events[{index}]")),
        raw: None,
    });
    Some(Event {
        id: format!("copilot:event:{index}"),
        kind: EventKind::Message,
        role,
        timestamp: event_time(event).unwrap_or_else(Utc::now),
        links: Links::default(),
        blocks: vec![Block::Text { text: content }],
        metadata: Metadata {
            source: Source {
                provider_id: PROVIDER_ID.into(),
                original_id: None,
                original_role: Some(kind.into()),
                phase: None,
            },
            model: event
                .get("data")
                .and_then(|d| d.get("model"))
                .and_then(Value::as_str)
                .map(str::to_string),
            usage: None,
            fidelity: Fidelity::Preserved,
            provider_ext: BTreeMap::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capabilities_match_native_read_only_contract() {
        let capabilities = CopilotProvider.capabilities();
        assert!(capabilities.scan);
        assert!(capabilities.import);
        assert!(!capabilities.export);
        assert!(!capabilities.delete);
        assert!(!capabilities.rename);
        assert!(!capabilities.resume);
        assert_eq!(capabilities.storage_shape, StorageShape::Directory);
        assert_eq!(capabilities.scan_strategy, ScanStrategy::FullScan);
        assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);
        assert_eq!(capabilities.resume_quality, ResumeQuality::None);
        assert!(!capabilities.backup_support.before_write);
        assert!(!capabilities.backup_support.restore);
    }
    #[test]
    fn identity_comes_from_session_directory() {
        assert_eq!(
            session_id(Path::new("/tmp/session-state/abc/events.jsonl")).as_deref(),
            Some("abc")
        );
    }
    #[test]
    fn maps_copilot_message_events() {
        let e = serde_json::json!({"type":"assistant.message","data":{"content":"hello"}});
        let mut r = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let mapped = map_event(&e, 0, &mut r).unwrap();
        assert!(matches!(mapped.blocks[0], Block::Text { ref text } if text == "hello"));
    }
}
