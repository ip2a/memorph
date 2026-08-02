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
                    created_at: None,
                    last_active_at: modified_ms(&db),
                    source_path: Some(source),
                });
            }
        }
        for root in legacy_paths() {
            let Ok(files) = std::fs::read_dir(root) else {
                continue;
            };
            for path in files
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("pbtxt"))
            {
                for (id, title, last_active_at) in legacy_sessions(&path)? {
                    out.push(ProviderSessionSummary {
                        session_id: id.clone(),
                        title,
                        project_dir: None,
                        created_at: None,
                        last_active_at,
                        source_path: Some(legacy_locator(&path, &id)),
                    });
                }
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
        out.dedup_by(|a, b| a.session_id == b.session_id);
        Ok(out)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        if source_path.contains("#conversation=") {
            return import_legacy_session(source_path);
        }
        let (db, workspace) = parse_locator(source_path)?;
        let encoded = read_active_value(&db, workspace)?;
        let blob = decode(&encoded)?;
        let id = trajectory_id(&blob).context("Windsurf trajectory has no UUID")?;
        let steps = decode_steps(&blob);
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let events: Vec<Event> = steps
            .iter()
            .enumerate()
            .filter_map(|(i, step)| map_step(step, i, &mut report))
            .collect();
        let event_meta = events
            .iter()
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect::<Vec<_>>();
        let mut extensions = BTreeMap::new();
        extensions.insert(
            "windsurf_trajectory_protobuf".into(),
            Value::String(encoded),
        );
        let now = modified_datetime(&db).unwrap_or_else(Utc::now);
        Ok(ImportedSession {
            session: Session {
                schema: Schema::default(),
                identity: Identity {
                    id: id.clone(),
                    title: steps
                        .iter()
                        .find_map(|s| s.user_text.clone().or(s.visible.clone())),
                },
                context: Context {
                    workspace: workspace_dir(&db, workspace),
                    created_at: Some(now),
                    last_active_at: Some(now),
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
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        if source_path.contains("#conversation=") {
            return legacy_fingerprint(source_path);
        }
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
        state_paths().into_iter().chain(legacy_paths()).collect()
    }
}

fn legacy_paths() -> Vec<PathBuf> {
    dirs::home_dir()
        .into_iter()
        .map(|home| home.join(".codeium/chat_state"))
        .collect()
}
fn legacy_locator(path: &Path, id: &str) -> String {
    format!("{}#conversation={id}", path.display())
}
fn parse_legacy_locator(source: &str) -> Result<(PathBuf, &str)> {
    let (path, id) = source
        .rsplit_once("#conversation=")
        .context("Windsurf legacy source locator must be chat_state file#conversation=<id>")?;
    Ok((PathBuf::from(path), id))
}
fn legacy_blocks<'a>(raw: &'a str, field: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(offset) = raw[start..].find(field) {
        let field_end = start + offset + field.len();
        let rest = &raw[field_end..];
        let Some(after_colon) = rest.trim_start().strip_prefix(':') else {
            start = field_end;
            continue;
        };
        let whitespace = after_colon.len() - after_colon.trim_start().len();
        if !after_colon[whitespace..].starts_with('{') {
            start = field_end;
            continue;
        }
        let colon_offset = rest.len() - rest.trim_start().len();
        let open = field_end + colon_offset + 1 + whitespace;
        let mut depth = 0;
        let mut quoted = false;
        let bytes = raw.as_bytes();
        let mut end = None;
        for i in open..raw.len() {
            match bytes[i] as char {
                '"' if i == 0 || bytes[i - 1] != b'\\' => quoted = !quoted,
                '{' if !quoted => depth += 1,
                '}' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        out.push(&raw[open + 1..end]);
        start = end + 1;
    }
    out
}
fn legacy_string(block: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    let start = block.find(&prefix)? + prefix.len();
    let rest = block[start..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (i, c) in rest[1..].char_indices() {
        if c == '"' && !escaped {
            return Some(rest[1..i + 1].replace("\\n", "\n").replace("\\\"", "\""));
        }
        escaped = c == '\\' && !escaped;
        if c != '\\' {
            escaped = false;
        }
    }
    None
}
type LegacySession = (String, Option<String>, Option<i64>);

fn legacy_sessions(path: &Path) -> Result<Vec<LegacySession>> {
    let raw = std::fs::read_to_string(path)?;
    let mut out = BTreeMap::new();
    for message in legacy_blocks(&raw, "message") {
        if !message.contains("CHAT_MESSAGE_SOURCE_USER") {
            continue;
        }
        let Some(id) = legacy_string(message, "conversation_id") else {
            continue;
        };
        let title = legacy_blocks(message, "intent")
            .into_iter()
            .find_map(|b| legacy_string(b, "text"));
        out.entry(id).or_insert((title, modified_ms(path)));
    }
    Ok(out
        .into_iter()
        .map(|(id, (title, modified))| (id, title, modified))
        .collect())
}
fn import_legacy_session(source: &str) -> Result<ImportedSession> {
    let (path, id) = parse_legacy_locator(source)?;
    let raw = std::fs::read_to_string(&path)?;
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    for (i, message) in legacy_blocks(&raw, "message").into_iter().enumerate() {
        if !message.contains("CHAT_MESSAGE_SOURCE_USER")
            || legacy_string(message, "conversation_id").as_deref() != Some(id)
        {
            continue;
        }
        let Some(text) = legacy_blocks(message, "intent")
            .into_iter()
            .find_map(|b| legacy_string(b, "text"))
        else {
            continue;
        };
        report.push_issue(crate::session::MappingIssue {
            level: crate::session::MappingIssueLevel::Info,
            disposition: Fidelity::Preserved,
            code: "windsurf-legacy-pbtxt".into(),
            message: "Mapped Windsurf legacy chat_state user message".into(),
            path: Some(format!("message[{i}]")),
            raw: None,
        });
        events.push(Event {
            id: format!("windsurf:legacy:{i}"),
            kind: EventKind::Message,
            role: Role::User,
            timestamp: modified_datetime(&path).unwrap_or_else(Utc::now),
            links: Links::default(),
            blocks: vec![Block::Text { text }],
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        });
    }
    let event_meta = events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
        .collect::<Vec<_>>();
    Ok(ImportedSession {
        session: Session {
            schema: Schema::default(),
            identity: Identity {
                id: id.into(),
                title: None,
            },
            context: Context {
                workspace: None,
                created_at: modified_datetime(&path),
                last_active_at: modified_datetime(&path),
                tags: Vec::new(),
            },
            events,
            extensions: BTreeMap::new(),
        },
        provenance: Provenance {
            imported_at: Utc::now(),
            imported_by: Some("memorph-cli".into()),
            primary_source: ProviderRef {
                provider_id: PROVIDER_ID.into(),
                session_id: id.into(),
                source_path: Some(source.into()),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    })
}
fn legacy_fingerprint(source: &str) -> Result<Option<ProviderSourceFingerprint>> {
    let (path, id) = parse_legacy_locator(source)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read(&path)?;
    let digest = Sha256::digest(&raw);
    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms: modified_ms(&path).unwrap_or(0),
        size_bytes: raw.len().min(i64::MAX as usize) as i64,
        value: format!("windsurf-legacy-pbtxt-v1:{id}:{digest:x}"),
    }))
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
fn map_step(step: &Step, i: usize, r: &mut MappingReport) -> Option<Event> {
    let mut blocks = Vec::new();
    if let Some(t) = &step.user_text {
        blocks.push(Block::Text { text: t.clone() })
    }
    if let Some(t) = &step.thinking {
        blocks.push(Block::Thinking {
            text: t.clone(),
            signature: None,
        })
    }
    if let Some(t) = &step.visible {
        blocks.push(Block::Text { text: t.clone() })
    }
    for tool in &step.tools {
        blocks.push(Block::ToolCall {
            tool_call_id: tool.id.clone(),
            name: tool.name.clone(),
            input: tool.input.clone(),
        });
        if let Some(result) = &tool.result {
            blocks.push(Block::ToolResult {
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
        Role::User
    } else {
        Role::Assistant
    };
    let kind = if step.tools.is_empty() {
        EventKind::Message
    } else {
        EventKind::Action
    };
    r.push_issue(crate::session::MappingIssue {
        level: crate::session::MappingIssueLevel::Info,
        disposition: Fidelity::Preserved,
        code: "windsurf-native-step".into(),
        message: "Mapped Windsurf trajectory step".into(),
        path: Some(format!("steps[{i}]")),
        raw: None,
    });
    Some(Event {
        id: format!("windsurf:event:{i}"),
        kind,
        role,
        timestamp: step.timestamp.unwrap_or_else(Utc::now),
        links: Links::default(),
        blocks,
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
    fn capabilities_match_read_only_source_boundary() {
        let capabilities = WindsurfProvider.capabilities();
        assert!(capabilities.scan);
        assert!(capabilities.import);
        assert!(!capabilities.export);
        assert!(!capabilities.delete);
        assert!(!capabilities.rename);
        assert!(!capabilities.resume);
        assert!(!capabilities.backup_support.before_write);
        assert!(!capabilities.backup_support.restore);
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
    fn imports_legacy_chat_state_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.pbtxt");
        std::fs::write(
            &path,
            r#"message: {source: CHAT_MESSAGE_SOURCE_USER conversation_id: "legacy-1" timestamp: {seconds: 10} intent: {text: "hello"}}
message:{source: CHAT_MESSAGE_SOURCE_USER conversation_id: "other" intent:{text: "skip"}}
"#,
        )
        .unwrap();
        let sessions = legacy_sessions(&path).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|session| session.0 == "legacy-1"));
        let imported = import_legacy_session(&legacy_locator(&path, "legacy-1")).unwrap();
        assert_eq!(imported.session.events.len(), 1);
        assert!(matches!(
            imported.session.events[0].blocks[0],
            Block::Text { ref text } if text == "hello"
        ));
        assert!(legacy_fingerprint(&legacy_locator(&path, "legacy-1"))
            .unwrap()
            .is_some());
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
