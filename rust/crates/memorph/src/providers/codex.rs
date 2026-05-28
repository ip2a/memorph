use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::provider::{
    canonical_block_text, canonical_event_text, canonical_export_result, canonical_session_title,
    Provider, ProviderCapabilities, ProviderSessionSummary,
};
use crate::utils;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct CodexProvider;

const PROVIDER_ID: &str = "codex";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodexWorkspaceRepairItem {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub rollout_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_model_provider: Option<String>,
    pub current_model_provider: String,
    pub updated_model_provider: bool,
    pub added_to_index: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodexWorkspaceRepairReport {
    pub workspace_dir: String,
    pub current_model_provider: String,
    pub scanned_rollouts: usize,
    pub workspace_session_count: usize,
    pub hidden_session_count: usize,
    pub repaired_session_count: usize,
    pub reindexed_session_count: usize,
    pub touched_sessions: Vec<CodexWorkspaceRepairItem>,
}

impl Provider for CodexProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "codex"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_session_management()
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let index_path = get_codex_dir().join("session_index.jsonl");
        if !index_path.exists() {
            return Ok(Vec::new());
        }

        // Build a lookup from id -> cwd from SQLite for fast access
        let cwd_lookup = build_cwd_lookup().unwrap_or_default();

        let file = File::open(&index_path).with_context(|| {
            format!(
                "Failed to open Codex session index: {}",
                index_path.display()
            )
        })?;
        let reader = BufReader::new(file);
        let mut sessions = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let id = value
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let thread_name = value
                .get("thread_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let updated_at = value
                .get("updated_at")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis());

            if let Some(id) = id {
                // Find the actual file path for this session
                let source_path = find_session_file(&id);
                // Get cwd from SQLite lookup, or fall back to scanning file
                let project_dir = cwd_lookup
                    .get(&id)
                    .cloned()
                    .or_else(|| extract_cwd_from_session_file(&id));

                sessions.push(ProviderSessionSummary {
                    session_id: id.clone(),
                    title: thread_name,
                    project_dir,
                    last_active_at: updated_at,
                    source_path: source_path.map(|p| p.to_string_lossy().to_string()),
                });
            }
        }

        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        import_canonical_session(Path::new(source_path))
    }

    fn export_session(
        &self,
        session: &CanonicalSession,
        target_dir: &Path,
    ) -> Result<ExportedSession> {
        let session_id = export_canonical_session(session, target_dir)?;
        Ok(canonical_export_result(
            PROVIDER_ID,
            session_id.clone(),
            self.resume_command(&session_id),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        // Find the session file
        let session_path = find_session_file(session_id)
            .with_context(|| format!("Codex session not found: {}", session_id))?;

        // Remove the JSONL file
        std::fs::remove_file(&session_path).with_context(|| {
            format!("Failed to remove session file: {}", session_path.display())
        })?;

        // Update session_index.jsonl to remove the entry
        let index_path = get_codex_dir().join("session_index.jsonl");
        if index_path.exists() {
            let content = std::fs::read_to_string(&index_path)?;
            let mut new_lines = Vec::new();
            for line in content.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    if v.get("id").and_then(|v| v.as_str()) == Some(session_id) {
                        continue;
                    }
                }
                new_lines.push(line.to_string());
            }
            std::fs::write(&index_path, new_lines.join("\n") + "\n")?;
        }

        // Remove from SQLite
        let db_path = get_codex_dir().join("state_5.sqlite");
        if db_path.exists() {
            let mut conn = Connection::open(&db_path)?;
            let tx = conn.transaction()?;
            delete_related_rows(&tx, "thread_dynamic_tools", "thread_id = ?1", session_id)?;
            delete_related_rows(&tx, "thread_goals", "thread_id = ?1", session_id)?;
            delete_related_rows(
                &tx,
                "thread_spawn_edges",
                "parent_thread_id = ?1 OR child_thread_id = ?1",
                session_id,
            )?;
            delete_related_rows(&tx, "stage1_outputs", "thread_id = ?1", session_id)?;
            if has_table(&tx, "agent_job_items")?
                && has_columns(&tx, "agent_job_items", &["assigned_thread_id"])?
            {
                let _ = tx.execute(
                    "UPDATE agent_job_items SET assigned_thread_id = NULL WHERE assigned_thread_id = ?1",
                    [session_id],
                );
            }
            let _ = tx.execute("DELETE FROM threads WHERE id = ?1", [session_id]);
            tx.commit()?;
        }

        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        // Update session_index.jsonl
        let index_path = get_codex_dir().join("session_index.jsonl");
        if !index_path.exists() {
            anyhow::bail!("Codex session index not found");
        }

        let content = std::fs::read_to_string(&index_path)?;
        let mut new_lines = Vec::new();
        let mut found = false;

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut v: Value = serde_json::from_str(line)?;
            if v.get("id").and_then(|v| v.as_str()) == Some(session_id) {
                if let Value::Object(ref mut map) = v {
                    map.insert(
                        "thread_name".to_string(),
                        Value::String(new_title.to_string()),
                    );
                    found = true;
                }
                new_lines.push(serde_json::to_string(&v)?);
            } else {
                new_lines.push(line.to_string());
            }
        }

        if !found {
            anyhow::bail!("Codex session not found in index: {}", session_id);
        }

        std::fs::write(&index_path, new_lines.join("\n") + "\n")?;
        if let Some(session_path) = find_session_file(session_id) {
            update_rollout_session_meta_title(&session_path, new_title)?;
        }
        let db_path = get_codex_dir().join("state_5.sqlite");
        if db_path.exists() {
            let conn = Connection::open(&db_path)?;
            if has_table(&conn, "threads")? && has_columns(&conn, "threads", &["title"])? {
                let _ = conn.execute(
                    "UPDATE threads SET title = ?1 WHERE id = ?2",
                    [new_title, session_id],
                );
            }
        }
        Ok(())
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("codex resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        if let Some(path) = find_session_file(session_id) {
            if path.exists() {
                return Ok(std::fs::metadata(path)?.len());
            }
        }
        Ok(0)
    }
}

pub fn repair_workspace_sessions(workspace: Option<&str>) -> Result<CodexWorkspaceRepairReport> {
    repair_workspace_sessions_in_codex_home(&get_codex_dir(), workspace)
}

fn repair_workspace_sessions_in_codex_home(
    codex_dir: &Path,
    workspace: Option<&str>,
) -> Result<CodexWorkspaceRepairReport> {
    let workspace_root = crate::config::resolve_workspace(workspace)?;
    let workspace_key = crate::provider::default_normalized_workspace_key(workspace_root.to_str())
        .with_context(|| {
            format!(
                "Failed to normalize workspace path: {}",
                workspace_root.display()
            )
        })?;
    let current_model_provider = read_codex_model_provider(codex_dir);
    let mut report = CodexWorkspaceRepairReport {
        workspace_dir: utils::user_visible_path(&workspace_key),
        current_model_provider: current_model_provider.clone(),
        scanned_rollouts: 0,
        workspace_session_count: 0,
        hidden_session_count: 0,
        repaired_session_count: 0,
        reindexed_session_count: 0,
        touched_sessions: Vec::new(),
    };

    let index_path = codex_dir.join("session_index.jsonl");
    let mut indexed_session_ids = load_session_index_ids(&index_path)?;
    let sessions_root = codex_dir.join("sessions");

    if !sessions_root.exists() {
        update_codex_global_state_file_if_exists(codex_dir, &workspace_root)?;
        return Ok(report);
    }

    for entry in WalkDir::new(&sessions_root)
        .max_depth(5)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        report.scanned_rollouts += 1;
        let Some(mut session) = read_codex_rollout_summary(path)? else {
            continue;
        };

        if !crate::provider::default_workspace_matches(
            session.workspace_dir.as_deref(),
            Some(&workspace_key),
        ) {
            continue;
        }

        report.workspace_session_count += 1;
        let provider_mismatch =
            session.model_provider.as_deref() != Some(current_model_provider.as_str());
        if provider_mismatch {
            report.hidden_session_count += 1;
        }

        let mut updated_model_provider = false;
        if provider_mismatch {
            rewrite_rollout_model_provider(path, &current_model_provider)?;
            session.model_provider = Some(current_model_provider.clone());
            updated_model_provider = true;
            report.repaired_session_count += 1;
        }

        let mut added_to_index = false;
        if !indexed_session_ids.contains(&session.session_id) {
            append_session_index_entry(
                &index_path,
                &session.session_id,
                session.title.as_deref().unwrap_or(&session.session_id),
                session.updated_at.as_deref(),
            )?;
            indexed_session_ids.insert(session.session_id.clone());
            added_to_index = true;
            report.reindexed_session_count += 1;
        }

        if updated_model_provider || added_to_index {
            report.touched_sessions.push(CodexWorkspaceRepairItem {
                session_id: session.session_id,
                title: session.title,
                rollout_path: utils::user_visible_path(&path.to_string_lossy()),
                workspace_dir: session.workspace_dir.as_deref().map(utils::user_visible_path),
                previous_model_provider: session.original_model_provider,
                current_model_provider: current_model_provider.clone(),
                updated_model_provider,
                added_to_index,
            });
        }
    }

    update_codex_global_state_file_if_exists(codex_dir, &workspace_root)?;
    Ok(report)
}

fn import_canonical_session(path: &Path) -> Result<ImportedSession> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<chrono::DateTime<Utc>> = None;
    let mut last_active_at: Option<chrono::DateTime<Utc>> = None;
    let mut source_title: Option<String> = None;
    let mut extensions = BTreeMap::new();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Codex session line: {}", error),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let timestamp = value
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        last_active_at = Some(timestamp);

        match line_type.as_str() {
            "session_meta" => {
                if let Some(payload) = value.get("payload") {
                    session_id = payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(session_id);
                    project_dir = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(project_dir);
                    created_at = payload
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .or(created_at);

                    if let Some(text) = payload
                        .get("base_instructions")
                        .and_then(|v| v.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        events.push(SessionEvent {
                            id: format!("codex:base_instructions:{}", line_idx + 1),
                            kind: SessionEventKind::Lifecycle,
                            role: EventRole::System,
                            timestamp,
                            links: EventLinks::default(),
                            blocks: vec![
                                EventBlock::Text {
                                    text: text.to_string(),
                                },
                                EventBlock::ProviderPayload {
                                    kind: "session_meta".to_string(),
                                    payload: payload.clone(),
                                },
                            ],
                            metadata: EventMetadata {
                                source: EventSource {
                                    provider_id: PROVIDER_ID.to_string(),
                                    original_id: None,
                                    original_role: Some("developer".to_string()),
                                    phase: None,
                                },
                                model: payload
                                    .get("model")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string),
                                usage: None,
                                fidelity: MappingDisposition::Preserved,
                                provider_ext: {
                                    let mut ext = BTreeMap::new();
                                    ext.insert("codex_raw_line".to_string(), value.clone());
                                    ext
                                },
                            },
                        });
                    } else {
                        events.push(provider_payload_event(
                            format!("codex:session_meta:{}", line_idx + 1),
                            SessionEventKind::Lifecycle,
                            EventRole::System,
                            timestamp,
                            "session_meta",
                            payload.clone(),
                            value.clone(),
                            None,
                        ));
                    }

                    source_title = payload
                        .get("title")
                        .or_else(|| payload.get("thread_name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(source_title);
                    extensions.insert("codex_session_meta".to_string(), payload.clone());
                }
            }
            "turn_context" => {
                if let Some(payload) = value.get("payload") {
                    project_dir = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(project_dir);
                    events.push(provider_payload_event(
                        format!("codex:turn_context:{}", line_idx + 1),
                        SessionEventKind::Lifecycle,
                        EventRole::System,
                        timestamp,
                        "turn_context",
                        payload.clone(),
                        value.clone(),
                        None,
                    ));
                }
            }
            "event_msg" => {
                if let Some(payload) = value.get("payload") {
                    events.push(codex_event_msg_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                    ));
                }
            }
            "response_item" => {
                if let Some(payload) = value.get("payload") {
                    let msg_type = payload.get("type").and_then(|v| v.as_str());
                    if msg_type == Some("token_count") {
                        continue;
                    }
                    events.push(codex_response_item_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                        &mut report,
                    ));
                }
            }
            other => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Normalized,
                    code: "unknown_codex_line".to_string(),
                    message: format!("Preserved unknown Codex line type '{}'", other),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(value.clone()),
                });
                events.push(provider_payload_event(
                    format!("codex:unknown:{}", line_idx + 1),
                    SessionEventKind::Unknown,
                    EventRole::Unknown,
                    timestamp,
                    other,
                    value.get("payload").cloned().unwrap_or(Value::Null),
                    value,
                    None,
                ));
            }
        }
    }

    let canonical_id = session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: canonical_id.clone(),
                source_title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: canonical_id,
                    source_path: Some(path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: project_dir,
                created_at,
                last_active_at,
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn codex_response_item_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
    report: &mut MappingReport,
) -> SessionEvent {
    let role_str = payload.get("role").and_then(|v| v.as_str());
    let msg_type = payload.get("type").and_then(|v| v.as_str());
    let phase = payload
        .get("phase")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let event_id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("codex:response_item:{}", line_no));

    if msg_type == Some("function_call") {
        let name = payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let call_id = payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let input = payload.get("arguments").cloned();
        let role = match role_str {
            Some("assistant") => EventRole::Assistant,
            _ => EventRole::Unknown,
        };
        return SessionEvent {
            id: event_id,
            kind: SessionEventKind::ToolCall,
            role,
            timestamp,
            links: EventLinks::default(),
            blocks: vec![EventBlock::ToolCall {
                tool_call_id: call_id.to_string(),
                name: name.to_string(),
                input,
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: PROVIDER_ID.to_string(),
                    original_id: None,
                    original_role: role_str.map(str::to_string),
                    phase: phase.clone(),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: {
                    let mut ext = BTreeMap::new();
                    ext.insert("codex_payload".to_string(), payload.clone());
                    ext.insert("codex_raw_line".to_string(), raw_line);
                    ext
                },
            },
        };
    }

    if msg_type == Some("function_call_output") {
        let call_id = payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = payload.get("output").and_then(|v| v.as_str()).unwrap_or("");
        return SessionEvent {
            id: event_id,
            kind: SessionEventKind::ToolResult,
            role: EventRole::Tool,
            timestamp,
            links: EventLinks::default(),
            blocks: vec![EventBlock::ToolResult {
                tool_call_id: call_id.to_string(),
                content: content.to_string(),
                is_error: false,
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: PROVIDER_ID.to_string(),
                    original_id: None,
                    original_role: Some("tool".to_string()),
                    phase: phase.clone(),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: {
                    let mut ext = BTreeMap::new();
                    ext.insert("codex_payload".to_string(), payload.clone());
                    ext.insert("codex_raw_line".to_string(), raw_line);
                    ext
                },
            },
        };
    }

    if msg_type != Some("message") {
        return provider_payload_event(
            event_id,
            SessionEventKind::Unknown,
            EventRole::Unknown,
            timestamp,
            msg_type.unwrap_or("response_item"),
            payload.clone(),
            raw_line,
            phase,
        );
    }

    let mut blocks = Vec::new();
    if let Some(content_arr) = payload.get("content").and_then(|v| v.as_array()) {
        for (idx, block) in content_arr.iter().enumerate() {
            let Some(block_type) = block.get("type").and_then(|v| v.as_str()) else {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Normalized,
                    code: "codex_block_missing_type".to_string(),
                    message: "Codex content block without a type was preserved as unknown"
                        .to_string(),
                    path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                    raw: Some(block.clone()),
                });
                blocks.push(EventBlock::Unknown { raw: block.clone() });
                continue;
            };
            match block_type {
                "input_text" | "output_text" => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        blocks.push(EventBlock::Text {
                            text: text.to_string(),
                        });
                    }
                }
                "refusal" => {
                    let text = block
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("[refused]");
                    blocks.push(EventBlock::Text {
                        text: text.to_string(),
                    });
                }
                "input_image" => {
                    if let Some(image_block) = codex_image_block(block) {
                        blocks.push(image_block);
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Info,
                            disposition: MappingDisposition::Normalized,
                            code: "codex_input_image_preserved_raw".to_string(),
                            message: "Codex input_image block was preserved as provider payload"
                                .to_string(),
                            path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                            raw: Some(block.clone()),
                        });
                        blocks.push(EventBlock::ProviderPayload {
                            kind: "input_image".to_string(),
                            payload: block.clone(),
                        });
                    }
                }
                "reasoning" => {
                    // Skip reasoning blocks - they are provider-internal telemetry
                    continue;
                }
                other => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: MappingDisposition::Normalized,
                        code: "codex_unknown_block_preserved".to_string(),
                        message: format!("Preserved unknown Codex content block '{}'", other),
                        path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                        raw: Some(block.clone()),
                    });
                    blocks.push(EventBlock::Unknown { raw: block.clone() });
                }
            }
        }
    } else if let Some(text) = payload.get("content").and_then(|v| v.as_str()) {
        blocks.push(EventBlock::Text {
            text: text.to_string(),
        });
    } else {
        blocks.push(EventBlock::ProviderPayload {
            kind: "message_without_content".to_string(),
            payload: payload.clone(),
        });
    }

    if phase.as_deref() == Some("commentary") && blocks.len() == 1 {
        if let EventBlock::Text { text } = &blocks[0] {
            blocks[0] = EventBlock::Thinking {
                text: text.clone(),
                signature: None,
            };
        }
    }

    let role = match role_str {
        Some("user") => EventRole::User,
        Some("assistant") => EventRole::Assistant,
        Some("developer") => EventRole::Developer,
        Some("system") => EventRole::System,
        Some("tool") => EventRole::Tool,
        _ => EventRole::Unknown,
    };

    SessionEvent {
        id: event_id,
        kind: SessionEventKind::Message,
        role,
        timestamp,
        links: EventLinks::default(),
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: role_str.map(str::to_string),
                phase,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("codex_payload".to_string(), payload.clone());
                ext.insert("codex_raw_line".to_string(), raw_line);
                ext
            },
        },
    }
}

fn codex_event_msg_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
) -> SessionEvent {
    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("event_msg");
    let role = match event_type {
        "user_message" => EventRole::User,
        "agent_message" => EventRole::Assistant,
        _ => EventRole::System,
    };

    let mut blocks = Vec::new();
    let message_text = payload.get("message").and_then(|v| v.as_str());
    let last_agent_text = payload.get("last_agent_message").and_then(|v| v.as_str());

    if let Some(text) = message_text {
        blocks.push(EventBlock::Text {
            text: text.to_string(),
        });
    }
    if let Some(text) = last_agent_text {
        if message_text != Some(text) && !text.trim().is_empty() {
            blocks.push(EventBlock::Text {
                text: text.to_string(),
            });
        }
    }
    blocks.push(EventBlock::ProviderPayload {
        kind: event_type.to_string(),
        payload: payload.clone(),
    });

    let mut event = provider_payload_event(
        format!("codex:event_msg:{}:{}", event_type, line_no),
        SessionEventKind::Lifecycle,
        role,
        timestamp,
        event_type,
        payload.clone(),
        raw_line,
        payload
            .get("phase")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    );
    event.blocks = blocks;
    event
}

fn codex_image_block(block: &Value) -> Option<EventBlock> {
    let mime_type = block
        .get("mime_type")
        .or_else(|| block.get("mimeType"))
        .and_then(|v| v.as_str())
        .unwrap_or("image/*")
        .to_string();
    let image_url = block
        .get("image_url")
        .or_else(|| block.get("url"))
        .or_else(|| block.get("source"))
        .and_then(|v| v.as_str())?;
    if let Some((mime, data)) = parse_data_uri(image_url) {
        return Some(EventBlock::Image {
            mime_type: mime.to_string(),
            data: Some(data.to_string()),
            path: None,
        });
    }
    Some(EventBlock::Image {
        mime_type,
        data: None,
        path: Some(image_url.to_string()),
    })
}

fn parse_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

fn provider_payload_event(
    id: String,
    kind: SessionEventKind,
    role: EventRole,
    timestamp: chrono::DateTime<Utc>,
    payload_kind: &str,
    payload: Value,
    raw_line: Value,
    phase: Option<String>,
) -> SessionEvent {
    SessionEvent {
        id,
        kind,
        role,
        timestamp,
        links: EventLinks::default(),
        blocks: vec![EventBlock::ProviderPayload {
            kind: payload_kind.to_string(),
            payload: payload.clone(),
        }],
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: None,
                phase,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("codex_payload".to_string(), payload);
                ext.insert("codex_raw_line".to_string(), raw_line);
                ext
            },
        },
    }
}

fn build_cwd_lookup() -> Result<std::collections::HashMap<String, String>> {
    let sqlite_path = get_codex_dir().join("state_5.sqlite");
    if !sqlite_path.exists() {
        return Ok(std::collections::HashMap::new());
    }
    let conn = rusqlite::Connection::open(&sqlite_path)?;
    let mut stmt = conn.prepare("SELECT id, cwd FROM threads")?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let cwd: String = row.get(1)?;
        Ok((id, cwd))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        if let Ok((id, cwd)) = row {
            map.insert(id, cwd);
        }
    }
    Ok(map)
}

#[derive(Debug, Clone)]
struct CodexRolloutSummary {
    session_id: String,
    title: Option<String>,
    workspace_dir: Option<String>,
    model_provider: Option<String>,
    original_model_provider: Option<String>,
    updated_at: Option<String>,
}

fn read_codex_rollout_summary(path: &Path) -> Result<Option<CodexRolloutSummary>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex rollout file: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut session_id = None;
    let mut title = None;
    let mut workspace_dir = None;
    let mut model_provider = None;
    let mut updated_at = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if let Some(timestamp) = value.get("timestamp").and_then(|value| value.as_str()) {
            updated_at = Some(timestamp.to_string());
        }

        if value.get("type").and_then(|value| value.as_str()) != Some("session_meta") {
            continue;
        }

        let Some(payload) = value.get("payload") else {
            continue;
        };
        session_id = payload
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(session_id);
        title = payload
            .get("title")
            .or_else(|| payload.get("thread_name"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(title);
        workspace_dir = payload
            .get("cwd")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(workspace_dir);
        model_provider = payload
            .get("model_provider")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(model_provider);
    }

    let Some(session_id) = session_id else {
        return Ok(None);
    };

    Ok(Some(CodexRolloutSummary {
        session_id,
        title,
        workspace_dir,
        original_model_provider: model_provider.clone(),
        model_provider,
        updated_at,
    }))
}

fn load_session_index_ids(index_path: &Path) -> Result<HashSet<String>> {
    if !index_path.exists() {
        return Ok(HashSet::new());
    }

    let file = File::open(index_path).with_context(|| {
        format!(
            "Failed to open Codex session index: {}",
            index_path.display()
        )
    })?;
    let reader = BufReader::new(file);
    let mut ids = HashSet::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some(id) = value.get("id").and_then(|value| value.as_str()) {
            ids.insert(id.to_string());
        }
    }
    Ok(ids)
}

fn append_session_index_entry(
    index_path: &Path,
    session_id: &str,
    title: &str,
    updated_at: Option<&str>,
) -> Result<()> {
    if let Some(parent) = index_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut index_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(index_path)?;
    let updated_at = updated_at
        .map(str::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    writeln!(
        index_file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "id": session_id,
            "thread_name": title,
            "updated_at": updated_at,
        }))?
    )?;
    Ok(())
}

fn rewrite_rollout_model_provider(path: &Path, model_provider: &str) -> Result<()> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex rollout file: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut updated = false;

    for line in reader.lines() {
        let line = line?;
        if updated || line.trim().is_empty() {
            lines.push(line);
            continue;
        }
        let mut value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                lines.push(line);
                continue;
            }
        };
        if value.get("type").and_then(|value| value.as_str()) == Some("session_meta") {
            if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                payload.insert(
                    "model_provider".to_string(),
                    Value::String(model_provider.to_string()),
                );
                updated = true;
                lines.push(serde_json::to_string(&value)?);
                continue;
            }
        }
        lines.push(line);
    }

    if !updated {
        anyhow::bail!(
            "Codex rollout file is missing session_meta payload: {}",
            path.display()
        );
    }

    std::fs::write(path, lines.join("\n") + "\n")
        .with_context(|| format!("Failed to write Codex rollout file: {}", path.display()))?;
    Ok(())
}

fn extract_cwd_from_session_file(id: &str) -> Option<String> {
    let path = find_session_file(id)?;
    let file = File::open(&path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(5) {
        let line = line.ok()?;
        let value: Value = serde_json::from_str(&line).ok()?;
        if value.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
            return value
                .get("payload")
                .and_then(|p| p.get("cwd"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    None
}

fn get_codex_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let timestamp_str = now.format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("rollout-{}-{}.jsonl", timestamp_str, session_id);
    let sessions_dir = get_codex_dir()
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string());
    std::fs::create_dir_all(&sessions_dir)?;

    let file_path = sessions_dir.join(&filename);
    let rollout_path = file_path.to_string_lossy().to_string();
    let mut file = File::create(&file_path)?;
    let git_info = get_git_info(target_dir);
    let codex_version = get_codex_version();
    let codex_model_provider = get_codex_model_provider();
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = canonical_session_title(session);
    let first_user_message = first_user_message(session);
    let has_user_event = has_user_event(session);
    let base_instructions = session
        .events
        .iter()
        .find(|event| matches!(event.role, EventRole::System | EventRole::Developer))
        .map(canonical_event_text)
        .filter(|text| !text.trim().is_empty());

    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": now.to_rfc3339(),
            "type": "session_meta",
            "payload": {
                "id": session_id,
                "timestamp": now.to_rfc3339(),
                "cwd": target_dir_str,
                "originator": "memorph-cli",
                "cli_version": codex_version,
                "source": "cli",
                "model_provider": codex_model_provider,
                "title": title,
                "base_instructions": base_instructions.as_ref().map(|text| {
                    serde_json::json!({ "text": text })
                }).unwrap_or(Value::Null),
                "git": {
                    "commit_hash": git_info.as_ref().and_then(|git| git.commit_hash.clone()).unwrap_or_default(),
                    "branch": git_info.as_ref().and_then(|git| git.branch.clone()).unwrap_or_default(),
                }
            }
        }))?
    )?;

    let turn_id = Uuid::new_v4().to_string();
    let first_ts = session
        .events
        .first()
        .map(|event| event.timestamp)
        .unwrap_or(now);
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": first_ts.to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "task_started",
                "turn_id": turn_id,
                "started_at": first_ts.timestamp(),
                "collaboration_mode_kind": "default"
            }
        }))?
    )?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": first_ts.to_rfc3339(),
            "type": "turn_context",
            "payload": {
                "turn_id": turn_id,
                "cwd": target_dir_str,
                "current_date": first_ts.format("%Y-%m-%d").to_string(),
                "timezone": "Asia/Shanghai"
            }
        }))?
    )?;

    let mut wrote_user_event = false;
    let mut last_agent_message = String::new();
    for event in &session.events {
        if event.role == EventRole::System {
            continue;
        }
        let role = match event.role {
            EventRole::Assistant => "assistant",
            EventRole::Developer => "developer",
            EventRole::User | EventRole::Tool | EventRole::Unknown => "user",
            EventRole::System => continue,
        };
        let content = canonical_event_to_codex_content(event);
        if content.is_empty() {
            continue;
        }
        let mut payload = serde_json::json!({
            "type": "message",
            "role": role,
            "content": content,
        });
        if event.role == EventRole::Assistant {
            payload["phase"] = Value::String("final_answer".to_string());
            last_agent_message = canonical_event_text(event);
            writeln!(
                file,
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "timestamp": event.timestamp.to_rfc3339(),
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "message": last_agent_message,
                        "phase": "final_answer",
                        "memory_citation": null
                    }
                }))?
            )?;
        }
        writeln!(
            file,
            "{}",
            serde_json::to_string(&serde_json::json!({
                "timestamp": event.timestamp.to_rfc3339(),
                "type": "response_item",
                "payload": payload,
            }))?
        )?;
        if event.role == EventRole::User && !wrote_user_event {
            let user_text = canonical_event_text(event);
            writeln!(
                file,
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "timestamp": event.timestamp.to_rfc3339(),
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": user_text,
                        "images": [],
                        "local_images": [],
                        "text_elements": []
                    }
                }))?
            )?;
            wrote_user_event = true;
        }
    }

    let last_ts = session
        .events
        .last()
        .map(|event| event.timestamp)
        .unwrap_or(now);
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": last_ts.to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": turn_id,
                "last_agent_message": last_agent_message,
                "completed_at": last_ts.timestamp(),
                "duration_ms": 1000
            }
        }))?
    )?;

    let index_path = get_codex_dir().join("session_index.jsonl");
    let mut index_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&index_path)?;
    writeln!(
        index_file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "id": session_id,
            "thread_name": title,
            "updated_at": now.to_rfc3339(),
        }))?
    )?;
    update_codex_sqlite(
        &session_id,
        &rollout_path,
        target_dir,
        &title,
        first_user_message.as_deref(),
        has_user_event,
        &now,
    )?;
    update_codex_global_state_workspace(target_dir)?;
    Ok(session_id)
}

fn canonical_event_to_codex_content(event: &SessionEvent) -> Vec<Value> {
    event
        .blocks
        .iter()
        .filter_map(|block| match block {
            EventBlock::Text { text } => Some(serde_json::json!({
                "type": if event.role == EventRole::Assistant { "output_text" } else { "input_text" },
                "text": text,
            })),
            EventBlock::Thinking { text, .. } => Some(serde_json::json!({
                "type": "output_text",
                "text": format!("[Thinking]\n{}", text),
            })),
            EventBlock::Image { data: Some(data), .. } if event.role != EventRole::Assistant => {
                Some(serde_json::json!({
                    "type": "input_image",
                    "image_url": data,
                }))
            }
            EventBlock::ProviderPayload { .. } => None,
            _ => {
                let text = canonical_block_text(block);
                (!text.trim().is_empty()).then(|| serde_json::json!({
                    "type": if event.role == EventRole::Assistant { "output_text" } else { "input_text" },
                    "text": text,
                }))
            }
        })
        .collect()
}

fn find_session_file(id: &str) -> Option<PathBuf> {
    // Search both active sessions and archived sessions
    let dirs = [
        get_codex_dir().join("sessions"),
        get_codex_dir().join("archived_sessions"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(id))
                .unwrap_or(false)
            {
                return Some(path.to_path_buf());
            }
        }
    }

    None
}

#[derive(Default)]
struct GitInfo {
    commit_hash: Option<String>,
    branch: Option<String>,
}

fn get_git_info(dir: &Path) -> Option<GitInfo> {
    let mut info = GitInfo::default();

    let branch_output = std::process::Command::new("git")
        .args([
            "-C",
            &dir.to_string_lossy(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
        .ok()?;
    if branch_output.status.success() {
        info.branch = Some(
            String::from_utf8_lossy(&branch_output.stdout)
                .trim()
                .to_string(),
        );
    }

    let hash_output = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if hash_output.status.success() {
        info.commit_hash = Some(
            String::from_utf8_lossy(&hash_output.stdout)
                .trim()
                .to_string(),
        );
    }

    Some(info)
}

fn first_user_message(session: &CanonicalSession) -> Option<String> {
    session
        .events
        .iter()
        .filter(|event| event.role == EventRole::User)
        .find_map(|event| {
            let text = canonical_event_text(event);
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
}

fn has_user_event(session: &CanonicalSession) -> bool {
    session
        .events
        .iter()
        .any(|event| event.role == EventRole::User)
}

fn update_codex_sqlite(
    session_id: &str,
    rollout_path: &str,
    cwd: &Path,
    title: &str,
    first_user_message: Option<&str>,
    has_user_event: bool,
    now: &chrono::DateTime<Utc>,
) -> Result<()> {
    let sqlite_path = get_codex_dir().join("state_5.sqlite");
    if !sqlite_path.exists() {
        // SQLite not present, skip
        return Ok(());
    }

    let conn = rusqlite::Connection::open(&sqlite_path)
        .with_context(|| format!("Failed to open Codex SQLite: {}", sqlite_path.display()))?;

    let created_at = now.timestamp();
    let created_at_ms = now.timestamp_millis();
    let cwd_str = cwd.to_string_lossy().to_string();
    let codex_version = get_codex_version();
    let codex_model_provider = get_codex_model_provider();
    let (codex_model, codex_reasoning) = get_codex_model_config();
    let sandbox_json = format!(
        "{{\"type\":\"workspace-write\",\"writable_roots\":[],\"network_access\":false,\"exclude_tmpdir_env_var\":false,\"exclude_slash_tmp\":false}}"
    );

    conn.execute(
        "INSERT INTO threads (
            id, rollout_path, created_at, updated_at, source, model_provider,
            cwd, title, sandbox_policy, approval_mode, tokens_used, has_user_event,
            archived, cli_version, first_user_message, memory_mode, git_branch,
            model, reasoning_effort, created_at_ms, updated_at_ms
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
        ) ON CONFLICT(id) DO UPDATE SET
            updated_at = excluded.updated_at,
            updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            session_id,
            rollout_path,
            created_at,
            created_at,
            "cli",
            codex_model_provider,
            cwd_str,
            title,
            sandbox_json,
            "on-request",
            0,
            if has_user_event { 1 } else { 0 },
            0,
            codex_version,
            first_user_message.unwrap_or(title),
            "enabled",
            get_git_branch(cwd).unwrap_or_else(|| "main".to_string()),
            codex_model,
            codex_reasoning,
            created_at_ms,
            created_at_ms,
        ],
    )
    .with_context(|| "Failed to insert thread into Codex SQLite")?;

    Ok(())
}

fn update_codex_global_state_workspace(workspace_root: &Path) -> Result<()> {
    let global_state_path = get_codex_dir().join(".codex-global-state.json");
    if !global_state_path.exists() {
        return Ok(());
    }
    update_codex_global_state_file(&global_state_path, workspace_root)
}

fn update_codex_global_state_file_if_exists(codex_dir: &Path, workspace_root: &Path) -> Result<()> {
    let global_state_path = codex_dir.join(".codex-global-state.json");
    if !global_state_path.exists() {
        return Ok(());
    }
    update_codex_global_state_file(&global_state_path, workspace_root)
}

fn update_codex_global_state_file(global_state_path: &Path, workspace_root: &Path) -> Result<()> {
    let content = std::fs::read_to_string(global_state_path).with_context(|| {
        format!(
            "Failed to read Codex global state: {}",
            global_state_path.display()
        )
    })?;
    let mut value: Value = serde_json::from_str(&content).with_context(|| {
        format!(
            "Failed to parse Codex global state: {}",
            global_state_path.display()
        )
    })?;
    let workspace = workspace_root.to_string_lossy().to_string();
    ensure_unique_string_array_entry(&mut value, "electron-saved-workspace-roots", &workspace);
    ensure_unique_string_array_entry(&mut value, "project-order", &workspace);
    let serialized = serde_json::to_string(&value)?;
    std::fs::write(global_state_path, serialized).with_context(|| {
        format!(
            "Failed to write Codex global state: {}",
            global_state_path.display()
        )
    })?;
    Ok(())
}

fn ensure_unique_string_array_entry(value: &mut Value, key: &str, entry: &str) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    let field = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(items) = field.as_array_mut() else {
        return;
    };
    if items.iter().any(|item| item.as_str() == Some(entry)) {
        return;
    }
    items.push(Value::String(entry.to_string()));
}

fn get_codex_version() -> String {
    let version_path = get_codex_dir().join("version.json");
    if let Ok(content) = std::fs::read_to_string(&version_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(ver) = v.get("latest_version").and_then(|v| v.as_str()) {
                return ver.to_string();
            }
        }
    }
    "0.124.0".to_string()
}

fn update_rollout_session_meta_title(path: &Path, new_title: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut new_lines = Vec::new();
    let mut updated = false;

    for line in content.lines() {
        if !updated && !line.trim().is_empty() {
            if let Ok(mut value) = serde_json::from_str::<Value>(line) {
                if value.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
                    if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                        payload.insert("title".to_string(), Value::String(new_title.to_string()));
                        new_lines.push(serde_json::to_string(&value)?);
                        updated = true;
                        continue;
                    }
                }
            }
        }
        new_lines.push(line.to_string());
    }

    if updated {
        std::fs::write(path, new_lines.join("\n") + "\n")?;
    }

    Ok(())
}

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .is_ok())
}

fn has_columns(conn: &Connection, table: &str, columns: &[&str]) -> Result<bool> {
    let existing: HashSet<String> = table_columns(conn, table)?.into_iter().collect();
    Ok(columns.iter().all(|column| existing.contains(*column)))
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "\"\"")
    ))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn delete_related_rows(
    conn: &Connection,
    table: &str,
    where_clause: &str,
    session_id: &str,
) -> Result<()> {
    if has_table(conn, table)? {
        conn.execute(
            &format!("DELETE FROM \"{table}\" WHERE {where_clause}"),
            [session_id],
        )?;
    }
    Ok(())
}

fn read_codex_model_provider(codex_dir: &Path) -> String {
    let config_path = codex_dir.join("config.toml");
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("model_provider") && !trimmed.starts_with("model_providers") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    return val.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    "openai".to_string()
}

fn get_codex_model_provider() -> String {
    read_codex_model_provider(&get_codex_dir())
}

fn get_codex_model_config() -> (String, String) {
    let config_path = get_codex_dir().join("config.toml");
    let mut model = String::new();
    let mut reasoning = String::new();
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if model.is_empty() && trimmed.starts_with("model ") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    model = val.trim().trim_matches('"').to_string();
                }
            }
            if reasoning.is_empty() && trimmed.starts_with("model_reasoning_effort") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    reasoning = val.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    if model.is_empty() {
        model = "gpt-5.3-codex".to_string();
    }
    if reasoning.is_empty() {
        reasoning = "xhigh".to_string();
    }
    (model, reasoning)
}

fn get_git_branch(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &dir.to_string_lossy(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn import_canonical_session_preserves_codex_runtime_and_message_events() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-1",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project",
                    "base_instructions": { "text": "Be careful." },
                    "model": "gpt-5.3-codex"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "turn_context",
                "payload": {
                    "turn_id": "turn-1",
                    "cwd": "/tmp/project",
                    "current_date": "2026-05-21",
                    "timezone": "Asia/Shanghai"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_started",
                    "turn_id": "turn-1",
                    "started_at": 1747821602
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:03Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "developer",
                    "content": [
                        { "type": "input_text", "text": "# AGENTS.md instructions" }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:04Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": [
                        { "type": "output_text", "text": "Thinking out loud" }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:05Z",
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "shell",
                    "call_id": "call_1",
                    "arguments": "{\"cmd\":\"echo hello\"}"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:05Z",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": "hello"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:06Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "Done."
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let events = &imported.session.events;

        assert_eq!(imported.session.identity.canonical_id, "session-1");
        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some("/tmp/project")
        );
        assert!(events.iter().any(|event| {
            event.role == EventRole::System
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "Be careful."
                )
        }));
        assert!(events.iter().any(|event| {
            event.role == EventRole::Developer
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "# AGENTS.md instructions"
                )
        }));
        assert!(events.iter().any(|event| {
            event.role == EventRole::Assistant
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Thinking { text, .. }) if text == "Thinking out loud"
                )
        }));
        assert!(events.iter().any(|event| {
            event.id == "codex:response_item:6"
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ToolCall { name, tool_call_id, .. })
                        if name == "shell" && tool_call_id == "call_1"
                )
        }));
        assert!(events.iter().any(|event| {
            event.id == "codex:response_item:7"
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ToolResult { content, tool_call_id, .. })
                        if content == "hello" && tool_call_id == "call_1"
                )
        }));
        assert!(events.iter().any(|event| {
            event.id == "codex:event_msg:task_complete:8"
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "Done."
                )
        }));
    }

    #[test]
    fn import_canonical_session_decodes_input_image_data_uri() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-2",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_image",
                            "mime_type": "image/png",
                            "image_url": "data:image/png;base64,QUJD"
                        }
                    ]
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let image_block = imported
            .session
            .events
            .iter()
            .flat_map(|event| event.blocks.iter())
            .find_map(|block| match block {
                EventBlock::Image {
                    mime_type,
                    data,
                    path,
                } => Some((mime_type, data, path)),
                _ => None,
            })
            .expect("expected image block");

        assert_eq!(image_block.0, "image/png");
        assert_eq!(image_block.1.as_deref(), Some("QUJD"));
        assert_eq!(image_block.2, &None);
    }

    #[test]
    fn compressed_segment_exports_as_portable_codex_text() {
        let event = SessionEvent {
            id: "compressed-source".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Compressed {
                source_provider_id: "opencode".to_string(),
                summary: "compressed summary".to_string(),
                source_event_ids: vec![
                    "old-event-1".to_string(),
                    "old-event-2".to_string(),
                    "old-event-3".to_string(),
                ],
                source_event_count: None,
                archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "memorph".to_string(),
                    original_id: None,
                    original_role: Some("assistant".to_string()),
                    phase: Some("compression".to_string()),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        let content = canonical_event_to_codex_content(&event);

        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0].get("type").and_then(Value::as_str),
            Some("output_text")
        );
        let text = content[0]
            .get("text")
            .and_then(Value::as_str)
            .expect("portable compressed text");
        assert!(text.contains("[Compressed session segment from opencode]"));
        assert!(text.contains("compressed summary"));
        assert!(text.contains("Source event count: 3"));
        assert!(text.contains("Archive: memorph-archive://s1/archive.json.gz"));
        assert!(!text.contains("old-event-1"));
        assert!(!text.contains("old-event-2"));
        assert!(!text.contains("old-event-3"));
    }

    #[test]
    fn first_user_message_skips_empty_user_events_but_has_user_event_stays_true() {
        let session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "session-3".to_string(),
                source_title: None,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: None,
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: "session-3".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: None,
                created_at: None,
                last_active_at: None,
                tags: Vec::new(),
            },
            events: vec![
                SessionEvent {
                    id: "user-empty".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: Utc::now(),
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "   ".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: PROVIDER_ID.to_string(),
                            original_id: None,
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
                SessionEvent {
                    id: "user-real".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: Utc::now(),
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "real prompt".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: PROVIDER_ID.to_string(),
                            original_id: None,
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };

        assert!(has_user_event(&session));
        assert_eq!(first_user_message(&session).as_deref(), Some("real prompt"));
    }

    #[test]
    fn update_codex_global_state_file_remembers_workspace_without_switching_active_root() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".codex-global-state.json");
        let workspace_a = "/tmp/a";
        let workspace_b = "/tmp/b";
        std::fs::write(
            &path,
            serde_json::to_string(&json!({
                "electron-saved-workspace-roots": [workspace_a],
                "active-workspace-roots": [workspace_a],
                "project-order": [workspace_a],
            }))
            .unwrap(),
        )
        .unwrap();

        update_codex_global_state_file(&path, Path::new(workspace_b)).unwrap();

        let updated: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            updated["electron-saved-workspace-roots"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![workspace_a, workspace_b]
        );
        assert_eq!(
            updated["project-order"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![workspace_a, workspace_b]
        );
        assert_eq!(
            updated["active-workspace-roots"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![workspace_a]
        );
    }

    #[test]
    fn repair_workspace_sessions_updates_provider_and_reindexes_matching_workspace() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        let sessions_dir = codex_dir.join("sessions/2026/05/27");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"custom-provider\"\n",
        )
        .unwrap();
        std::fs::write(
            codex_dir.join(".codex-global-state.json"),
            serde_json::to_string(&json!({
                "electron-saved-workspace-roots": [],
                "project-order": [],
                "active-workspace-roots": [],
            }))
            .unwrap(),
        )
        .unwrap();

        let session_path = sessions_dir.join("rollout-2026-05-27T12-00-00-session-1.jsonl");
        std::fs::write(
            &session_path,
            [
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-27T12:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": "session-1",
                        "timestamp": "2026-05-27T12:00:00Z",
                        "cwd": workspace.to_string_lossy(),
                        "model_provider": "openai",
                        "title": "Repair me"
                    }
                }))
                .unwrap(),
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-27T12:05:00Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": "hello"
                    }
                }))
                .unwrap(),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        let report =
            repair_workspace_sessions_in_codex_home(&codex_dir, Some(workspace.to_str().unwrap()))
                .unwrap();

        assert_eq!(report.current_model_provider, "custom-provider");
        assert_eq!(report.workspace_session_count, 1);
        assert_eq!(report.hidden_session_count, 1);
        assert_eq!(report.repaired_session_count, 1);
        assert_eq!(report.reindexed_session_count, 1);
        assert_eq!(report.touched_sessions.len(), 1);
        assert_eq!(
            report.touched_sessions[0]
                .previous_model_provider
                .as_deref(),
            Some("openai")
        );

        let updated_rollout = std::fs::read_to_string(&session_path).unwrap();
        assert!(updated_rollout.contains("\"model_provider\":\"custom-provider\""));

        let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
        assert!(index.contains("\"id\":\"session-1\""));

        let global_state: Value = serde_json::from_str(
            &std::fs::read_to_string(codex_dir.join(".codex-global-state.json")).unwrap(),
        )
        .unwrap();
        let saved = global_state["electron-saved-workspace-roots"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        let canonical_workspace = workspace.canonicalize().unwrap();
        assert_eq!(saved, vec![canonical_workspace.to_string_lossy().as_ref()]);
    }

    #[test]
    fn import_canonical_session_drops_token_count() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-tc",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        { "type": "output_text", "text": "Hello" }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "response_item",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "input_tokens": 100,
                        "output_tokens": 50
                    }
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        // session_meta + message = 2 events; token_count is dropped
        assert_eq!(imported.session.events.len(), 2);
        assert!(!imported.session.events.iter().any(|event| {
            event.blocks.iter().any(|block| matches!(block, EventBlock::ProviderPayload { kind, .. } if kind == "token_count"))
        }));
    }

    #[test]
    fn import_canonical_session_dedupes_last_agent_message() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-dedup",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "event_msg",
                "payload": {
                    "type": "agent_message",
                    "message": "Same text",
                    "last_agent_message": "Same text",
                    "phase": "final_answer"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "Different text"
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let events: Vec<_> = imported.session.events.iter().collect();

        let agent_msg = events
            .iter()
            .find(|e| e.id == "codex:event_msg:agent_message:2")
            .unwrap();
        let text_blocks: Vec<_> = agent_msg
            .blocks
            .iter()
            .filter_map(|b| match b {
                EventBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_blocks, vec!["Same text"]);

        let complete_msg = events
            .iter()
            .find(|e| e.id == "codex:event_msg:task_complete:3")
            .unwrap();
        let text_blocks: Vec<_> = complete_msg
            .blocks
            .iter()
            .filter_map(|b| match b {
                EventBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_blocks, vec!["Different text"]);
    }

    #[test]
    fn provider_payload_block_is_skipped_in_codex_export() {
        let event = SessionEvent {
            id: "test".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![
                EventBlock::Text {
                    text: "Hello".to_string(),
                },
                EventBlock::ProviderPayload {
                    kind: "task_complete".to_string(),
                    payload: serde_json::json!({"type": "task_complete"}),
                },
            ],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "codex".to_string(),
                    original_id: None,
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: BTreeMap::new(),
            },
        };

        let content = canonical_event_to_codex_content(&event);
        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0].get("text").and_then(Value::as_str),
            Some("Hello")
        );
    }
}
