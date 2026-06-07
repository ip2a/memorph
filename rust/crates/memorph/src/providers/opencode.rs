use crate::canonical::{
    ArtifactKind, CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata,
    EventRole, EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionArtifact,
    SessionContext, SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance, UsageStats,
};
use crate::core::compression::{
    self, CompressedSegment, CompressionProjection, NativeTargetProjection,
};
use crate::provider::{
    canonical_block_text, canonical_export_result, canonical_session_title,
    compression_retrieval_hint, Provider, ProviderCapabilities, ProviderSessionSummary,
};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct OpenCodeProvider;

const PROVIDER_ID: &str = "opencode";
const OPENCODE_VERSION: &str = "1.3.17";

impl Provider for OpenCodeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "OpenCode"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_session_management()
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let mut sessions = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // 1. Scan SQLite DB (primary source for recent sessions)
        if let Ok(db_sessions) = scan_sessions_from_db() {
            for s in db_sessions {
                seen.insert(s.session_id.clone());
                sessions.push(s);
            }
        }

        // 2. Scan filesystem (fallback for older sessions)
        let storage_dir = get_opencode_dir().join("storage").join("session");
        if storage_dir.exists() {
            for entry in WalkDir::new(&storage_dir)
                .max_depth(3)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if let Some(meta) = parse_session_file(path) {
                    if !seen.contains(&meta.session_id) {
                        sessions.push(meta);
                    }
                }
            }
        }

        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        import_canonical_session(source_path)
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
        let db_path = get_db_path();
        let storage_dir = get_opencode_dir().join("storage");

        // Delete from DB
        if db_path.exists() {
            let conn = Connection::open(&db_path)?;
            conn.execute("DELETE FROM part WHERE session_id = ?1", [session_id])?;
            conn.execute("DELETE FROM message WHERE session_id = ?1", [session_id])?;
            conn.execute("DELETE FROM session WHERE id = ?1", [session_id])?;
        }

        // Delete from filesystem
        // Remove session file
        for entry in WalkDir::new(storage_dir.join("session"))
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                let _ = std::fs::remove_file(path);
                // Try to remove empty parent dirs
                if let Some(parent) = path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
                break;
            }
        }

        // Remove message directory
        let msg_dir = storage_dir.join("message").join(session_id);
        if msg_dir.exists() {
            let _ = std::fs::remove_dir_all(&msg_dir);
        }

        // Remove part directories for this session's messages
        let parts_dir = storage_dir.join("part");
        if parts_dir.exists() {
            for entry in std::fs::read_dir(&parts_dir)? {
                let entry = entry?;
                let msg_dir = entry.path();
                if !msg_dir.is_dir() {
                    continue;
                }
                // Check if this message belongs to our session by looking at the first part
                let mut belongs = false;
                for part_entry in std::fs::read_dir(&msg_dir)? {
                    let part_entry = part_entry?;
                    let part_path = part_entry.path();
                    if part_path.extension().and_then(|e| e.to_str()) == Some("json") {
                        if let Ok(content) = std::fs::read_to_string(&part_path) {
                            if let Ok(v) = serde_json::from_str::<Value>(&content) {
                                if v.get("sessionID").and_then(|v| v.as_str()) == Some(session_id) {
                                    belongs = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                if belongs {
                    let _ = std::fs::remove_dir_all(&msg_dir);
                }
            }
        }

        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let db_path = get_db_path();
        let storage_dir = get_opencode_dir().join("storage");
        let now = Utc::now().timestamp_millis();

        // Update DB
        if db_path.exists() {
            let conn = Connection::open(&db_path)?;
            conn.execute(
                "UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3",
                [new_title, &now.to_string(), session_id],
            )?;
        }

        // Update filesystem
        for entry in WalkDir::new(storage_dir.join("session"))
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                if let Ok(content) = std::fs::read_to_string(path) {
                    if let Ok(mut v) = serde_json::from_str::<Value>(&content) {
                        if let Value::Object(ref mut map) = v {
                            map.insert("title".to_string(), Value::String(new_title.to_string()));
                            if let Some(Value::Object(ref mut time)) = map.get_mut("time") {
                                time.insert("updated".to_string(), Value::Number(now.into()));
                            }
                            let _ = std::fs::write(path, serde_json::to_string_pretty(&v)?);
                        }
                    }
                }
                break;
            }
        }

        Ok(())
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("opencode --session {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        if let Some(total) = opencode_session_files_size(session_id)? {
            return Ok(total);
        }

        if let Ok(size) = opencode_session_db_size(session_id) {
            return Ok(size);
        }

        Ok(0)
    }

    fn session_sizes(&self, session_ids: &[&str]) -> HashMap<String, u64> {
        let mut sizes = HashMap::new();
        let mut missing_db = Vec::new();
        for session_id in session_ids {
            match opencode_session_files_size(session_id) {
                Ok(Some(size)) if size > 0 => {
                    sizes.insert((*session_id).to_string(), size);
                }
                _ => missing_db.push(*session_id),
            }
        }

        if missing_db.is_empty() {
            return sizes;
        }

        let db_path = get_db_path();
        let Ok(conn) = Connection::open(&db_path) else {
            return sizes;
        };
        for session_id in missing_db {
            if let Ok(size) = opencode_session_db_size_with_conn(&conn, session_id) {
                if size > 0 {
                    sizes.insert(session_id.to_string(), size);
                }
            }
        }
        sizes
    }
}

fn import_canonical_session(session_id: &str) -> Result<ImportedSession> {
    let (session_json, messages, parts) = if let Ok(data) = load_session_from_db(session_id) {
        data
    } else {
        load_session_from_filesystem(session_id)?
    };

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut artifacts = Vec::new();

    let mut msg_list: Vec<(i64, Value, Vec<Value>)> = messages
        .into_iter()
        .map(|(created, msg_json)| {
            let msg_id = msg_json
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let msg_parts: Vec<Value> = parts.get(&msg_id).cloned().unwrap_or_default();
            (created, msg_json, msg_parts)
        })
        .collect();
    msg_list.sort_by_key(|(created, _, _)| *created);

    for (_, msg_json, msg_parts) in msg_list {
        let role_str = msg_json
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user");
        let role = match role_str {
            "user" => EventRole::User,
            "assistant" => EventRole::Assistant,
            other => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Normalized,
                    code: "unknown_role_normalized".to_string(),
                    message: format!("Normalized unknown OpenCode role '{}'", other),
                    path: None,
                    raw: Some(msg_json.clone()),
                });
                EventRole::Unknown
            }
        };

        let msg_id = msg_json
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let parent_id = msg_json
            .get("parentID")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let created = msg_json
            .get("time")
            .and_then(|v| v.get("created"))
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| Utc::now().timestamp_millis());
        let timestamp = chrono::DateTime::from_timestamp_millis(created).unwrap_or_else(Utc::now);

        let blocks = canonical_blocks_from_parts(&msg_id, &msg_parts, &mut report, &mut artifacts);
        if blocks.is_empty() {
            continue;
        }

        let model = msg_json
            .get("modelID")
            .or_else(|| msg_json.get("model").and_then(|m| m.get("modelID")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let provider = msg_json
            .get("providerID")
            .or_else(|| msg_json.get("model").and_then(|m| m.get("providerID")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| PROVIDER_ID.to_string());

        let usage = msg_json.get("tokens").map(|t| UsageStats {
            input_tokens: t.get("input").and_then(|v| v.as_u64()),
            output_tokens: t.get("output").and_then(|v| v.as_u64()),
            total_tokens: t.get("total").and_then(|v| v.as_u64()),
        });

        let finish = msg_json
            .get("finish")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let cost = msg_json.get("cost").and_then(|v| v.as_f64());
        let agent = msg_json
            .get("agent")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let mode = msg_json
            .get("mode")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut provider_ext = BTreeMap::new();
        provider_ext.insert("opencode_message".to_string(), msg_json.clone());
        if let Some(finish) = finish {
            provider_ext.insert("finish".to_string(), Value::String(finish));
        }
        if let Some(cost) = cost {
            provider_ext.insert("cost".to_string(), Value::from(cost));
        }
        if let Some(agent) = agent {
            provider_ext.insert("agent".to_string(), Value::String(agent));
        }
        if let Some(mode) = mode {
            provider_ext.insert("mode".to_string(), Value::String(mode));
        }

        let kind = derive_event_kind(&blocks);
        events.push(SessionEvent {
            id: msg_id.clone(),
            kind,
            role,
            timestamp,
            links: EventLinks {
                parent_event_id: parent_id.clone(),
                provider_parent_id: parent_id,
                turn_index: None,
                related_event_ids: Vec::new(),
            },
            blocks,
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: provider,
                    original_id: Some(msg_id),
                    original_role: Some(role_str.to_string()),
                    phase: None,
                },
                model,
                usage,
                fidelity: MappingDisposition::Preserved,
                provider_ext,
            },
        });
    }

    let session_id_val = session_json
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or(session_id)
        .to_string();
    let title = session_json
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let project_dir = session_json
        .get("directory")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let created = session_json
        .get("time")
        .and_then(|v| v.get("created"))
        .and_then(|v| v.as_i64())
        .and_then(chrono::DateTime::from_timestamp_millis);
    let updated = session_json
        .get("time")
        .and_then(|v| v.get("updated"))
        .and_then(|v| v.as_i64())
        .and_then(chrono::DateTime::from_timestamp_millis);

    let mut extensions = BTreeMap::new();
    extensions.insert("opencode_session".to_string(), session_json.clone());

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: session_id_val.clone(),
                source_title: title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: session_id_val.clone(),
                    source_path: Some(session_id.to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: project_dir,
                created_at: created,
                last_active_at: updated,
                tags: Vec::new(),
            },
            events,
            artifacts,
            extensions,
        },
        report,
    })
}

fn canonical_blocks_from_parts(
    msg_id: &str,
    msg_parts: &[Value],
    report: &mut MappingReport,
    artifacts: &mut Vec<SessionArtifact>,
) -> Vec<EventBlock> {
    let mut blocks = Vec::new();

    for (idx, part) in msg_parts.iter().enumerate() {
        let part_type = part.get("type").and_then(|v| v.as_str());
        match part_type {
            Some("text") => {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    blocks.push(EventBlock::Text {
                        text: text.to_string(),
                    });
                }
            }
            Some("reasoning") => {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    blocks.push(EventBlock::Thinking {
                        text: text.to_string(),
                        signature: None,
                    });
                }
            }
            Some("tool") => {
                let call_id = part
                    .get("callID")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_name = part
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let state = part.get("state").cloned().unwrap_or_default();
                let input = state.get("input").cloned();
                let output = state
                    .get("output")
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_default();
                let status = state
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("completed");

                if tool_name != "unknown" || input.is_some() {
                    blocks.push(EventBlock::ToolCall {
                        tool_call_id: call_id.clone(),
                        name: tool_name,
                        input,
                    });
                }
                blocks.push(EventBlock::ToolResult {
                    tool_call_id: call_id,
                    content: output,
                    is_error: status == "error",
                });
            }
            Some("file") => {
                let mime = part
                    .get("mime")
                    .and_then(|v| v.as_str())
                    .unwrap_or("application/octet-stream");
                let filename = part
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("file");
                let url = part.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if mime.starts_with("image/") && url.starts_with("data:") {
                    if let Some((mime_type, data)) = parse_data_uri(url) {
                        blocks.push(EventBlock::Image {
                            mime_type: mime_type.to_string(),
                            data: Some(data.to_string()),
                            path: Some(filename.to_string()),
                        });
                        artifacts.push(SessionArtifact {
                            id: format!("{}:image:{}", msg_id, idx),
                            kind: ArtifactKind::Image,
                            path: Some(filename.to_string()),
                            mime_type: Some(mime_type.to_string()),
                            content: None,
                            metadata: BTreeMap::new(),
                        });
                    }
                } else if !url.is_empty() {
                    blocks.push(EventBlock::File {
                        path: filename.to_string(),
                        content: Some(url.to_string()),
                        mime_type: Some(mime.to_string()),
                    });
                    artifacts.push(SessionArtifact {
                        id: format!("{}:file:{}", msg_id, idx),
                        kind: ArtifactKind::File,
                        path: Some(filename.to_string()),
                        mime_type: Some(mime.to_string()),
                        content: Some(url.to_string()),
                        metadata: BTreeMap::new(),
                    });
                }
            }
            Some("patch") => {
                let files = part
                    .get("files")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let hash = part
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let diff_text = part
                    .get("text")
                    .or_else(|| part.get("diff"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                blocks.push(EventBlock::Patch {
                    summary: None,
                    diff_text: diff_text.clone(),
                    files: files.clone(),
                    hash: hash.clone(),
                });
                artifacts.push(SessionArtifact {
                    id: format!("{}:patch:{}", msg_id, idx),
                    kind: ArtifactKind::Patch,
                    path: None,
                    mime_type: None,
                    content: diff_text,
                    metadata: {
                        let mut metadata = BTreeMap::new();
                        if let Some(hash) = hash {
                            metadata.insert("hash".to_string(), Value::String(hash));
                        }
                        if !files.is_empty() {
                            metadata.insert(
                                "files".to_string(),
                                Value::Array(files.into_iter().map(Value::String).collect()),
                            );
                        }
                        metadata
                    },
                });
            }
            Some("step-start") | Some("step-finish") | Some("compaction") => {
                blocks.push(EventBlock::ProviderPayload {
                    kind: part_type.unwrap_or("unknown").to_string(),
                    payload: part.clone(),
                });
            }
            Some(other) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Normalized,
                    code: "unknown_part_preserved".to_string(),
                    message: format!("Preserved unknown OpenCode part '{}'", other),
                    path: Some(format!("{}:part:{}", msg_id, idx)),
                    raw: Some(part.clone()),
                });
                blocks.push(EventBlock::Unknown { raw: part.clone() });
            }
            None => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Normalized,
                    code: "missing_part_type".to_string(),
                    message: "OpenCode part without a type was preserved as unknown payload"
                        .to_string(),
                    path: Some(format!("{}:part:{}", msg_id, idx)),
                    raw: Some(part.clone()),
                });
                blocks.push(EventBlock::Unknown { raw: part.clone() });
            }
        }
    }

    blocks
}

fn derive_event_kind(blocks: &[EventBlock]) -> SessionEventKind {
    if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::Patch { .. }))
    {
        SessionEventKind::Patch
    } else if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolResult { .. }))
    {
        SessionEventKind::ToolResult
    } else if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolCall { .. }))
    {
        SessionEventKind::ToolCall
    } else if blocks.iter().any(|block| {
        matches!(
            block,
            EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. }
        )
    }) {
        SessionEventKind::Unknown
    } else {
        SessionEventKind::Message
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_opencode_dir() -> PathBuf {
    // OpenCode uses ~/.local/share/opencode even on macOS
    dirs::home_dir()
        .map(|h| h.join(".local/share/opencode"))
        .unwrap_or_else(|| PathBuf::from(".local/share/opencode"))
}

fn generate_opencode_id(prefix: &str) -> String {
    let uuid = Uuid::new_v4().to_string().replace("-", "");
    format!("{}_{}", prefix, &uuid[..24.min(uuid.len())])
}

fn generate_slug() -> String {
    let adjectives = [
        "bright", "calm", "swift", "keen", "bold", "warm", "cool", "sharp", "clear", "steady",
    ];
    let nouns = [
        "river", "forest", "mountain", "ocean", "sky", "star", "path", "garden", "valley",
        "horizon",
    ];
    let idx1 = (Uuid::new_v4().as_u128() % adjectives.len() as u128) as usize;
    let idx2 = (Uuid::new_v4().as_u128() % nouns.len() as u128) as usize;
    format!("{}-{}", adjectives[idx1], nouns[idx2])
}

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    let now = Utc::now().timestamp_millis();
    let session_id = generate_opencode_id("ses");
    let project_id = find_or_create_project(target_dir)?;
    let slug = generate_slug();
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = canonical_session_title(session);

    let session_json = serde_json::json!({
        "id": &session_id,
        "slug": &slug,
        "version": OPENCODE_VERSION,
        "projectID": &project_id,
        "directory": &target_dir_str,
        "title": &title,
        "time": {
            "created": now,
            "updated": now
        }
    });

    let mut oc_messages: Vec<(String, i64, Value)> = Vec::new();
    let mut oc_parts: Vec<(String, String, i64, Value)> = Vec::new();
    let mut last_user_msg_id: Option<String> = None;
    let native_compression_target =
        compression::target_projection(PROVIDER_ID) == CompressionProjection::Native;

    for event in &session.events {
        if native_compression_target {
            if let Some(NativeTargetProjection::OpencodeCompaction(segment)) =
                compression::native_target_projection_for_event(PROVIDER_ID, event)
            {
                append_compressed_opencode_segment(
                    &session_id,
                    event,
                    segment,
                    &target_dir_str,
                    &mut last_user_msg_id,
                    &mut oc_messages,
                    &mut oc_parts,
                );
                continue;
            }
        }

        let msg_id = generate_opencode_id("msg");
        let msg_created = event.timestamp.timestamp_millis();
        let (role, parent_id) = match event.role {
            EventRole::Assistant => ("assistant", last_user_msg_id.clone()),
            EventRole::User => {
                last_user_msg_id = Some(msg_id.clone());
                ("user", None)
            }
            _ => {
                last_user_msg_id = Some(msg_id.clone());
                ("user", None)
            }
        };

        let msg_json = build_opencode_message_data_from_event(
            &session_id,
            event,
            &msg_id,
            role,
            parent_id.as_deref(),
            &target_dir_str,
        );
        oc_messages.push((msg_id.clone(), msg_created, msg_json));

        for block in &event.blocks {
            let part_id = generate_opencode_id("prt");
            let part_created = msg_created + 1;
            let part_json = canonical_block_to_opencode_part(
                &session_id,
                &msg_id,
                &part_id,
                block,
                part_created,
            );
            oc_parts.push((part_id, msg_id.clone(), part_created, part_json));
        }
    }

    write_to_db(
        &session_id,
        &project_id,
        &slug,
        &target_dir_str,
        &title,
        now,
        &oc_messages,
        &oc_parts,
    )
    .context("Failed to write to OpenCode SQLite database")?;
    load_session_from_db(&session_id).context("Failed to verify OpenCode SQLite write result")?;
    write_to_filesystem(
        &session_id,
        &project_id,
        &session_json,
        &oc_messages,
        &oc_parts,
    )?;

    Ok(session_id)
}

fn append_compressed_opencode_segment(
    session_id: &str,
    event: &SessionEvent,
    segment: CompressedSegment<'_>,
    target_dir: &str,
    last_user_msg_id: &mut Option<String>,
    oc_messages: &mut Vec<(String, i64, Value)>,
    oc_parts: &mut Vec<(String, String, i64, Value)>,
) {
    let created = event.timestamp.timestamp_millis();
    let marker_msg_id = generate_opencode_id("msg");
    let summary_msg_id = generate_opencode_id("msg");
    let marker_part_id = generate_opencode_id("prt");
    let summary_part_id = generate_opencode_id("prt");

    let mut marker_msg = build_opencode_message_data_from_event(
        session_id,
        event,
        &marker_msg_id,
        "user",
        None,
        target_dir,
    );
    if let Some(obj) = marker_msg.as_object_mut() {
        obj.insert("mode".to_string(), Value::String("compaction".to_string()));
        obj.insert("agent".to_string(), Value::String("compaction".to_string()));
    }
    oc_messages.push((marker_msg_id.clone(), created, marker_msg));
    let marker_part = opencode_compaction_part(
        session_id,
        &marker_msg_id,
        &marker_part_id,
        segment.source_provider_id,
        segment.source_event_ids,
        segment.source_event_count,
        segment.archive_ref,
    );
    oc_parts.push((
        marker_part_id,
        marker_msg_id.clone(),
        created + 1,
        marker_part,
    ));

    let mut summary_msg = build_opencode_message_data_from_event(
        session_id,
        event,
        &summary_msg_id,
        "assistant",
        Some(&marker_msg_id),
        target_dir,
    );
    if let Some(obj) = summary_msg.as_object_mut() {
        obj.insert("summary".to_string(), Value::Bool(true));
        obj.insert("mode".to_string(), Value::String("compaction".to_string()));
        obj.insert("agent".to_string(), Value::String("compaction".to_string()));
    }
    oc_messages.push((summary_msg_id.clone(), created + 2, summary_msg));
    oc_parts.push((
        summary_part_id.clone(),
        summary_msg_id.clone(),
        created + 3,
        serde_json::json!({
            "id": summary_part_id,
            "sessionID": session_id,
            "messageID": summary_msg_id,
            "type": "text",
            "text": segment.summary,
        }),
    ));

    *last_user_msg_id = Some(marker_msg_id);
}

fn opencode_compaction_part(
    session_id: &str,
    msg_id: &str,
    part_id: &str,
    source_provider_id: &str,
    source_event_ids: &[String],
    source_event_count: Option<usize>,
    archive_ref: Option<&str>,
) -> Value {
    let mut part = serde_json::json!({
        "id": part_id,
        "sessionID": session_id,
        "messageID": msg_id,
        "type": "compaction",
        "auto": false,
        "memorph": {
            "sourceProviderID": source_provider_id,
            "sourceEventIDs": source_event_ids,
            "sourceEventCount": source_event_count,
        }
    });
    if let Some(archive_ref) = archive_ref {
        part["memorph"]["archiveRef"] = Value::String(archive_ref.to_string());
        part["memorph"]["retrievalHint"] = Value::String(compression_retrieval_hint(archive_ref));
    }
    part
}

fn build_opencode_message_data_from_event(
    session_id: &str,
    event: &SessionEvent,
    msg_id: &str,
    role: &str,
    parent_id: Option<&str>,
    target_dir: &str,
) -> Value {
    let provider_id = normalize_provider_id(event.metadata.source.provider_id.as_str());
    let model_id = event
        .metadata
        .model
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_model_id(&provider_id));
    let mut msg_json = serde_json::Map::new();
    msg_json.insert("id".to_string(), Value::String(msg_id.to_string()));
    msg_json.insert(
        "sessionID".to_string(),
        Value::String(session_id.to_string()),
    );
    msg_json.insert("role".to_string(), Value::String(role.to_string()));
    msg_json.insert(
        "time".to_string(),
        serde_json::json!({"created": event.timestamp.timestamp_millis()}),
    );
    if let Some(parent_id) = parent_id {
        msg_json.insert("parentID".to_string(), Value::String(parent_id.to_string()));
    }
    msg_json.insert("providerID".to_string(), Value::String(provider_id.clone()));
    msg_json.insert("modelID".to_string(), Value::String(model_id.to_string()));
    msg_json.insert(
        "model".to_string(),
        serde_json::json!({
            "providerID": provider_id,
            "modelID": model_id,
        }),
    );
    msg_json.insert("agent".to_string(), Value::String("build".to_string()));
    msg_json.insert("mode".to_string(), Value::String("build".to_string()));
    msg_json.insert(
        "tokens".to_string(),
        serde_json::json!({
            "input": event.metadata.usage.as_ref().and_then(|usage| usage.input_tokens).unwrap_or(0),
            "output": event.metadata.usage.as_ref().and_then(|usage| usage.output_tokens).unwrap_or(0),
            "reasoning": 0,
            "cache": {"read": 0, "write": 0},
        }),
    );
    if role == "assistant" {
        msg_json.insert(
            "path".to_string(),
            serde_json::json!({"cwd": target_dir, "root": target_dir}),
        );
        msg_json.insert("cost".to_string(), Value::from(0));
        msg_json.insert("finish".to_string(), Value::String("stop".to_string()));
    }
    Value::Object(msg_json)
}

fn canonical_block_to_opencode_part(
    session_id: &str,
    msg_id: &str,
    part_id: &str,
    block: &EventBlock,
    part_created: i64,
) -> Value {
    match block {
        EventBlock::Text { text } => serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "text",
            "text": text,
        }),
        EventBlock::Thinking { text, .. } => serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "reasoning",
            "text": text,
        }),
        EventBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "tool",
            "callID": tool_call_id,
            "tool": "unknown",
            "state": {
                "status": if *is_error { "error" } else { "completed" },
                "input": {},
                "output": content,
                "title": "Tool result",
                "metadata": {},
                "time": {
                    "start": part_created,
                    "end": part_created
                }
            }
        }),
        EventBlock::Image {
            mime_type, data, ..
        } => serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "file",
            "mime": mime_type,
            "filename": "image.png",
            "url": data.as_deref().unwrap_or(""),
        }),
        EventBlock::File { path, content, .. } => serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "file",
            "mime": "text/plain",
            "filename": path,
            "url": content.as_deref().unwrap_or(""),
        }),
        _ => serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "text",
            "text": canonical_block_text(block),
        }),
    }
}

fn parse_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

fn normalize_provider_id(provider: &str) -> String {
    match provider {
        "claude" | "anthropic" => "anthropic".to_string(),
        "codex" | "openai" => "openai".to_string(),
        "opencode" => "opencode".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "openai".to_string(),
    }
}

fn default_model_id(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-sonnet-4-5",
        _ => "gpt-5.3-codex",
    }
}

// ---------------------------------------------------------------------------
// DB operations
// ---------------------------------------------------------------------------

fn get_db_path() -> PathBuf {
    get_opencode_dir().join("opencode.db")
}

/// Estimate session size from OpenCode SQLite DB.
fn opencode_session_db_size(session_id: &str) -> Result<u64> {
    let db_path = get_db_path();
    if !db_path.exists() {
        return Ok(0);
    }
    let conn = Connection::open(&db_path)?;
    opencode_session_db_size_with_conn(&conn, session_id)
}

fn opencode_session_files_size(session_id: &str) -> Result<Option<u64>> {
    let storage_dir = get_opencode_dir().join("storage");
    let session_path = storage_dir
        .join("session")
        .join(format!("{}.json", session_id));
    if !session_path.exists() {
        return Ok(None);
    }

    let mut total = std::fs::metadata(&session_path)?.len();
    let msg_dir = storage_dir.join("message").join(session_id);
    if msg_dir.exists() {
        for entry in std::fs::read_dir(&msg_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                total += std::fs::metadata(&path)?.len();
            }
        }
    }
    Ok(Some(total))
}

fn opencode_session_db_size_with_conn(conn: &Connection, session_id: &str) -> Result<u64> {
    let mut total: u64 = 0;

    // Session row size (approximate via JSON fields)
    if let Ok(size) = conn.query_row(
        "SELECT length(id) + length(project_id) + COALESCE(length(parent_id), 0) + length(slug) + length(directory) + length(title) + length(version) + COALESCE(length(share_url), 0) + COALESCE(length(summary_diffs), 0) + COALESCE(length(revert), 0) + COALESCE(length(permission), 0) FROM session WHERE id = ?1",
        [session_id],
        |row| row.get::<_, i64>(0),
    ) {
        total += size as u64;
    }

    // Messages size
    let mut stmt = conn.prepare("SELECT length(id) + length(session_id) + length(role) + COALESCE(length(content), 0) + COALESCE(length(name), 0) FROM message WHERE session_id = ?1")?;
    let rows = stmt.query_map([session_id], |row| row.get::<_, i64>(0))?;
    for row in rows {
        if let Ok(size) = row {
            total += size as u64;
        }
    }

    // Parts size
    let mut stmt = conn.prepare("SELECT length(id) + length(message_id) + length(session_id) + length(data) FROM part WHERE session_id = ?1")?;
    let rows = stmt.query_map([session_id], |row| row.get::<_, i64>(0))?;
    for row in rows {
        if let Ok(size) = row {
            total += size as u64;
        }
    }

    Ok(total)
}

fn scan_sessions_from_db() -> Result<Vec<ProviderSessionSummary>> {
    let db_path = get_db_path();
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open(&db_path)?;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, directory, title, time_created, time_updated FROM session WHERE time_archived IS NULL ORDER BY time_updated DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        let session_id: String = row.get(0)?;
        let _project_id: String = row.get(1)?;
        let directory: String = row.get(2)?;
        let title: String = row.get(3)?;
        let _created: i64 = row.get(4)?;
        let updated: i64 = row.get(5)?;
        Ok(ProviderSessionSummary {
            session_id: session_id.clone(),
            title: Some(title),
            project_dir: Some(directory),
            last_active_at: Some(updated),
            source_path: Some(session_id),
        })
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        if let Ok(s) = row {
            sessions.push(s);
        }
    }
    Ok(sessions)
}

fn load_session_from_db(
    session_id: &str,
) -> Result<(Value, Vec<(i64, Value)>, HashMap<String, Vec<Value>>)> {
    let db_path = get_db_path();
    let conn = Connection::open(&db_path)?;

    // Load session
    let session_json: Value = conn.query_row(
        "SELECT id, project_id, parent_id, slug, directory, title, version, share_url, summary_additions, summary_deletions, summary_files, summary_diffs, revert, permission, time_created, time_updated, time_compacting, time_archived, workspace_id FROM session WHERE id = ?1",
        [session_id],
        |row| {
            let mut obj = serde_json::Map::new();
            obj.insert("id".to_string(), Value::String(row.get(0)?));
            obj.insert("projectID".to_string(), Value::String(row.get(1)?));
            if let Ok(Some(v)) = row.get::<_, Option<String>>(2) {
                obj.insert("parentID".to_string(), Value::String(v));
            }
            obj.insert("slug".to_string(), Value::String(row.get(3)?));
            obj.insert("directory".to_string(), Value::String(row.get(4)?));
            obj.insert("title".to_string(), Value::String(row.get(5)?));
            obj.insert("version".to_string(), Value::String(row.get(6)?));
            if let Ok(Some(v)) = row.get::<_, Option<String>>(7) {
                obj.insert("shareURL".to_string(), Value::String(v));
            }
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(8) {
                obj.insert("summaryAdditions".to_string(), Value::Number(v.into()));
            }
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(9) {
                obj.insert("summaryDeletions".to_string(), Value::Number(v.into()));
            }
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(10) {
                obj.insert("summaryFiles".to_string(), Value::Number(v.into()));
            }
            if let Ok(Some(v)) = row.get::<_, Option<String>>(11) {
                obj.insert("summaryDiffs".to_string(), Value::String(v));
            }
            if let Ok(Some(v)) = row.get::<_, Option<String>>(12) {
                obj.insert("revert".to_string(), Value::String(v));
            }
            if let Ok(Some(v)) = row.get::<_, Option<String>>(13) {
                obj.insert("permission".to_string(), Value::String(v));
            }
            let created: i64 = row.get(14)?;
            let updated: i64 = row.get(15)?;
            let mut time = serde_json::Map::new();
            time.insert("created".to_string(), Value::Number(created.into()));
            time.insert("updated".to_string(), Value::Number(updated.into()));
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(16) {
                time.insert("compacting".to_string(), Value::Number(v.into()));
            }
            if let Ok(Some(v)) = row.get::<_, Option<i64>>(17) {
                time.insert("archived".to_string(), Value::Number(v.into()));
            }
            obj.insert("time".to_string(), Value::Object(time));
            if let Ok(Some(v)) = row.get::<_, Option<String>>(18) {
                obj.insert("workspaceID".to_string(), Value::String(v));
            }
            Ok(Value::Object(obj))
        },
    )?;

    // Load messages
    let mut stmt = conn.prepare(
        "SELECT id, session_id, time_created, time_updated, data FROM message WHERE session_id = ?1 ORDER BY time_created"
    )?;
    let rows = stmt.query_map([session_id], |row| {
        let msg_id: String = row.get(0)?;
        let session_id: String = row.get(1)?;
        let created: i64 = row.get(2)?;
        let _updated: i64 = row.get(3)?;
        let data_str: String = row.get(4)?;
        let mut data: Value = serde_json::from_str(&data_str).unwrap_or_default();
        if let Value::Object(ref mut map) = data {
            map.insert("id".to_string(), Value::String(msg_id));
            map.insert("sessionID".to_string(), Value::String(session_id));
        }
        Ok((created, data))
    })?;

    let mut messages = Vec::new();
    for row in rows {
        if let Ok(r) = row {
            messages.push(r);
        }
    }

    // Load parts
    let mut parts_map: HashMap<String, Vec<Value>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, message_id, session_id, time_created, time_updated, data FROM part WHERE session_id = ?1"
    )?;
    let rows = stmt.query_map([session_id], |row| {
        let part_id: String = row.get(0)?;
        let message_id: String = row.get(1)?;
        let session_id: String = row.get(2)?;
        let _created: i64 = row.get(3)?;
        let _updated: i64 = row.get(4)?;
        let data_str: String = row.get(5)?;
        let mut data: Value = serde_json::from_str(&data_str).unwrap_or_default();
        if let Value::Object(ref mut map) = data {
            map.insert("id".to_string(), Value::String(part_id));
            map.insert("messageID".to_string(), Value::String(message_id.clone()));
            map.insert("sessionID".to_string(), Value::String(session_id));
        }
        Ok((message_id, data))
    })?;

    for row in rows {
        if let Ok((msg_id, part)) = row {
            parts_map.entry(msg_id).or_default().push(part);
        }
    }

    Ok((session_json, messages, parts_map))
}

fn load_session_from_filesystem(
    session_id: &str,
) -> Result<(Value, Vec<(i64, Value)>, HashMap<String, Vec<Value>>)> {
    let storage_dir = get_opencode_dir().join("storage");

    // Find session file
    let mut session_path: Option<PathBuf> = None;
    for entry in WalkDir::new(storage_dir.join("session"))
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
            session_path = Some(path.to_path_buf());
            break;
        }
    }

    let session_path = session_path.context("Session not found in filesystem")?;
    let session_json: Value = serde_json::from_reader(File::open(&session_path)?)?;

    // Load messages from filesystem
    let mut messages = Vec::new();
    let msg_dir = storage_dir.join("message").join(session_id);
    if msg_dir.exists() {
        for entry in std::fs::read_dir(&msg_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let msg_json: Value = serde_json::from_reader(File::open(&path)?)?;
            let created = msg_json
                .get("time")
                .and_then(|v| v.get("created"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            messages.push((created, msg_json));
        }
    }

    // Load parts from filesystem
    let mut parts_map: HashMap<String, Vec<Value>> = HashMap::new();
    let parts_dir = storage_dir.join("part");
    if parts_dir.exists() {
        for entry in std::fs::read_dir(&parts_dir)? {
            let entry = entry?;
            let msg_id = entry.file_name().to_string_lossy().to_string();
            let msg_parts_dir = entry.path();
            if !msg_parts_dir.is_dir() {
                continue;
            }
            for part_entry in std::fs::read_dir(&msg_parts_dir)? {
                let part_entry = part_entry?;
                let part_path = part_entry.path();
                if part_path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let part_json: Value = serde_json::from_reader(File::open(&part_path)?)?;
                parts_map.entry(msg_id.clone()).or_default().push(part_json);
            }
        }
    }

    Ok((session_json, messages, parts_map))
}

fn parse_session_file(path: &Path) -> Option<ProviderSessionSummary> {
    let file = File::open(path).ok()?;
    let json: Value = serde_json::from_reader(file).ok()?;

    let session_id = json.get("id").and_then(|v| v.as_str())?.to_string();
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let directory = json
        .get("directory")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let _created = json
        .get("time")
        .and_then(|v| v.get("created"))
        .and_then(|v| v.as_i64());
    let updated = json
        .get("time")
        .and_then(|v| v.get("updated"))
        .and_then(|v| v.as_i64());

    Some(ProviderSessionSummary {
        session_id: session_id.clone(),
        title,
        project_dir: directory,
        last_active_at: updated,
        source_path: Some(session_id),
    })
}

fn find_or_create_project(target_dir: &Path) -> Result<String> {
    let db_path = get_db_path();
    if !db_path.exists() {
        // Generate a deterministic project ID from path
        return Ok(generate_project_id(target_dir));
    }

    let conn = Connection::open(&db_path)?;
    let target_dir_str = target_dir.to_string_lossy().to_string();

    // Try to find existing project
    let existing: Result<String, _> = conn.query_row(
        "SELECT id FROM project WHERE worktree = ?1 ORDER BY time_updated DESC LIMIT 1",
        [&target_dir_str],
        |row| row.get(0),
    );

    if let Ok(id) = existing {
        return Ok(id);
    }

    // Create new project
    let project_id = generate_project_id(target_dir);
    let now = Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO project (id, worktree, vcs, name, time_created, time_updated, sandboxes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        [
            &project_id,
            &target_dir_str,
            &get_git_remote(target_dir).unwrap_or_default(),
            &target_dir.file_name().and_then(|n| n.to_str()).unwrap_or("project").to_string(),
            &now.to_string(),
            &now.to_string(),
            "{}",
        ],
    )?;

    Ok(project_id)
}

fn generate_project_id(target_dir: &Path) -> String {
    // Use a SHA256 of the absolute path for determinism
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let path_str = target_dir.to_string_lossy().to_string();
    let mut hasher = DefaultHasher::new();
    path_str.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{:016x}{:024x}", hash, hash)
}

fn get_git_remote(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "remote", "get-url", "origin"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn write_to_db(
    session_id: &str,
    project_id: &str,
    slug: &str,
    directory: &str,
    title: &str,
    now: i64,
    messages: &[(String, i64, Value)],
    parts: &[(String, String, i64, Value)],
) -> Result<()> {
    let db_path = get_db_path();
    if !db_path.exists() {
        anyhow::bail!(
            "OpenCode database does not exist: {}. Please launch OpenCode once to initialize storage before importing.",
            db_path.display()
        );
    }
    let mut conn = Connection::open(&db_path)?;

    let tx = conn.transaction()?;

    // Insert session
    tx.execute(
        "INSERT INTO session (id, project_id, slug, directory, title, version, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        [session_id, project_id, slug, directory, title, OPENCODE_VERSION, &now.to_string(), &now.to_string()],
    )?;

    // Insert messages
    for (msg_id, created, data) in messages {
        // Strip id/sessionID from JSON for DB storage (they're in columns)
        let mut db_data = data.clone();
        if let Value::Object(ref mut map) = db_data {
            map.remove("id");
            map.remove("sessionID");
        }
        let data_str = serde_json::to_string(&db_data)?;
        tx.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            [msg_id, session_id, &created.to_string(), &created.to_string(), &data_str],
        )?;
    }

    // Insert parts
    for (part_id, msg_id, created, data) in parts {
        let mut db_data = data.clone();
        if let Value::Object(ref mut map) = db_data {
            map.remove("id");
            map.remove("messageID");
            map.remove("sessionID");
        }
        let data_str = serde_json::to_string(&db_data)?;
        tx.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            [part_id, msg_id, session_id, &created.to_string(), &created.to_string(), &data_str],
        )?;
    }

    tx.commit()?;
    Ok(())
}

fn write_to_filesystem(
    session_id: &str,
    project_id: &str,
    session_json: &Value,
    messages: &[(String, i64, Value)],
    parts: &[(String, String, i64, Value)],
) -> Result<()> {
    let storage_dir = get_opencode_dir().join("storage");

    // Write session
    let session_dir = storage_dir.join("session").join(project_id);
    std::fs::create_dir_all(&session_dir)?;
    let session_file = session_dir.join(format!("{}.json", session_id));
    std::fs::write(&session_file, serde_json::to_string_pretty(session_json)?)?;

    // Write messages
    let msg_dir = storage_dir.join("message").join(session_id);
    std::fs::create_dir_all(&msg_dir)?;
    for (msg_id, _created, data) in messages {
        let msg_file = msg_dir.join(format!("{}.json", msg_id));
        std::fs::write(&msg_file, serde_json::to_string_pretty(data)?)?;
    }

    // Write parts
    let parts_base = storage_dir.join("part");
    for (part_id, msg_id, _created, data) in parts {
        let part_dir = parts_base.join(msg_id);
        std::fs::create_dir_all(&part_dir)?;
        let part_file = part_dir.join(format!("{}.json", part_id));
        std::fs::write(&part_file, serde_json::to_string_pretty(data)?)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn opencode_message_data_preserves_model_provider_metadata() {
        let event = SessionEvent {
            id: "source-message".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            blocks: vec![EventBlock::Text {
                text: "hello".to_string(),
            }],
            timestamp: Utc
                .timestamp_millis_opt(1_700_000_000_000)
                .single()
                .unwrap(),
            links: EventLinks::default(),
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "codex".to_string(),
                    original_id: None,
                    original_role: None,
                    phase: None,
                },
                model: Some("gpt-5.4".to_string()),
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: BTreeMap::new(),
            },
        };

        let data = build_opencode_message_data_from_event(
            "ses_test",
            &event,
            "msg_test",
            "assistant",
            Some("msg_parent"),
            "/tmp/project",
        );
        let obj = data.as_object().expect("message data should be an object");

        assert_eq!(obj.get("role").and_then(Value::as_str), Some("assistant"));
        assert_eq!(
            obj.get("parentID").and_then(Value::as_str),
            Some("msg_parent")
        );
        assert_eq!(
            obj.get("providerID").and_then(Value::as_str),
            Some("openai")
        );
        assert_eq!(obj.get("modelID").and_then(Value::as_str), Some("gpt-5.4"));
        assert_eq!(
            obj.get("model")
                .and_then(|value| value.get("providerID"))
                .and_then(Value::as_str),
            Some("openai")
        );
        assert_eq!(obj.get("agent").and_then(Value::as_str), Some("build"));
        assert!(obj.get("path").is_some());
        assert!(obj.get("tokens").is_some());
    }

    #[test]
    fn compressed_segment_exports_as_native_opencode_compaction() {
        let event = SessionEvent {
            id: "compressed-source".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            blocks: vec![EventBlock::Compressed {
                source_provider_id: "opencode".to_string(),
                summary: "portable summary".to_string(),
                source_event_ids: vec!["old-1".to_string(), "old-2".to_string()],
                source_event_count: Some(2),
                archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
            }],
            timestamp: Utc
                .timestamp_millis_opt(1_700_000_000_000)
                .single()
                .unwrap(),
            links: EventLinks::default(),
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "memorph".to_string(),
                    original_id: None,
                    original_role: None,
                    phase: Some("compression".to_string()),
                },
                model: Some("gpt-5.4".to_string()),
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };
        let mut last_user_msg_id = None;
        let mut messages = Vec::new();
        let mut parts = Vec::new();
        let projection = compression::native_target_projection_for_event(PROVIDER_ID, &event)
            .expect("native opencode projection");
        let NativeTargetProjection::OpencodeCompaction(segment) = projection else {
            panic!("expected opencode compaction projection");
        };

        append_compressed_opencode_segment(
            "ses_test",
            &event,
            segment,
            "/tmp/project",
            &mut last_user_msg_id,
            &mut messages,
            &mut parts,
        );

        assert_eq!(messages.len(), 2);
        assert_eq!(parts.len(), 2);
        assert_eq!(
            messages[0].1,
            Utc.timestamp_millis_opt(1_700_000_000_000)
                .single()
                .unwrap()
                .timestamp_millis()
        );
        assert_eq!(
            messages[0].2.get("role").and_then(Value::as_str),
            Some("user")
        );
        assert_eq!(
            messages[0].2.get("mode").and_then(Value::as_str),
            Some("compaction")
        );
        assert_eq!(
            parts[0].3.get("type").and_then(Value::as_str),
            Some("compaction")
        );
        assert_eq!(
            parts[0]
                .3
                .get("memorph")
                .and_then(|value| value.get("sourceProviderID"))
                .and_then(Value::as_str),
            Some("opencode")
        );
        assert_eq!(
            parts[0]
                .3
                .get("memorph")
                .and_then(|value| value.get("sourceEventCount"))
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            parts[0]
                .3
                .get("memorph")
                .and_then(|value| value.get("archiveRef"))
                .and_then(Value::as_str),
            Some("memorph-archive://s1/archive.json.gz")
        );
        assert_eq!(
            parts[0]
                .3
                .get("memorph")
                .and_then(|value| value.get("retrievalHint"))
                .and_then(Value::as_str),
            Some("Retrieve specific details with: memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5")
        );
        assert_eq!(
            messages[1].2.get("summary").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            messages[1].2.get("parentID").and_then(Value::as_str),
            Some(messages[0].0.as_str())
        );
        assert_eq!(parts[1].3.get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            parts[1].3.get("text").and_then(Value::as_str),
            Some("portable summary")
        );
    }
}
