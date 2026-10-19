//! Grok Build (xAI coding CLI).
//!
//! Layout: `~/.grok/sessions/<url-encoded-cwd>/<uuid>/updates.jsonl` plus a
//! `summary.json` sidecar (`info.cwd`, `generated_title` / `session_summary`,
//! `created_at` / `updated_at` ISO, `current_model_id`). `updates.jsonl` is an
//! ACP-style stream — `{timestamp(unix sec), method:"session/update",
//! params:{update:{sessionUpdate:…}}}` — where text chunks are incremental
//! fragments that must be concatenated per role segment, never trimmed/joined
//! (token-boundary spaces would be eaten). `chat_history.jsonl` is a
//! compaction snapshot, not full history; `session_search.sqlite` and
//! `prompt_history.jsonl` are Grok's own index — none of these are session
//! transcripts.
//!
//! Semantics ported from the Wake reference implementation (real-machine
//! verified there). Known non-content updates (task_backgrounded, task_completed,
//! auto_compact_*, compaction_checkpoint, plan, current_mode_update) are skipped.
//!
//! ponytail: subagent sessions are imported standalone with their own summary
//! cwd; Wake additionally re-parents grok subagents via a worktree registry
//! (grok_group) — add if workspace attribution of child sessions ever matters.

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
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct GrokProvider;
const PROVIDER_ID: &str = "grok";

impl Provider for GrokProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "Grok Build"
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
                provider_payload: Some(Fidelity::Dropped),
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
                if path.file_name().and_then(|n| n.to_str()) != Some("updates.jsonl") {
                    continue;
                }
                let Some(id) = path.parent().and_then(|p| p.file_name()) else {
                    continue;
                };
                let summary = read_summary(path);
                let id = id.to_string_lossy().into_owned();
                sessions.push(ProviderSessionSummary {
                    archived: false,
                    session_id: id.clone(),
                    title: summary.title.filter(|t| !t.trim().is_empty()),
                    project_dir: summary.cwd,
                    created_at: summary.created_ms,
                    last_active_at: summary
                        .updated_ms
                        .or_else(|| modified_ms(path)),
                    source_path: Some(path.to_string_lossy().into_owned()),
                });
            }
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_active_at.unwrap_or(0)));
        Ok(sessions)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let path = Path::new(source_path);
        anyhow::ensure!(
            path.file_name().and_then(|n| n.to_str()) == Some("updates.jsonl"),
            "Grok source must be an updates.jsonl transcript: {source_path}"
        );
        let id = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .context("Grok source must live inside a session directory")?;
        let summary = read_summary(path);
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let mut events = parse_updates(path, &mut report)?;
        if let Some(model) = &summary.model {
            for event in events.iter_mut() {
                event.metadata.model = Some(model.clone());
            }
        }
        let title = summary
            .title
            .filter(|t| !t.trim().is_empty())
            .or_else(|| {
                events
                    .iter()
                    .find(|e| e.role == Role::User)
                    .and_then(|e| first_text(e))
            });
        let created = summary
            .created_ms
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .or_else(|| modified_datetime(path))
            .unwrap_or_else(Utc::now);
        let mut imported = ImportedSession {
            session: Session {
                lineage: Vec::new(),
                schema: Schema::default(),
                identity: Identity { id: id.clone(), title },
                context: Context {
                    workspace: summary.cwd,
                    created_at: Some(created),
                    last_active_at: summary
                        .updated_ms
                        .and_then(DateTime::<Utc>::from_timestamp_millis)
                        .or_else(|| modified_datetime(path)),
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
                    session_id: id,
                    source_path: Some(path.to_string_lossy().into_owned()),
                },
                aliases: Vec::new(),
            },
            event_meta: Vec::new(),
            report,
        };
        imported.event_meta = (0..imported.session.events.len())
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect();
        Ok(imported)
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
            value: format!("{PROVIDER_ID}-session-v1:{digest:x}"),
        }))
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        roots()
    }
}

fn roots() -> Vec<PathBuf> {
    dirs::home_dir()
        .into_iter()
        .map(|h| h.join(".grok/sessions"))
        .collect()
}

struct Summary {
    cwd: Option<String>,
    title: Option<String>,
    model: Option<String>,
    created_ms: Option<i64>,
    updated_ms: Option<i64>,
}

fn read_summary(updates_path: &Path) -> Summary {
    let mut s = Summary {
        cwd: None,
        title: None,
        model: None,
        created_ms: None,
        updated_ms: None,
    };
    let path = updates_path.with_file_name("summary.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return s;
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return s;
    };
    s.cwd = v
        .pointer("/info/cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    for key in ["generated_title", "session_summary"] {
        if let Some(t) = v.get(key).and_then(Value::as_str) {
            if !t.trim().is_empty() {
                s.title = Some(t.to_string());
                break;
            }
        }
    }
    s.model = v
        .get("current_model_id")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    s.created_ms = v
        .get("created_at")
        .and_then(Value::as_str)
        .and_then(iso_ms);
    s.updated_ms = v
        .get("updated_at")
        .and_then(Value::as_str)
        .and_then(iso_ms);
    s
}

/// Replay state: one segment per role turn; assistant segments accumulate
/// interleaved thought/message/tool chunks into a single event.
struct Replay {
    events: Vec<Event>,
    cur_role: Option<Role>,
    cur_ts: Option<DateTime<Utc>>,
    cur_blocks: Vec<Block>,
}

impl Replay {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            cur_role: None,
            cur_ts: None,
            cur_blocks: Vec::new(),
        }
    }

    fn flush(&mut self) {
        let Some(role) = self.cur_role.take() else {
            return;
        };
        let blocks = std::mem::take(&mut self.cur_blocks);
        if blocks.is_empty() {
            return;
        }
        let kind = if blocks.iter().any(|b| matches!(b, Block::ToolResult { .. })) {
            EventKind::Observation
        } else if blocks.iter().any(|b| matches!(b, Block::ToolCall { .. })) {
            EventKind::Action
        } else {
            EventKind::Message
        };
        self.events.push(Event {
            id: format!("grok:event:{}", self.events.len()),
            kind,
            role,
            timestamp: self.cur_ts.unwrap_or_else(Utc::now),
            links: Links::default(),
            blocks,
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        });
        self.cur_ts = None;
    }

    /// Enter (or continue) a role segment.
    fn ensure(&mut self, role: Role, ts: DateTime<Utc>) {
        if self.cur_role != Some(role) {
            self.flush();
            self.cur_role = Some(role);
            self.cur_ts = Some(ts);
        }
    }

    /// Append incremental text to the current segment's trailing Text block.
    fn append_text(&mut self, text: &str) {
        if let Some(Block::Text { text: acc }) = self.cur_blocks.last_mut() {
            acc.push_str(text);
        } else {
            self.cur_blocks.push(Block::Text {
                text: text.to_string(),
            });
        }
    }

    /// Append incremental thinking to the current segment's trailing Thinking block.
    fn append_thinking(&mut self, text: &str) {
        if let Some(Block::Thinking { text: acc, .. }) = self.cur_blocks.last_mut() {
            acc.push_str(text);
        } else {
            self.cur_blocks.push(Block::Thinking {
                text: text.to_string(),
                signature: None,
            });
        }
    }
}

fn chunk_text(update: &Value) -> &str {
    update
        .pointer("/content/text")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// Parse `updates.jsonl` into canonical events.
fn parse_updates(
    path: &Path,
    report: &mut MappingReport,
) -> Result<Vec<Event>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::with_capacity(1 << 20, file);
    let mut rp = Replay::new();
    let mut unknown = 0u32;

    for line in reader.lines() {
        let Ok(line) = line else {
            unknown += 1;
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            unknown += 1;
            continue;
        };
        let ts = row
            .get("timestamp")
            .and_then(|v| v.as_i64())
            .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
            .unwrap_or_else(Utc::now);
        let Some(update) = row.pointer("/params/update") else {
            unknown += 1;
            continue;
        };
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("user_message_chunk") => {
                rp.ensure(Role::User, ts);
                rp.append_text(chunk_text(update));
            }
            Some("agent_message_chunk") => {
                rp.ensure(Role::Assistant, ts);
                rp.append_text(chunk_text(update));
            }
            Some("agent_thought_chunk") => {
                rp.ensure(Role::Assistant, ts);
                rp.append_thinking(chunk_text(update));
            }
            Some("tool_call") => {
                rp.ensure(Role::Assistant, ts);
                rp.cur_blocks.push(Block::ToolCall {
                    tool_call_id: update
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .into(),
                    name: update
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .into(),
                    input: update.get("rawInput").filter(|r| !r.is_null()).cloned(),
                });
            }
            Some("tool_call_update") => {
                rp.ensure(Role::Assistant, ts);
                let id = update
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let content = update
                    .get("content")
                    .and_then(|c| c.pointer("/text"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default();
                let failed =
                    update.get("status").and_then(Value::as_str) == Some("failed");
                rp.cur_blocks.push(Block::ToolResult {
                    tool_call_id: id.into(),
                    content,
                    outcome: crate::session::execution_outcome(failed),
                });
            }
            // Known non-content updates.
            Some(
                "task_backgrounded"
                | "task_completed"
                | "auto_compact_started"
                | "auto_compact_completed"
                | "compaction_checkpoint"
                | "plan"
                | "current_mode_update"
            ) => {}
            _ => {
                unknown += 1;
            }
        }
    }
    rp.flush();
    if unknown > 0 {
        report.push_issue(crate::session::MappingIssue {
            level: crate::session::MappingIssueLevel::Info,
            disposition: Fidelity::Preserved,
            code: "grok-unknown-updates".into(),
            message: format!("{unknown} updates.jsonl rows carried no sessionUpdate payload"),
            path: None,
            raw: None,
        });
    }
    Ok(rp.events)
}

fn first_text(event: &Event) -> Option<String> {
    event.blocks.iter().find_map(|b| match b {
        Block::Text { text } => Some(text.clone()),
        _ => None,
    })
}

fn iso_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc).timestamp_millis())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(dir: &Path, rows: &[&str]) -> PathBuf {
        let session_dir = dir.join("9f2c1a44-1111-2222-3333-444455556666");
        std::fs::create_dir_all(&session_dir).unwrap();
        let updates = session_dir.join("updates.jsonl");
        std::fs::write(&updates, rows.join("\n") + "\n").unwrap();
        std::fs::write(
            session_dir.join("summary.json"),
            r#"{"info":{"cwd":"/tmp/grok-project"},"generated_title":"Refactor auth","created_at":"2026-09-20T10:00:00Z","updated_at":"2026-09-20T11:30:00Z"}"#,
        )
        .unwrap();
        updates
    }

    #[test]
    fn imports_segmented_transcript_with_tools_and_thinking() {
        let dir = tempfile::tempdir().unwrap();
        let updates = write_session(
            dir.path(),
            &[
                r#"{"timestamp":1789908000,"method":"session/update","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"run the "}}}}"#,
                r#"{"timestamp":1789908001,"method":"session/update","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"tests"}}}}"#,
                r#"{"timestamp":1789908002,"method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"checking deps"}}}}"#,
                r#"{"timestamp":1789908003,"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"on it"}}}}"#,
                r#"{"timestamp":1789908004,"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"tc-1","title":"shell","rawInput":{"command":"pytest"}}}}"#,
                r#"{"timestamp":1789908009,"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tc-1","content":{"type":"text","text":"3 passed"},"status":"completed"}}}"#,
                r#"{"timestamp":1789908010,"method":"session/update","params":{"update":{"sessionUpdate":"task_completed"}}}"#,
            ],
        );

        let imported = GrokProvider
            .import_session(updates.to_str().unwrap())
            .unwrap();
        assert_eq!(imported.session.identity.id, "9f2c1a44-1111-2222-3333-444455556666");
        assert_eq!(imported.session.identity.title.as_deref(), Some("Refactor auth"));
        assert_eq!(imported.session.context.workspace.as_deref(), Some("/tmp/grok-project"));
        assert_eq!(imported.session.events.len(), 2);

        let user = &imported.session.events[0];
        assert_eq!(user.role, Role::User);
        assert!(matches!(&user.blocks[0], Block::Text { text } if text == "run the tests"));

        let assistant = &imported.session.events[1];
        assert_eq!(assistant.role, Role::Assistant);
        // thinking + text + tool_call + tool_result all inside one segment
        assert_eq!(assistant.blocks.len(), 4);
        assert!(matches!(&assistant.blocks[0], Block::Thinking { text, .. } if text == "checking deps"));
        assert!(matches!(&assistant.blocks[2], Block::ToolCall { tool_call_id, name, .. } if tool_call_id == "tc-1" && name == "shell"));
        assert!(matches!(&assistant.blocks[3], Block::ToolResult { content, outcome, .. } if content == "3 passed" && !crate::session::execution_outcome_is_error(*outcome)));
    }

    #[test]
    fn scan_finds_updates_jsonl_with_sidecar_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let _ = write_session(
            dir.path(),
            &[r#"{"timestamp":1789908000,"method":"session/update","params":{"update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"hi"}}}}"#],
        );

        // scan reads the real root; emulate by checking the summary parser instead
        let updates = dir
            .path()
            .join("9f2c1a44-1111-2222-3333-444455556666/updates.jsonl");
        let summary = read_summary(&updates);
        assert_eq!(summary.cwd.as_deref(), Some("/tmp/grok-project"));
        assert_eq!(summary.created_ms, Some(1789898400000));
    }
}
