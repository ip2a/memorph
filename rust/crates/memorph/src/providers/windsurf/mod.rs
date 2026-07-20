use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ImportedSession, MappingDirection, MappingDisposition, MappingReport,
    ProviderSessionRef, SessionContext, SessionEvent, SessionEventKind, SessionIdentity,
    SessionProvenance,
};
use crate::provider::{
    PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionSummary, ProviderSourceFingerprint, ProviderWriteRisk,
    ResumeQuality, ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct WindsurfProvider;
const PROVIDER_ID: &str = "windsurf";
const ACTIVE_PREFIX: &str = "windsurf.state.cachedActiveTrajectory:";

impl Provider for WindsurfProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "Windsurf"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            storage_shape: StorageShape::Mixed,
            scan_strategy: ScanStrategy::Indexed,
            page_strategy: PageStrategy::FullImport,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Preserved),
                tool_call: Some(MappingDisposition::Preserved),
                tool_result: Some(MappingDisposition::Preserved),
                provider_payload: Some(MappingDisposition::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            resume_quality: ResumeQuality::None,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: true,
                sidecar_files: true,
                index_repair: true,
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
        for db in state_paths() {
            if !db.exists() {
                continue;
            }
            let conn = Connection::open(&db)?;
            let mut stmt = conn.prepare("SELECT key, value FROM ItemTable WHERE key LIKE ?1")?;
            let rows = stmt.query_map([format!("{ACTIVE_PREFIX}%")], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows.flatten() {
                let (key, encoded) = row;
                let workspace = key.strip_prefix(ACTIVE_PREFIX).unwrap_or_default();
                let blob = match decode(&encoded) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let trajectory = match trajectory_id(&blob) {
                    Some(v) => v,
                    None => continue,
                };
                let steps = decode_steps(&blob);
                let source = locator(&db, workspace);
                out.push(ProviderSessionSummary {
                    session_id: trajectory,
                    title: steps
                        .iter()
                        .find_map(|s| s.user_text.clone().or(s.visible.clone())),
                    project_dir: workspace_dir(&db, workspace),
                    last_active_at: modified_ms(&db),
                    source_path: Some(source),
                });
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
        out.dedup_by(|a, b| a.session_id == b.session_id);
        Ok(out)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let (db, workspace) = parse_locator(source_path)?;
        let encoded = read_active_value(&db, workspace)?;
        let blob = decode(&encoded)?;
        let id = trajectory_id(&blob).context("Windsurf trajectory has no UUID")?;
        let steps = decode_steps(&blob);
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let events = steps
            .iter()
            .enumerate()
            .filter_map(|(i, step)| map_step(step, i, &mut report))
            .collect();
        let mut extensions = BTreeMap::new();
        extensions.insert(
            "windsurf_trajectory_protobuf".into(),
            Value::String(encoded),
        );
        let now = modified_datetime(&db).unwrap_or_else(Utc::now);
        Ok(ImportedSession {
            session: CanonicalSession {
                schema: CanonicalSchema::default(),
                identity: SessionIdentity {
                    canonical_id: id.clone(),
                    source_title: steps
                        .iter()
                        .find_map(|s| s.user_text.clone().or(s.visible.clone())),
                },
                provenance: SessionProvenance {
                    imported_at: Utc::now(),
                    imported_by: Some("memorph-cli".into()),
                    primary_source: ProviderSessionRef {
                        provider_id: PROVIDER_ID.into(),
                        session_id: id,
                        source_path: Some(source_path.into()),
                    },
                    aliases: Vec::new(),
                },
                context: SessionContext {
                    workspace_dir: workspace_dir(&db, workspace),
                    created_at: Some(now),
                    last_active_at: Some(now),
                    tags: Vec::new(),
                },
                events,
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
        let (db, workspace) = parse_locator(source_path)?;
        if !db.exists() {
            return Ok(None);
        }
        let encoded = read_active_value(&db, workspace)?;
        let digest = Sha256::digest(encoded.as_bytes());
        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms: modified_ms(&db).unwrap_or(0),
            size_bytes: encoded.len().min(i64::MAX as usize) as i64,
            value: format!("windsurf-trajectory-v1:{digest:x}"),
        }))
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        state_paths()
    }
}

#[derive(Default)]
struct Step {
    id: Option<i64>,
    timestamp: Option<DateTime<Utc>>,
    user_text: Option<String>,
    visible: Option<String>,
    thinking: Option<String>,
    tools: Vec<Tool>,
    provider: Option<String>,
}
struct Tool {
    id: String,
    name: String,
    input: Option<Value>,
    result: Option<String>,
}

fn state_paths() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    ["Windsurf - Next", "Windsurf"]
        .into_iter()
        .map(|name| {
            home.join("Library/Application Support")
                .join(name)
                .join("User/globalStorage/state.vscdb")
        })
        .collect()
}
fn locator(db: &Path, workspace: &str) -> String {
    format!("{}#workspace={}", db.display(), workspace)
}
fn parse_locator(source: &str) -> Result<(PathBuf, &str)> {
    let (path, workspace) = source
        .rsplit_once("#workspace=")
        .context("Windsurf source locator must be state.vscdb#workspace=<id>")?;
    Ok((PathBuf::from(path), workspace))
}
fn read_active_value(db: &Path, workspace: &str) -> Result<String> {
    let conn = Connection::open(db)?;
    conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        [format!("{ACTIVE_PREFIX}{workspace}")],
        |r| r.get(0),
    )
    .with_context(|| format!("Windsurf active trajectory not found for workspace {workspace}"))
}
fn decode(encoded: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(encoded)
        .context("invalid Windsurf trajectory base64")
}
fn workspace_dir(db: &Path, workspace: &str) -> Option<String> {
    let p = db
        .parent()?
        .parent()?
        .join("workspaceStorage")
        .join(workspace)
        .join("workspace.json");
    let raw = std::fs::read_to_string(p).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    v.get("folder")
        .or_else(|| v.get("workspace"))
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

fn read_varint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut out = 0;
    for shift in (0..70).step_by(7) {
        let b = *bytes.get(*pos)?;
        *pos += 1;
        out |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(out);
        }
    }
    None
}
fn fields(bytes: &[u8]) -> Vec<(u32, u8, Vec<u8>, u64)> {
    let mut p = 0;
    let mut out = Vec::new();
    while p < bytes.len() {
        let tag = match read_varint(bytes, &mut p) {
            Some(v) => v,
            None => break,
        };
        let no = (tag >> 3) as u32;
        let wt = (tag & 7) as u8;
        match wt {
            0 => {
                if let Some(v) = read_varint(bytes, &mut p) {
                    out.push((no, wt, Vec::new(), v));
                }
            }
            2 => {
                let n = match read_varint(bytes, &mut p) {
                    Some(v) => v as usize,
                    None => break,
                };
                if p + n > bytes.len() {
                    break;
                }
                out.push((no, wt, bytes[p..p + n].to_vec(), 0));
                p += n;
            }
            _ => break,
        }
    }
    out
}
fn text(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec())
        .ok()
        .filter(|s| !s.is_empty())
}
fn trajectory_id(blob: &[u8]) -> Option<String> {
    fields(blob)
        .into_iter()
        .find(|(n, w, _, _)| *n == 1 && *w == 2)
        .and_then(|(_, _, b, _)| text(&b))
}
fn decode_steps(blob: &[u8]) -> Vec<Step> {
    let top = fields(blob);
    let container = top
        .iter()
        .find(|(n, w, _, _)| *n == 2 && *w == 2)
        .map(|x| x.2.as_slice())
        .unwrap_or(&[]);
    fields(container)
        .into_iter()
        .filter(|(_, w, _, _)| *w == 2)
        .filter_map(|(_, _, b, _)| decode_step(&b))
        .collect()
}
fn decode_step(blob: &[u8]) -> Option<Step> {
    let mut step = Step::default();
    for (n, w, b, v) in fields(blob) {
        if n == 1 && w == 0 {
            step.id = Some(v as i64)
        } else if n == 5 && w == 2 {
            step.timestamp = decode_timestamp(&b)
        } else if n == 19 && w == 2 {
            step.user_text = decode_text_message(&b)
        } else if n == 20 && w == 2 {
            decode_ai(&b, &mut step)
        } else if n == 28 && w == 2 {
            step.visible = step.visible.or_else(|| text(&b));
        }
    }
    Some(step)
}
fn decode_timestamp(blob: &[u8]) -> Option<DateTime<Utc>> {
    let f = fields(blob)
        .into_iter()
        .find(|(n, w, _, _)| *n == 1 && *w == 2)?
        .2;
    let fs = fields(&f);
    let sec = fs.iter().find(|(n, w, _, _)| *n == 1 && *w == 0)?.3 as i64;
    let nano = fs
        .iter()
        .find(|(n, w, _, _)| *n == 2 && *w == 0)
        .map(|x| x.3 as u32)
        .unwrap_or(0);
    DateTime::from_timestamp(sec, nano)
}
fn decode_text_message(blob: &[u8]) -> Option<String> {
    fields(blob)
        .into_iter()
        .filter(|(_, w, _, _)| *w == 2)
        .find_map(|(_, _, b, _)| text(&b))
}
fn decode_ai(blob: &[u8], step: &mut Step) {
    for (n, w, b, _) in fields(blob) {
        if w != 2 {
            continue;
        }
        if n == 3 {
            step.thinking = text(&b)
        } else if n == 8 {
            step.visible = text(&b)
        } else if n == 12 {
            step.provider = text(&b)
        } else if n == 7 {
            let fs = fields(&b);
            let id = fs
                .iter()
                .find(|(n, w, _, _)| *n == 1 && *w == 2)
                .and_then(|x| text(&x.2))
                .unwrap_or_else(|| "unknown".into());
            let name = fs
                .iter()
                .find(|(n, w, _, _)| *n == 2 && *w == 2)
                .and_then(|x| text(&x.2))
                .unwrap_or_else(|| "tool".into());
            let input = fs
                .iter()
                .find(|(n, w, _, _)| *n == 3 && *w == 2)
                .and_then(|x| text(&x.2))
                .and_then(|s| serde_json::from_str(&s).ok());
            step.tools.push(Tool {
                id,
                name,
                input,
                result: None,
            });
        }
    }
}
fn map_step(step: &Step, i: usize, r: &mut MappingReport) -> Option<SessionEvent> {
    let mut blocks = Vec::new();
    if let Some(t) = &step.user_text {
        blocks.push(EventBlock::Text { text: t.clone() })
    }
    if let Some(t) = &step.thinking {
        blocks.push(EventBlock::Thinking {
            text: t.clone(),
            signature: None,
        })
    }
    if let Some(t) = &step.visible {
        blocks.push(EventBlock::Text { text: t.clone() })
    }
    for tool in &step.tools {
        blocks.push(EventBlock::ToolCall {
            tool_call_id: tool.id.clone(),
            name: tool.name.clone(),
            input: tool.input.clone(),
        });
        if let Some(result) = &tool.result {
            blocks.push(EventBlock::ToolResult {
                tool_call_id: tool.id.clone(),
                content: result.clone(),
                is_error: false,
            })
        }
    }
    if blocks.is_empty() {
        return None;
    }
    let role = if step.user_text.is_some() {
        EventRole::User
    } else {
        EventRole::Assistant
    };
    let kind = if step.tools.is_empty() {
        SessionEventKind::Message
    } else {
        SessionEventKind::ToolCall
    };
    r.push_issue(crate::canonical::MappingIssue {
        level: crate::canonical::MappingIssueLevel::Info,
        disposition: MappingDisposition::Preserved,
        code: "windsurf-native-step".into(),
        message: "Mapped Windsurf trajectory step".into(),
        path: Some(format!("steps[{i}]")),
        raw: None,
    });
    Some(SessionEvent {
        id: format!("windsurf:event:{i}"),
        kind,
        role,
        timestamp: step.timestamp.unwrap_or_else(Utc::now),
        links: EventLinks::default(),
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.into(),
                original_id: step.id.map(|v| v.to_string()),
                original_role: None,
                phase: None,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: BTreeMap::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(no: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut tag = ((no as u64) << 3) | 2;
        while tag >= 0x80 {
            out.push((tag as u8) | 0x80);
            tag >>= 7;
        }
        out.push(tag as u8);
        out.push(payload.len() as u8);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn trajectory_identity_comes_from_protobuf_field_one() {
        let blob = field(1, b"trajectory-id");
        assert_eq!(trajectory_id(&blob).as_deref(), Some("trajectory-id"));
    }

    #[test]
    fn locator_keeps_workspace_identity_separate_from_database_path() {
        let (path, workspace) = parse_locator("/tmp/state.vscdb#workspace=abc").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/state.vscdb"));
        assert_eq!(workspace, "abc");
    }

    #[test]
    fn decodes_native_ai_text_thinking_and_tool_call() {
        let tool = [
            field(1, b"call-1"),
            field(2, b"read"),
            field(3, br#"{"path":"x"}"#),
        ]
        .concat();
        let ai = [field(3, b"thinking"), field(7, &tool), field(8, b"answer")].concat();
        let step = [field(1, b"\x01"), field(20, &ai)].concat();
        let decoded = decode_step(&step).unwrap();
        assert_eq!(decoded.thinking.as_deref(), Some("thinking"));
        assert_eq!(decoded.visible.as_deref(), Some("answer"));
        assert_eq!(decoded.tools[0].name, "read");
        assert_eq!(
            decoded.tools[0].input,
            Some(serde_json::json!({"path":"x"}))
        );
    }
}
