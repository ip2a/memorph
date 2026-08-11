//! Qwen CLI sessions, whose JSONL format follows Gemini CLI.
use crate::provider::{
    Provider, ProviderCapabilities, ProviderContentFidelity, ProviderSessionSummary, ScanStrategy,
    StorageShape, TurnQuality,
};
use crate::session::{
    Context, Event, EventKind, Fidelity, Identity, ImportedSession, Links, MappingDirection,
    MappingReport, Metadata, Provenance, ProviderRef, Role, Schema, Session,
};
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct QwenProvider;
const ID: &str = "qwen";
impl Provider for QwenProvider {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "Qwen"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            scan_strategy: ScanStrategy::FullScan,
            storage_shape: StorageShape::Jsonl,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                provider_payload: Some(Fidelity::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            ..ProviderCapabilities::default()
        }
    }
    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let Some(home) = dirs::home_dir() else {
            return Ok(Vec::new());
        };
        let root = home.join(".qwen").join("projects");
        let mut out = Vec::new();
        if !root.is_dir() {
            return Ok(out);
        }
        for e in WalkDir::new(root)
            .min_depth(1)
            .max_depth(4)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_type().is_file()
                    && e.path().extension().and_then(|x| x.to_str()) == Some("jsonl")
            })
        {
            let path = e.path();
            let records = read_records(path)?;
            let Some(id) = records.iter().find_map(|v| {
                v.get("sessionId")
                    .or_else(|| v.get("session_id"))
                    .and_then(Value::as_str)
            }) else {
                continue;
            };
            let title = records
                .iter()
                .find_map(|v| v.get("content").and_then(Value::as_str))
                .map(|s| s.chars().take(100).collect());
            out.push(ProviderSessionSummary {
                session_id: id.into(),
                title,
                project_dir: path.parent().map(|p| p.to_string_lossy().into()),
                created_at: None,
                last_active_at: None,
                source_path: Some(path.to_string_lossy().into()),
            });
        }
        Ok(out)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let path = Path::new(source_path);
        let records = read_records(path)?;
        let id = records
            .iter()
            .find_map(|v| {
                v.get("sessionId")
                    .or_else(|| v.get("session_id"))
                    .and_then(Value::as_str)
            })
            .unwrap_or_else(|| path.file_stem().and_then(|s| s.to_str()).unwrap_or("qwen"))
            .to_string();
        let report = MappingReport::new(ID, MappingDirection::Import);
        let events = records
            .into_iter()
            .enumerate()
            .filter_map(|(i, v)| event_from_record(&v, i))
            .collect::<Vec<_>>();
        let event_meta = events
            .iter()
            .map(|_| crate::session::EventMeta::preserved(ID))
            .collect();
        Ok(ImportedSession {
            session: Session {
                lineage: Vec::new(),
                schema: Schema::default(),
                identity: Identity {
                    id: id.clone(),
                    title: None,
                },
                context: Context {
                    workspace: path.parent().map(|p| p.to_string_lossy().into()),
                    created_at: None,
                    last_active_at: None,
                    tags: Vec::new(),
                },
                events,
                extensions: Default::default(),
            },
            provenance: Provenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".into()),
                primary_source: ProviderRef {
                    provider_id: ID.into(),
                    session_id: id,
                    source_path: Some(source_path.into()),
                },
                aliases: Vec::new(),
            },
            event_meta,
            report,
        })
    }
    fn session_source_fingerprint(
        &self,
        p: &str,
    ) -> Result<Option<crate::provider::ProviderSourceFingerprint>> {
        let m = std::fs::metadata(p)?;
        let d = Sha256::digest(std::fs::read(p)?);
        Ok(Some(crate::provider::ProviderSourceFingerprint {
            modified_at_ms: 0,
            size_bytes: m.len() as i64,
            value: format!("qwen-jsonl-v1:{d:x}"),
        }))
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        dirs::home_dir()
            .into_iter()
            .map(|h| h.join(".qwen").join("projects"))
            .collect()
    }
}
fn read_records(p: &Path) -> Result<Vec<Value>> {
    Ok(std::fs::read_to_string(p)?
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}
fn event_from_record(v: &Value, i: usize) -> Option<Event> {
    let role = match v.get("type").and_then(Value::as_str).unwrap_or("") {
        "user" => Role::User,
        "assistant" | "gemini" => Role::Assistant,
        _ => return None,
    };
    let text = v.get("content").and_then(Value::as_str).or_else(|| {
        v.get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
    })?;
    Some(Event {
        id: format!("qwen:{i}"),
        kind: EventKind::Message,
        role,
        timestamp: Utc::now(),
        links: Links::default(),
        blocks: vec![crate::session::Block::Text { text: text.into() }],
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    })
}
