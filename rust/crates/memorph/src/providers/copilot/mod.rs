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

pub struct CopilotProvider;
const PROVIDER_ID: &str = "copilot";
const EVENTS_FILE: &str = "events.jsonl";
const CHAT_SESSIONS_DIR: &str = "chatSessions";

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
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Preserved),
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
        // Source 1: CLI session-state (events.jsonl)
        for root in cli_roots() {
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
                    archived: false,
                    session_id: id,
                    title,
                    project_dir: session_cwd(&events),
                    created_at: None,
                    last_active_at: file_modified_ms(path),
                    source_path: Some(path.to_string_lossy().into_owned()),
                });
            }
        }
        // Source 2: VS Code chatSessions
        for root in vscode_workspace_storage_roots() {
            if !root.exists() {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };
            for ws_entry in entries.flatten() {
                let ws_path = ws_entry.path();
                if !ws_path.is_dir() {
                    continue;
                }
                let chat_dir = ws_path.join(CHAT_SESSIONS_DIR);
                if !chat_dir.is_dir() {
                    continue;
                }
                let workspace = vscode_workspace_folder(&ws_path);
                let Ok(files) = std::fs::read_dir(&chat_dir) else {
                    continue;
                };
                for file_entry in files.flatten() {
                    let path = file_entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let ext = path.extension().and_then(|e| e.to_str());
                    if ext != Some("jsonl") && ext != Some("json") {
                        continue;
                    }
                    let Some(sid) = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(str::to_string)
                    else {
                        continue;
                    };
                    let title = vscode_session_title(&path);
                    out.push(ProviderSessionSummary {
                        archived: false,
                        session_id: sid,
                        title,
                        project_dir: workspace.clone(),
                        created_at: None,
                        last_active_at: file_modified_ms(&path),
                        source_path: Some(path.to_string_lossy().into_owned()),
                    });
                }
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
        out.dedup_by(|a, b| a.session_id == b.session_id);
        Ok(out)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        if is_vscode_chat_session(source_path) {
            return import_vscode_session(source_path);
        }
        import_cli_session(source_path)
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
        let prefix = if is_vscode_chat_session(source_path) {
            "copilot-vscode-v1"
        } else {
            "copilot-events-v1"
        };
        let id = if is_vscode_chat_session(source_path) {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string()
        } else {
            session_id(path).unwrap_or_default()
        };
        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms: file_modified_ms(path).unwrap_or(0),
            size_bytes: metadata.len().min(i64::MAX as u64) as i64,
            value: format!("{prefix}:{id}:{digest:x}"),
        }))
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        let mut paths = cli_roots();
        paths.extend(vscode_workspace_storage_roots());
        paths
    }
}

fn cli_roots() -> Vec<PathBuf> {
    dirs::home_dir()
        .into_iter()
        .map(|h| h.join(".copilot/session-state"))
        .collect()
}

fn vscode_workspace_storage_roots() -> Vec<PathBuf> {
    let Some(cfg) = dirs::config_dir() else {
        return Vec::new();
    };
    ["Code", "Code - Insiders", "VSCodium"]
        .iter()
        .map(|app| cfg.join(app).join("User").join("workspaceStorage"))
        .collect()
}

fn is_vscode_chat_session(source_path: &str) -> bool {
    (source_path.contains("/chatSessions/") || source_path.contains("\\chatSessions\\"))
        && (source_path.contains("/workspaceStorage/")
            || source_path.contains("\\workspaceStorage\\"))
}

fn vscode_workspace_folder(ws_path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(ws_path.join("workspace.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    v.get("folder").and_then(Value::as_str).map(str::to_string)
}

fn vscode_session_title(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let first_line = raw.lines().next()?;
    let header: Value = serde_json::from_str(first_line).ok()?;
    let state = if header.get("kind").and_then(Value::as_u64) == Some(0) {
        header.get("v")?
    } else {
        &header
    };
    // Try customTitle first, then first user request text
    if let Some(title) = state.get("customTitle").and_then(Value::as_str) {
        if !title.is_empty() {
            return Some(title.to_string());
        }
    }
    state
        .get("requests")
        .and_then(Value::as_array)?
        .first()?
        .get("message")
        .and_then(|m| m.get("text").and_then(Value::as_str).map(str::to_string))
}

fn replay_vscode_session(raw: &str) -> Result<Value> {
    let mut lines = raw.lines().filter(|l| !l.trim().is_empty());
    let first = lines.next().context("empty VS Code chat session file")?;
    let header: Value = serde_json::from_str(first).context("invalid VS Code session header")?;
    if header.get("kind").and_then(Value::as_u64) != Some(0) {
        // Legacy JSON format: entire file is the state
        return serde_json::from_str(raw).context("invalid VS Code session JSON");
    }
    let mut state = header
        .get("v")
        .cloned()
        .context("VS Code session snapshot has no `v` field")?;
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = entry.get("kind").and_then(Value::as_u64).unwrap_or(0);
        let Some(path) = entry.get("k").and_then(Value::as_array) else {
            continue;
        };
        match kind {
            1 => {
                if let Some(value) = entry.get("v").cloned() {
                    set_at_path(&mut state, path, value);
                }
            }
            2 => {
                delete_at_path(&mut state, path);
            }
            _ => {}
        }
    }
    Ok(state)
}

fn set_at_path(root: &mut Value, path: &[Value], value: Value) {
    if path.is_empty() {
        *root = value;
        return;
    }
    if path.len() == 1 {
        match &path[0] {
            Value::String(key) => {
                if let Some(obj) = root.as_object_mut() {
                    obj.insert(key.clone(), value);
                }
            }
            Value::Number(n) => {
                if let (Some(arr), Some(idx)) = (root.as_array_mut(), n.as_u64()) {
                    let idx = idx as usize;
                    if idx < arr.len() {
                        arr[idx] = value;
                    } else {
                        arr.resize(idx + 1, Value::Null);
                        arr[idx] = value;
                    }
                }
            }
            _ => {}
        }
        return;
    }
    let child = match &path[0] {
        Value::String(key) => root.as_object_mut().and_then(|obj| obj.get_mut(key)),
        Value::Number(n) => n.as_u64().and_then(|idx| {
            root.as_array_mut()
                .and_then(|arr| arr.get_mut(idx as usize))
        }),
        _ => None,
    };
    if let Some(child) = child {
        set_at_path(child, &path[1..], value);
    }
}

fn delete_at_path(root: &mut Value, path: &[Value]) {
    if path.is_empty() {
        return;
    }
    if path.len() == 1 {
        match &path[0] {
            Value::String(key) => {
                if let Some(obj) = root.as_object_mut() {
                    obj.remove(key);
                }
            }
            Value::Number(n) => {
                if let (Some(arr), Some(idx)) = (root.as_array_mut(), n.as_u64()) {
                    let idx = idx as usize;
                    if idx < arr.len() {
                        arr.remove(idx);
                    }
                }
            }
            _ => {}
        }
        return;
    }
    let child = match &path[0] {
        Value::String(key) => root.as_object_mut().and_then(|obj| obj.get_mut(key)),
        Value::Number(n) => n.as_u64().and_then(|idx| {
            root.as_array_mut()
                .and_then(|arr| arr.get_mut(idx as usize))
        }),
        _ => None,
    };
    if let Some(child) = child {
        delete_at_path(child, &path[1..]);
    }
}

fn import_vscode_session(source_path: &str) -> Result<ImportedSession> {
    let path = Path::new(source_path);
    if !path.exists() {
        anyhow::bail!("VS Code chat session does not exist: {source_path}");
    }
    let raw = std::fs::read_to_string(path)?;
    let state = replay_vscode_session(&raw)?;
    let sid = state
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string());
    let creation_ms = state.get("creationDate").and_then(Value::as_u64);
    let title = state
        .get("customTitle")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| vscode_first_user_text(&state));
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let events = vscode_state_to_events(&state, &mut report);
    let event_meta = events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
        .collect::<Vec<_>>();
    let timestamp = creation_ms
        .and_then(|ms| DateTime::<Utc>::from_timestamp_millis(ms as i64))
        .or_else(|| file_modified_datetime(path))
        .unwrap_or_else(Utc::now);
    let workspace = path
        .parent()
        .and_then(|chat_dir| chat_dir.parent())
        .and_then(|ws| vscode_workspace_folder(ws));
    let mut extensions = BTreeMap::new();
    extensions.insert("copilot_vscode_state".into(), state);
    Ok(ImportedSession {
        session: Session {
            lineage: Vec::new(),
            schema: Schema::default(),
            identity: Identity {
                id: sid.clone(),
                title,
            },
            context: Context {
                workspace,
                created_at: Some(timestamp),
                last_active_at: file_modified_datetime(path),
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
                session_id: sid,
                source_path: Some(source_path.into()),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    })
}

fn import_cli_session(source_path: &str) -> Result<ImportedSession> {
    let path = std::fs::canonicalize(source_path)
        .with_context(|| format!("Copilot source does not exist: {source_path}"))?;
    let id = session_id(&path).context("Copilot source must be session-state/<id>/events.jsonl")?;
    let events = read_events(&path)?;
    let timestamp = events
        .iter()
        .find_map(event_time)
        .or_else(|| file_modified_datetime(&path))
        .unwrap_or_else(Utc::now);
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let canonical_events: Vec<Event> = events
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
    let event_meta = canonical_events
        .iter()
        .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
        .collect::<Vec<_>>();
    Ok(ImportedSession {
        session: Session {
            lineage: Vec::new(),
            schema: Schema::default(),
            identity: Identity {
                id: id.clone(),
                title,
            },
            context: Context {
                workspace: session_cwd_from_value(&extensions["copilot_events_jsonl"]),
                created_at: Some(timestamp),
                last_active_at: file_modified_datetime(&path),
                tags: Vec::new(),
            },
            events: canonical_events,
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

fn vscode_first_user_text(state: &Value) -> Option<String> {
    state
        .get("requests")
        .and_then(Value::as_array)?
        .first()?
        .get("message")?
        .get("text")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn vscode_state_to_events(state: &Value, report: &mut MappingReport) -> Vec<Event> {
    let Some(requests) = state.get("requests").and_then(Value::as_array) else {
        return Vec::new();
    };
    let base_ts = state
        .get("creationDate")
        .and_then(Value::as_u64)
        .and_then(|ms| DateTime::<Utc>::from_timestamp_millis(ms as i64))
        .unwrap_or_else(Utc::now);
    let mut events = Vec::new();
    for (idx, req) in requests.iter().enumerate() {
        let req_ts = req
            .get("timestamp")
            .and_then(Value::as_u64)
            .and_then(|ms| DateTime::<Utc>::from_timestamp_millis(ms as i64))
            .unwrap_or(base_ts);
        // User message
        if let Some(text) = req
            .get("message")
            .and_then(|m| m.get("text").and_then(Value::as_str))
        {
            if !text.is_empty() {
                report.push_issue(crate::session::MappingIssue {
                    level: crate::session::MappingIssueLevel::Info,
                    disposition: Fidelity::Preserved,
                    code: "copilot-vscode-user".into(),
                    message: "Mapped VS Code Copilot Chat user request".into(),
                    path: Some(format!("requests[{idx}].message")),
                    raw: None,
                });
                events.push(Event {
                    id: format!("copilot:vscode:req:{idx}:user"),
                    kind: EventKind::Message,
                    role: Role::User,
                    timestamp: req_ts,
                    links: Links::default(),
                    blocks: vec![Block::Text {
                        text: text.to_string(),
                    }],
                    tags: Vec::new(),
                    extensions: Default::default(),
                    metadata: Metadata {
                        model: None,
                        usage: None,
                    },
                });
            }
        }
        // Assistant response
        let Some(response) = req.get("response").and_then(Value::as_array) else {
            continue;
        };
        let mut blocks = Vec::new();
        for part in response {
            let kind = part.get("kind").and_then(Value::as_str);
            match kind {
                None => {
                    if let Some(text) = part.get("value").and_then(Value::as_str) {
                        if !text.is_empty() {
                            blocks.push(Block::Text {
                                text: text.to_string(),
                            });
                        }
                    }
                }
                Some("thinking") => {
                    if let Some(text) = part.get("value").and_then(Value::as_str) {
                        if !text.is_empty() {
                            blocks.push(Block::Thinking {
                                text: text.to_string(),
                                signature: None,
                            });
                        }
                    }
                }
                Some("toolInvocationSerialized") => {
                    let tool_name = part
                        .get("toolId")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string();
                    let call_id = part
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    blocks.push(Block::ToolCall {
                        tool_call_id: call_id,
                        name: tool_name,
                        input: part.get("rawInput").cloned(),
                    });
                }
                _ => {}
            }
        }
        if !blocks.is_empty() {
            report.push_issue(crate::session::MappingIssue {
                level: crate::session::MappingIssueLevel::Info,
                disposition: Fidelity::Preserved,
                code: "copilot-vscode-assistant".into(),
                message: "Mapped VS Code Copilot Chat assistant response".into(),
                path: Some(format!("requests[{idx}].response")),
                raw: None,
            });
            let kind = if blocks.iter().any(|b| matches!(b, Block::ToolCall { .. })) {
                EventKind::Action
            } else {
                EventKind::Message
            };
            events.push(Event {
                id: format!("copilot:vscode:req:{idx}:assistant"),
                kind,
                role: Role::Assistant,
                timestamp: req_ts,
                links: Links::default(),
                blocks,
                tags: Vec::new(),
                extensions: Default::default(),
                metadata: Metadata {
                    model: req
                        .get("response")
                        .and_then(|r| r.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|p| p.get("modelId"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    usage: None,
                },
            });
        }
    }
    events
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
    report.push_issue(crate::session::MappingIssue {
        level: crate::session::MappingIssueLevel::Info,
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
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: event
                .get("data")
                .and_then(|d| d.get("model"))
                .and_then(Value::as_str)
                .map(str::to_string),
            usage: None,
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
    #[test]
    fn detects_vscode_chat_session_path() {
        assert!(is_vscode_chat_session(
            "/home/user/.config/Code/User/workspaceStorage/abc/chatSessions/uuid.jsonl"
        ));
        assert!(!is_vscode_chat_session(
            "/home/user/.copilot/session-state/abc/events.jsonl"
        ));
    }
    #[test]
    fn imports_vscode_chat_session_patch_log() {
        let tmp = tempfile::tempdir().unwrap();
        let chat_dir = tmp.path().join("workspaceStorage/abc/chatSessions");
        std::fs::create_dir_all(&chat_dir).unwrap();
        std::fs::write(
            tmp.path().join("workspaceStorage/abc/workspace.json"),
            r#"{"folder":"file:///tmp/project"}"#,
        )
        .unwrap();
        let session_file = chat_dir.join("sess-001.jsonl");
        let state = serde_json::json!({
            "sessionId": "sess-001",
            "creationDate": 1700000000000u64,
            "customTitle": "Test session",
            "requests": [{
                "message": {"text": "hello copilot"},
                "timestamp": 1700000001000u64,
                "response": [{"value": "Hi there!"}]
            }]
        });
        let line = format!("{{\"kind\":0,\"v\":{}}}\n", state);
        std::fs::write(&session_file, line).unwrap();
        let imported = import_vscode_session(&session_file.to_string_lossy()).unwrap();
        assert_eq!(imported.session.identity.id, "sess-001");
        assert_eq!(
            imported.session.identity.title.as_deref(),
            Some("Test session")
        );
        assert_eq!(imported.session.events.len(), 2);
        assert_eq!(imported.session.events[0].role, Role::User);
        assert_eq!(imported.session.events[1].role, Role::Assistant);
    }
    #[test]
    fn vscode_tool_invocation_maps_to_action_with_tool_call_block() {
        let tmp = tempfile::tempdir().unwrap();
        let chat_dir = tmp.path().join("workspaceStorage/abc/chatSessions");
        std::fs::create_dir_all(&chat_dir).unwrap();
        std::fs::write(
            tmp.path().join("workspaceStorage/abc/workspace.json"),
            r#"{"folder":"file:///tmp/project"}"#,
        )
        .unwrap();
        let session_file = chat_dir.join("sess-002.jsonl");
        let state = serde_json::json!({
            "sessionId": "sess-002",
            "creationDate": 1700000000000u64,
            "requests": [{
                "message": {"text": "run the tests"},
                "timestamp": 1700000001000u64,
                "response": [
                    {"kind": "toolInvocationSerialized", "toolId": "runInTerminal", "toolCallId": "tc-1", "rawInput": {"command": "npm test"}}
                ]
            }]
        });
        let v = serde_json::to_string(&state).unwrap();
        let line = format!(r#"{{"kind":0,"v":{}}}"#, v);
        std::fs::write(&session_file, line).unwrap();
        let imported = import_vscode_session(&session_file.to_string_lossy()).unwrap();
        let assistant = imported
            .session
            .events
            .iter()
            .find(|e| e.role == Role::Assistant)
            .expect("assistant event");
        assert_eq!(assistant.kind, EventKind::Action);
        assert!(matches!(
            &assistant.blocks[0],
            Block::ToolCall { ref name, ref tool_call_id, .. }
                if name == "runInTerminal" && tool_call_id == "tc-1"
        ));
    }
}
