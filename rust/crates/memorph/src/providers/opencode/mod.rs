pub mod adapter;
pub mod backup;
pub mod hook;
use self::backup::*;

use crate::canonical::{
    ArtifactKind, CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata,
    EventRole, EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionArtifact,
    SessionContext, SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
    TurnBoundary, UsageStats,
};
use crate::core::compression::{self, CompressedSegment};
use crate::provider::{
    canonical_event_is_visible_message, canonical_event_visible_message_role,
    canonical_export_result, canonical_session_title, canonical_visible_block_text,
    compression_retrieval_hint, CompressionProjection, PageStrategy, Provider,
    ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities, ProviderContentFidelity,
    ProviderSessionBackup, ProviderSessionImportPage, ProviderSessionSummary,
    ProviderSourceMutation, ProviderWriteRisk, ResumeQuality, ScanStrategy, StorageShape,
    TurnQuality, WriteRiskLevel,
};
use crate::session_projection::project_session_turns;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct OpenCodeProvider;

const PROVIDER_ID: &str = "opencode";
const OPENCODE_VERSION: &str = "1.3.17";
const OPENCODE_BACKUP_FORMAT: &str = "opencode-session-backup-v1";
const OPENCODE_BACKUP_MIME: &str = "application/vnd.memorph.opencode-session-backup";
const OPENCODE_BACKUP_DB_PATH: &str = "sqlite/opencode-session.db";

#[cfg(test)]
static TEST_OPENCODE_STATE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_OPENCODE_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_OPENCODE_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    opencode_dir: PathBuf,
    db_path: PathBuf,
    database_present: bool,
    sqlite_tables: Vec<OpenCodeSqliteTableManifest>,
    filesystem_entries: Vec<OpenCodeFilesystemEntryBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeSqliteTableManifest {
    table: String,
    columns: Vec<String>,
    row_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OpenCodeFilesystemEntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeFilesystemEntryBackup {
    source_path: PathBuf,
    relative_path: PathBuf,
    kind: OpenCodeFilesystemEntryKind,
    present: bool,
}

#[derive(Debug, Clone)]
struct OpenCodeForeignKey {
    child_table: String,
    child_columns: Vec<String>,
    parent_table: String,
    parent_columns: Vec<Option<String>>,
    on_delete: String,
}

#[derive(Debug, Clone)]
struct OpenCodeMutationPaths {
    session_files: Vec<PathBuf>,
    message_dir: PathBuf,
    part_dirs: Vec<PathBuf>,
}

impl Provider for OpenCodeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "OpenCode"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: true,
            scan_strategy: ScanStrategy::Hybrid,
            page_strategy: PageStrategy::NativePage,
            storage_shape: StorageShape::Mixed,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Preserved),
                tool_call: Some(MappingDisposition::Preserved),
                tool_result: Some(MappingDisposition::Preserved),
                patch: Some(MappingDisposition::Preserved),
                image: Some(MappingDisposition::Normalized),
                file: Some(MappingDisposition::Normalized),
                compressed: Some(MappingDisposition::Downgraded),
                provider_payload: Some(MappingDisposition::Preserved),
            },
            export_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Preserved),
                tool_call: Some(MappingDisposition::Downgraded),
                tool_result: Some(MappingDisposition::Downgraded),
                patch: Some(MappingDisposition::Downgraded),
                image: Some(MappingDisposition::Downgraded),
                file: Some(MappingDisposition::Downgraded),
                compressed: Some(MappingDisposition::Preserved),
                provider_payload: Some(MappingDisposition::Dropped),
            },
            resume_quality: ResumeQuality::Native,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: true,
                sidecar_files: true,
                index_repair: false,
            },
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
            },
            activity_support: ProviderActivitySupport {
                hook_events: true,
                runtime_endpoint: true,
                session_activity: true,
            },
        }
    }

    fn detects_native_compression_source(&self) -> bool {
        true
    }

    fn compression_projection(&self) -> CompressionProjection {
        CompressionProjection::Native
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let mut sessions = BTreeMap::new();

        // OpenCode exposes two native source planes. A session id collision is
        // represented once using the database locator, which is stable across
        // filesystem layout changes.
        for session in scan_sessions_from_db()? {
            sessions.insert(session.session_id.clone(), session);
        }
        for session in scan_sessions_from_filesystem()? {
            sessions
                .entry(session.session_id.clone())
                .or_insert(session);
        }

        Ok(sessions.into_values().collect())
    }

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        Ok(self
            .scan_sessions()?
            .into_iter()
            .find(|session| session.session_id == session_id))
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let session_id = opencode_session_id_from_source_locator(source_path)?;
        let mut imported = import_canonical_session_from_source(&session_id, source_path)?;
        imported.session.provenance.primary_source.source_path = Some(source_path.to_string());
        Ok(imported)
    }

    fn import_session_page(
        &self,
        source_path: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<ProviderSessionImportPage> {
        import_opencode_session_page(source_path, event_offset, event_limit)
    }

    fn supports_native_session_replace(&self) -> bool {
        true
    }

    fn replace_session(&self, session_id: &str, session: &CanonicalSession) -> Result<()> {
        replace_opencode_session(session_id, session)
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
            session,
            self.capabilities(),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        delete_opencode_session(session_id)
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        rename_opencode_session(session_id, new_title)
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        create_opencode_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        restore_opencode_session_backup(backup)
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

    fn data_source_paths(&self) -> Vec<PathBuf> {
        vec![get_db_path(), get_opencode_dir()]
    }
}

fn opencode_session_id_from_source_locator(source_locator: &str) -> Result<String> {
    if let Some((_, session_id)) = opencode_database_source(source_locator) {
        return Ok(session_id.to_string());
    }
    anyhow::ensure!(
        !source_locator.contains('#'),
        "OpenCode database source locator is invalid"
    );

    let source_path = Path::new(source_locator);
    if source_path.is_file() {
        let source: Value =
            serde_json::from_reader(File::open(source_path)?).with_context(|| {
                format!(
                    "Failed to parse OpenCode session source: {}",
                    source_path.display()
                )
            })?;
        return source
            .get("id")
            .and_then(Value::as_str)
            .filter(|session_id| !session_id.is_empty())
            .map(str::to_string)
            .context("OpenCode session source has no id");
    }

    if !source_locator.is_empty() && !source_locator.contains('/') && !source_locator.contains('\\')
    {
        return Ok(source_locator.to_string());
    }

    anyhow::bail!("OpenCode source locator does not exist: {source_locator}")
}

fn import_canonical_session_from_source(
    session_id: &str,
    source_locator: &str,
) -> Result<ImportedSession> {
    let data = if let Some((database_path, locator_session_id)) =
        opencode_database_source(source_locator)
    {
        anyhow::ensure!(
            locator_session_id == session_id,
            "OpenCode database source locator session does not match projected session"
        );
        load_session_from_db_path(Path::new(database_path), session_id)?
    } else {
        let source_path = Path::new(source_locator);
        if source_path.is_file() {
            load_session_from_filesystem_path(session_id, source_path)?
        } else {
            anyhow::bail!(
                "OpenCode source locator does not identify a native source plane: {source_locator}"
            );
        }
    };
    imported_session_from_data(session_id, data)
}

/// Load a single page of an OpenCode session by event (message) range.
///
/// OpenCode stores sessions as structured rows/files rather than an append-only
/// line stream, so pagination is native to the source plane: the database plane
/// uses SQL LIMIT/OFFSET and the filesystem plane skips/takes sorted message
/// files. No separate byte-offset index is needed because the source itself is
/// the index. `event_count` is the total message count for the session.
///
/// `message_count` (visible messages across the whole session) and the page
/// events are derived from the same canonical mapping used by a full import, so
/// counts and per-page visibility stay identical. The full message list is read
/// for counting (cheap: SQL rows / file list) but only the requested page is
/// materialized into canonical events, which is where the prior full-import
/// cost came from.
fn import_opencode_session_page(
    source_locator: &str,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let session_id = opencode_session_id_from_source_locator(source_locator)?;

    let (session_json, messages_page, parts_page, total_message_count, full_message_count) =
        if let Some((database_path, locator_session_id)) = opencode_database_source(source_locator)
        {
            anyhow::ensure!(
                locator_session_id == session_id,
                "OpenCode database source locator session does not match projected session"
            );
            load_session_page_from_db_path(
                Path::new(database_path),
                &session_id,
                event_offset,
                event_limit,
            )?
        } else {
            let source_path = Path::new(source_locator);
            if source_path.is_file() {
                load_session_page_from_filesystem_path(
                    &session_id,
                    source_path,
                    event_offset,
                    event_limit,
                )?
            } else {
                anyhow::bail!(
                    "OpenCode source locator does not identify a native source plane: {source_locator}"
                );
            }
        };

    let imported =
        imported_session_from_data(&session_id, (session_json, messages_page, parts_page))?;

    let turns = project_session_turns(
        &imported.session.identity.canonical_id,
        &imported.session.events,
        TurnQuality::Inferred,
    );
    let turn_count = (event_offset == 0 && imported.session.events.len() == total_message_count)
        .then_some(turns.len());
    Ok(ProviderSessionImportPage {
        imported,
        event_count: total_message_count,
        message_count: full_message_count,
        turn_count,
        turns,
    })
}

fn opencode_database_source(source_locator: &str) -> Option<(&str, &str)> {
    let (database_path, session_id) = source_locator.rsplit_once("#session=")?;
    (!database_path.is_empty() && !session_id.is_empty()).then_some((database_path, session_id))
}

fn imported_session_from_data(
    session_id: &str,
    (session_json, messages, parts): (
        Value,
        Vec<(Option<i64>, Value)>,
        HashMap<String, Vec<Value>>,
    ),
) -> Result<ImportedSession> {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut artifacts = Vec::new();

    let mut msg_list: Vec<(Option<i64>, Value, Vec<Value>)> = messages
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
    msg_list.sort_by(|(left_created, left, _), (right_created, right, _)| {
        let left_id = left.get("id").and_then(Value::as_str).unwrap_or("");
        let right_id = right.get("id").and_then(Value::as_str).unwrap_or("");
        left_created
            .cmp(right_created)
            .then_with(|| left_id.cmp(right_id))
    });

    for (source_order, (source_created, msg_json, msg_parts)) in msg_list.into_iter().enumerate() {
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
        let timestamp = msg_json
            .get("time")
            .and_then(|v| v.get("created"))
            .and_then(|v| v.as_i64())
            .and_then(chrono::DateTime::from_timestamp_millis)
            .or_else(|| source_created.and_then(chrono::DateTime::from_timestamp_millis))
            .unwrap_or_else(|| {
                chrono::DateTime::from_timestamp_millis(source_order as i64)
                    .expect("OpenCode message source order is a valid timestamp")
            });

        let mut blocks =
            canonical_blocks_from_parts(&msg_id, &msg_parts, &mut report, &mut artifacts);
        if blocks.is_empty() {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Normalized,
                code: "opencode_message_without_mappable_parts".to_string(),
                message:
                    "OpenCode message had no mappable parts and was preserved as provider payload"
                        .to_string(),
                path: Some(format!("message:{}", msg_id)),
                raw: Some(msg_json.clone()),
            });
            blocks = vec![EventBlock::ProviderPayload {
                kind: "message_without_mappable_parts".to_string(),
                payload: msg_json.clone(),
            }];
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
        let turn_boundary = opencode_turn_boundary(finish.as_deref());
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
                provider_turn_id: None,
                turn_index: None,
                turn_boundary,
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

fn opencode_turn_boundary(finish: Option<&str>) -> Option<TurnBoundary> {
    match finish {
        Some("stop") => Some(TurnBoundary::Completed),
        Some("error") => Some(TurnBoundary::Failed),
        Some("abort" | "cancelled" | "canceled" | "length" | "content_filter") => {
            Some(TurnBoundary::Interrupted)
        }
        _ => None,
    }
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
                } else {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "opencode_text_part_missing_text".to_string(),
                        message:
                            "OpenCode text part without text was preserved as provider payload"
                                .to_string(),
                        path: Some(format!("{}:part:{}", msg_id, idx)),
                        raw: Some(part.clone()),
                    });
                    blocks.push(EventBlock::ProviderPayload {
                        kind: "text".to_string(),
                        payload: part.clone(),
                    });
                }
            }
            Some("reasoning") => {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    blocks.push(EventBlock::Thinking {
                        text: text.to_string(),
                        signature: None,
                    });
                } else {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "opencode_reasoning_part_missing_text".to_string(),
                        message:
                            "OpenCode reasoning part without text was preserved as provider payload"
                                .to_string(),
                        path: Some(format!("{}:part:{}", msg_id, idx)),
                        raw: Some(part.clone()),
                    });
                    blocks.push(EventBlock::ProviderPayload {
                        kind: "reasoning".to_string(),
                        payload: part.clone(),
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
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Warning,
                            disposition: MappingDisposition::Normalized,
                            code: "opencode_image_part_invalid_data_uri".to_string(),
                            message: "OpenCode image part with an invalid data URI was preserved as provider payload"
                                .to_string(),
                            path: Some(format!("{}:part:{}", msg_id, idx)),
                            raw: Some(part.clone()),
                        });
                        blocks.push(EventBlock::ProviderPayload {
                            kind: "file".to_string(),
                            payload: part.clone(),
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
                } else {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "opencode_file_part_missing_url".to_string(),
                        message:
                            "OpenCode file part without a URL was preserved as provider payload"
                                .to_string(),
                        path: Some(format!("{}:part:{}", msg_id, idx)),
                        raw: Some(part.clone()),
                    });
                    blocks.push(EventBlock::ProviderPayload {
                        kind: "file".to_string(),
                        payload: part.clone(),
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
    #[cfg(test)]
    if let Some(path) = TEST_OPENCODE_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test opencode dir lock")
        .clone()
    {
        return path;
    }

    // OpenCode uses ~/.local/share/opencode even on macOS
    dirs::home_dir()
        .map(|h| h.join(".local/share/opencode"))
        .unwrap_or_else(|| PathBuf::from(".local/share/opencode"))
}

#[cfg(test)]
pub(crate) fn lock_test_opencode_state() -> std::sync::MutexGuard<'static, ()> {
    TEST_OPENCODE_STATE_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .expect("test opencode state lock")
}

#[cfg(test)]
pub(crate) fn set_test_opencode_dir(path: Option<PathBuf>) {
    *TEST_OPENCODE_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test opencode dir lock") = path;
}

#[cfg(test)]
fn set_test_opencode_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_OPENCODE_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test opencode mutation failure lock") = mutation;
}

#[cfg(test)]
fn fail_opencode_mutation_after_database_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_OPENCODE_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test opencode mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected OpenCode mutation failure after database write");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_opencode_mutation_after_database_write(_mutation: ProviderSourceMutation) -> Result<()> {
    Ok(())
}

fn delete_opencode_session(session_id: &str) -> Result<()> {
    let db_path = get_db_path();
    let database_message_ids = if db_path.exists() {
        let conn = Connection::open(&db_path)?;
        opencode_message_ids(&conn, session_id)?
    } else {
        HashSet::new()
    };
    let mutation_paths = discover_opencode_mutation_paths(session_id, &database_message_ids)?;
    let mut database_deleted = false;

    if db_path.exists() {
        let mut conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let tx = conn.transaction()?;
        database_deleted = tx.execute("DELETE FROM session WHERE id = ?1", [session_id])? > 0;
        tx.commit()?;
    }
    let filesystem_present = !mutation_paths.session_files.is_empty()
        || path_lexists(&mutation_paths.message_dir)
        || !mutation_paths.part_dirs.is_empty();
    if !database_deleted && !filesystem_present {
        anyhow::bail!("OpenCode session not found: {session_id}");
    }

    fail_opencode_mutation_after_database_write(ProviderSourceMutation::Delete)?;
    for session_path in &mutation_paths.session_files {
        remove_opencode_filesystem_entry(session_path)?;
        if let Some(parent) = session_path.parent() {
            match std::fs::remove_dir(parent) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    remove_opencode_filesystem_entry(&mutation_paths.message_dir)?;
    for part_dir in &mutation_paths.part_dirs {
        remove_opencode_filesystem_entry(part_dir)?;
    }
    Ok(())
}

fn replace_opencode_session(session_id: &str, session: &CanonicalSession) -> Result<()> {
    let db_path = get_db_path();
    if !db_path.exists() {
        anyhow::bail!("OpenCode database does not exist: {}", db_path.display());
    }
    let mut conn = Connection::open(&db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    let (project_id, slug, directory, created_at): (String, String, String, i64) = conn
        .query_row(
            "SELECT project_id, slug, directory, time_created FROM session WHERE id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .with_context(|| format!("OpenCode session not found: {session_id}"))?;
    let old_message_ids = opencode_message_ids(&conn, session_id)?;
    let old_paths = discover_opencode_mutation_paths(session_id, &old_message_ids)?;
    let now = Utc::now().timestamp_millis();
    let title = canonical_session_title(session);
    let projection = build_opencode_projection(
        session,
        session_id,
        &project_id,
        &slug,
        &directory,
        &title,
        created_at,
        now,
    );

    let tx = conn.transaction()?;
    tx.execute("DELETE FROM part WHERE session_id = ?1", [session_id])?;
    tx.execute("DELETE FROM message WHERE session_id = ?1", [session_id])?;
    tx.execute(
        "UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3",
        rusqlite::params![title, now, session_id],
    )?;
    insert_opencode_projection_rows(&tx, session_id, &projection.messages, &projection.parts)?;
    tx.commit()?;

    fail_opencode_mutation_after_database_write(ProviderSourceMutation::Replace)?;
    for path in old_paths.session_files {
        remove_opencode_filesystem_entry(&path)?;
    }
    remove_opencode_filesystem_entry(&old_paths.message_dir)?;
    for path in old_paths.part_dirs {
        remove_opencode_filesystem_entry(&path)?;
    }
    write_to_filesystem(
        session_id,
        &project_id,
        &projection.session_json,
        &projection.messages,
        &projection.parts,
    )?;

    let source = opencode_db_session_source_locator(session_id);
    let imported = import_canonical_session_from_source(session_id, &source)?;
    if imported.session.identity.canonical_id != session_id {
        anyhow::bail!("OpenCode replacement validation changed session identity");
    }
    Ok(())
}

fn rename_opencode_session(session_id: &str, new_title: &str) -> Result<()> {
    let db_path = get_db_path();
    let session_files = find_opencode_session_files(session_id)?;
    let now = Utc::now().timestamp_millis();
    let mut database_updated = false;

    if db_path.exists() {
        let mut conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let tx = conn.transaction()?;
        database_updated = tx.execute(
            "UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3",
            rusqlite::params![new_title, now, session_id],
        )? > 0;
        tx.commit()?;
    }
    if !database_updated && session_files.is_empty() {
        anyhow::bail!("OpenCode session not found: {session_id}");
    }

    fail_opencode_mutation_after_database_write(ProviderSourceMutation::Rename)?;
    for path in session_files {
        let content = std::fs::read(&path)
            .with_context(|| format!("Failed to read OpenCode session file: {}", path.display()))?;
        let mut value: Value = serde_json::from_slice(&content).with_context(|| {
            format!("Failed to parse OpenCode session file: {}", path.display())
        })?;
        let object = value.as_object_mut().with_context(|| {
            format!("OpenCode session file is not an object: {}", path.display())
        })?;
        object.insert("title".to_string(), Value::String(new_title.to_string()));
        let time = object
            .get_mut("time")
            .and_then(Value::as_object_mut)
            .with_context(|| {
                format!(
                    "OpenCode session file has no time object: {}",
                    path.display()
                )
            })?;
        time.insert("updated".to_string(), Value::Number(now.into()));
        std::fs::write(&path, serde_json::to_vec_pretty(&value)?).with_context(|| {
            format!("Failed to write OpenCode session file: {}", path.display())
        })?;
    }
    Ok(())
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

struct OpenCodeProjection {
    session_json: Value,
    messages: Vec<(String, i64, Value)>,
    parts: Vec<(String, String, i64, Value)>,
}

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    let now = Utc::now().timestamp_millis();
    let session_id = generate_opencode_id("ses");
    let project_id = find_or_create_project(target_dir)?;
    let slug = generate_slug();
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = canonical_session_title(session);
    let projection = build_opencode_projection(
        session,
        &session_id,
        &project_id,
        &slug,
        &target_dir_str,
        &title,
        now,
        now,
    );

    write_to_db(
        &session_id,
        &project_id,
        &slug,
        &target_dir_str,
        &title,
        now,
        &projection.messages,
        &projection.parts,
    )
    .context("Failed to write to OpenCode SQLite database")?;
    load_session_from_db(&session_id).context("Failed to verify OpenCode SQLite write result")?;
    write_to_filesystem(
        &session_id,
        &project_id,
        &projection.session_json,
        &projection.messages,
        &projection.parts,
    )?;

    Ok(session_id)
}

fn build_opencode_projection(
    session: &CanonicalSession,
    session_id: &str,
    project_id: &str,
    slug: &str,
    target_dir_str: &str,
    title: &str,
    created_at: i64,
    updated_at: i64,
) -> OpenCodeProjection {
    let session_json = serde_json::json!({
        "id": session_id,
        "slug": slug,
        "version": OPENCODE_VERSION,
        "projectID": project_id,
        "directory": target_dir_str,
        "title": title,
        "time": {
            "created": created_at,
            "updated": updated_at
        }
    });

    let mut oc_messages: Vec<(String, i64, Value)> = Vec::new();
    let mut oc_parts: Vec<(String, String, i64, Value)> = Vec::new();
    let mut last_user_msg_id: Option<String> = None;

    for event in &session.events {
        if let Some(segment) = compression::compressed_segment(event) {
            append_compressed_opencode_segment(
                session_id,
                event,
                segment,
                target_dir_str,
                &mut last_user_msg_id,
                &mut oc_messages,
                &mut oc_parts,
            );
            continue;
        }

        let Some(visible_role) = canonical_event_visible_message_role(event) else {
            continue;
        };
        if !canonical_event_is_visible_message(event) {
            continue;
        }
        let msg_id = generate_opencode_id("msg");
        let msg_created = event.timestamp.timestamp_millis();
        let (role, parent_id) = match visible_role {
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
            session_id,
            event,
            &msg_id,
            role,
            parent_id.as_deref(),
            target_dir_str,
        );
        oc_messages.push((msg_id.clone(), msg_created, msg_json));

        for block in &event.blocks {
            let part_id = generate_opencode_id("prt");
            let part_created = msg_created + 1;
            let Some(part_json) = canonical_block_to_opencode_part(
                session_id,
                &msg_id,
                &part_id,
                block,
                part_created,
            ) else {
                continue;
            };
            oc_parts.push((part_id, msg_id.clone(), part_created, part_json));
        }
    }

    OpenCodeProjection {
        session_json,
        messages: oc_messages,
        parts: oc_parts,
    }
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
) -> Option<Value> {
    match block {
        EventBlock::Text { text } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "text",
            "text": text,
        })),
        EventBlock::Thinking { text, .. } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "reasoning",
            "text": text,
        })),
        EventBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => Some(serde_json::json!({
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
        })),
        EventBlock::Image {
            mime_type, data, ..
        } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "file",
            "mime": mime_type,
            "filename": "image.png",
            "url": data.as_deref().unwrap_or(""),
        })),
        EventBlock::File { path, content, .. } => Some(serde_json::json!({
            "id": part_id,
            "sessionID": session_id,
            "messageID": msg_id,
            "type": "file",
            "mime": "text/plain",
            "filename": path,
            "url": content.as_deref().unwrap_or(""),
        })),
        _ => canonical_visible_block_text(block).map(|text| {
            serde_json::json!({
                "id": part_id,
                "sessionID": session_id,
                "messageID": msg_id,
                "type": "text",
                "text": text,
            })
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

fn opencode_db_session_source_locator(session_id: &str) -> String {
    format!("{}#session={session_id}", get_db_path().to_string_lossy())
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
            source_path: Some(opencode_db_session_source_locator(&session_id)),
        })
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }
    Ok(sessions)
}

fn scan_sessions_from_filesystem() -> Result<Vec<ProviderSessionSummary>> {
    let storage_dir = get_opencode_dir().join("storage").join("session");
    if !storage_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in WalkDir::new(storage_dir)
        .max_depth(3)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        if let Some(session) = parse_session_file(path) {
            sessions.push(session);
        }
    }
    sessions.sort_by(|left, right| {
        left.session_id
            .cmp(&right.session_id)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });
    Ok(sessions)
}

fn load_session_from_db(
    session_id: &str,
) -> Result<(
    Value,
    Vec<(Option<i64>, Value)>,
    HashMap<String, Vec<Value>>,
)> {
    let db_path = get_db_path();
    load_session_from_db_path(&db_path, session_id)
}

fn load_session_from_db_path(
    db_path: &Path,
    session_id: &str,
) -> Result<(
    Value,
    Vec<(Option<i64>, Value)>,
    HashMap<String, Vec<Value>>,
)> {
    let conn = Connection::open(db_path)?;

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
        "SELECT id, session_id, time_created, time_updated, data
         FROM message
         WHERE session_id = ?1
         ORDER BY time_created, id",
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
        Ok((Some(created), data))
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
        "SELECT id, message_id, session_id, time_created, time_updated, data
         FROM part
         WHERE session_id = ?1
         ORDER BY message_id, time_created, id",
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

fn load_session_from_filesystem_path(
    session_id: &str,
    session_path: &Path,
) -> Result<(
    Value,
    Vec<(Option<i64>, Value)>,
    HashMap<String, Vec<Value>>,
)> {
    let storage_dir = get_opencode_dir().join("storage");
    let session_json: Value = serde_json::from_reader(File::open(&session_path)?)?;

    // Load messages from filesystem
    let mut messages = Vec::new();
    let msg_dir = storage_dir.join("message").join(session_id);
    if msg_dir.exists() {
        let mut entries = std::fs::read_dir(&msg_dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let msg_json: Value = serde_json::from_reader(File::open(&path)?)?;
            let created = msg_json
                .get("time")
                .and_then(|v| v.get("created"))
                .and_then(|v| v.as_i64());
            messages.push((created, msg_json));
        }
    }

    // Load parts from filesystem
    let mut parts_map: HashMap<String, Vec<Value>> = HashMap::new();
    let parts_dir = storage_dir.join("part");
    if parts_dir.exists() {
        let mut entries = std::fs::read_dir(&parts_dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let msg_id = entry.file_name().to_string_lossy().to_string();
            let msg_parts_dir = entry.path();
            if !msg_parts_dir.is_dir() {
                continue;
            }
            let mut part_entries =
                std::fs::read_dir(&msg_parts_dir)?.collect::<std::io::Result<Vec<_>>>()?;
            part_entries.sort_by_key(|entry| entry.path());
            for part_entry in part_entries {
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

/// Paged variant of [`load_session_from_db_path`].
///
/// Messages are fetched with SQL `LIMIT ? OFFSET ?` so only the requested page
/// of message rows is read; parts are loaded only for the message ids on that
/// page. `event_count` is the total message count (`SELECT COUNT(*)`) and
/// `visible_message_count` is computed by reusing the canonical block mapper
/// over the full message list so it matches a full import exactly.
///
/// Returns `(session_json, page_messages, page_parts, total_messages, visible_messages)`.
fn load_session_page_from_db_path(
    db_path: &Path,
    session_id: &str,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<(
    Value,
    Vec<(Option<i64>, Value)>,
    HashMap<String, Vec<Value>>,
    usize,
    usize,
)> {
    let mut conn = Connection::open(db_path)?;
    let tx = conn.transaction()?;

    let session_json: Value = tx.query_row(
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

    // Total message count across the whole session.
    let total_messages: i64 = tx.query_row(
        "SELECT COUNT(*) FROM message WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;

    // Full message list (for accurate visible-message counting) is cheap to
    // read from SQL; the expensive part is event/block materialization, which
    // only happens for the requested page via imported_session_from_data.
    let mut full_messages: Vec<(Option<i64>, Value)> = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id, session_id, time_created, time_updated, data
             FROM message
             WHERE session_id = ?1
             ORDER BY time_created, id",
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
            Ok((Some(created), data))
        })?;
        for row in rows {
            if let Ok(r) = row {
                full_messages.push(r);
            }
        }
    }

    // Page slice.
    let page_messages: Vec<(Option<i64>, Value)> = full_messages
        .iter()
        .cloned()
        .skip(event_offset)
        .take(event_limit.unwrap_or(usize::MAX))
        .collect();
    let page_msg_ids: HashSet<String> = page_messages
        .iter()
        .filter_map(|(_, msg)| msg.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();

    // Parts only for the page messages.
    let mut page_parts: HashMap<String, Vec<Value>> = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id, message_id, session_id, time_created, time_updated, data
             FROM part
             WHERE session_id = ?1
             ORDER BY message_id, time_created, id",
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
                if page_msg_ids.contains(&msg_id) {
                    page_parts.entry(msg_id).or_default().push(part);
                }
            }
        }
    }

    // All parts, grouped by message id, for accurate visible counting.
    let mut full_parts: HashMap<String, Vec<Value>> = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT message_id, data
             FROM part
             WHERE session_id = ?1",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let message_id: String = row.get(0)?;
            let data_str: String = row.get(1)?;
            let data: Value = serde_json::from_str(&data_str).unwrap_or_default();
            Ok((message_id, data))
        })?;
        for row in rows {
            if let Ok((msg_id, part)) = row {
                full_parts.entry(msg_id).or_default().push(part);
            }
        }
    }

    let visible_messages = count_visible_opencode_messages(&full_messages, &full_parts);

    Ok((
        session_json,
        page_messages,
        page_parts,
        total_messages as usize,
        visible_messages,
    ))
}

/// Paged variant of [`load_session_from_filesystem_path`].
///
/// Message files are enumerated and sorted once; only the requested page is
/// parsed into JSON. Parts are loaded for the whole session (they are needed
/// for accurate visible-message counting and are cheap to read as small JSON
/// files), but only parts belonging to the page messages are returned for
/// event materialization.
fn load_session_page_from_filesystem_path(
    session_id: &str,
    session_path: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<(
    Value,
    Vec<(Option<i64>, Value)>,
    HashMap<String, Vec<Value>>,
    usize,
    usize,
)> {
    let storage_dir = get_opencode_dir().join("storage");
    let session_json: Value = serde_json::from_reader(File::open(&session_path)?)?;

    // Enumerate all message files in stable order, then parse only the page.
    let msg_dir = storage_dir.join("message").join(session_id);
    let mut all_msg_paths: Vec<PathBuf> = Vec::new();
    if msg_dir.exists() {
        let mut entries = std::fs::read_dir(&msg_dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                all_msg_paths.push(path);
            }
        }
    }
    let total_messages = all_msg_paths.len();

    let page_paths: Vec<&PathBuf> = all_msg_paths
        .iter()
        .skip(event_offset)
        .take(event_limit.unwrap_or(usize::MAX))
        .collect();

    let mut page_messages: Vec<(Option<i64>, Value)> = Vec::new();
    for path in &page_paths {
        let msg_json: Value = serde_json::from_reader(File::open(path)?)?;
        let created = msg_json
            .get("time")
            .and_then(|v| v.get("created"))
            .and_then(|v| v.as_i64());
        page_messages.push((created, msg_json));
    }

    // Load all parts (small JSON files) for accurate visible counting.
    let mut full_parts: HashMap<String, Vec<Value>> = HashMap::new();
    let parts_dir = storage_dir.join("part");
    if parts_dir.exists() {
        let mut entries = std::fs::read_dir(&parts_dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let msg_id = entry.file_name().to_string_lossy().to_string();
            let msg_parts_dir = entry.path();
            if !msg_parts_dir.is_dir() {
                continue;
            }
            let mut part_entries =
                std::fs::read_dir(&msg_parts_dir)?.collect::<std::io::Result<Vec<_>>>()?;
            part_entries.sort_by_key(|entry| entry.path());
            for part_entry in part_entries {
                let part_path = part_entry.path();
                if part_path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let part_json: Value = serde_json::from_reader(File::open(&part_path)?)?;
                full_parts
                    .entry(msg_id.clone())
                    .or_default()
                    .push(part_json);
            }
        }
    }

    // Build full message list metadata for counting without re-parsing page
    // files. We re-parse all message files here to get role/data for counting;
    // opencode filesystem sessions are small and this keeps counts identical to
    // a full import.
    let mut full_messages: Vec<(Option<i64>, Value)> = Vec::new();
    for path in &all_msg_paths {
        let msg_json: Value = serde_json::from_reader(File::open(path)?)?;
        let created = msg_json
            .get("time")
            .and_then(|v| v.get("created"))
            .and_then(|v| v.as_i64());
        full_messages.push((created, msg_json));
    }

    // Restrict page parts to the page messages.
    let page_msg_ids: HashSet<String> = page_messages
        .iter()
        .filter_map(|(_, msg)| msg.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut page_parts: HashMap<String, Vec<Value>> = HashMap::new();
    for (msg_id, parts) in &full_parts {
        if page_msg_ids.contains(msg_id) {
            page_parts.insert(msg_id.clone(), parts.clone());
        }
    }

    let visible_messages = count_visible_opencode_messages(&full_messages, &full_parts);

    Ok((
        session_json,
        page_messages,
        page_parts,
        total_messages,
        visible_messages,
    ))
}

/// Count messages that would be visible under the same rules as
/// [`canonical_event_is_visible_message`], reusing the canonical block mapper
/// so the result is identical to a full import. This is the single source of
/// truth for `ProviderSessionImportPage::message_count` for OpenCode.
fn count_visible_opencode_messages(
    messages: &[(Option<i64>, Value)],
    parts: &HashMap<String, Vec<Value>>,
) -> usize {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut artifacts = Vec::new();
    let mut count = 0usize;
    for (_, msg_json) in messages {
        let role_str = msg_json
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user");
        let role = match role_str {
            "user" => EventRole::User,
            "assistant" => EventRole::Assistant,
            _ => EventRole::Unknown,
        };
        if !matches!(
            role,
            EventRole::User | EventRole::Assistant | EventRole::Tool
        ) {
            continue;
        }
        let msg_id = msg_json
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let msg_parts: Vec<Value> = parts.get(&msg_id).cloned().unwrap_or_default();
        let mut blocks =
            canonical_blocks_from_parts(&msg_id, &msg_parts, &mut report, &mut artifacts);
        if blocks.is_empty() {
            blocks = vec![EventBlock::ProviderPayload {
                kind: "message_without_mappable_parts".to_string(),
                payload: msg_json.clone(),
            }];
        }
        let kind = derive_event_kind(&blocks);
        if matches!(
            kind,
            SessionEventKind::Lifecycle | SessionEventKind::Unknown
        ) {
            continue;
        }
        let visible_text: String = blocks
            .iter()
            .filter_map(|block| crate::provider::canonical_visible_block_text(block))
            .collect::<Vec<_>>()
            .join("\n");
        if !visible_text.trim().is_empty() {
            count += 1;
        }
    }
    count
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
        session_id,
        title,
        project_dir: directory,
        last_active_at: updated,
        source_path: Some(path.to_string_lossy().to_string()),
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

    insert_opencode_projection_rows(&tx, session_id, messages, parts)?;
    tx.commit()?;
    Ok(())
}

fn insert_opencode_projection_rows(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    messages: &[(String, i64, Value)],
    parts: &[(String, String, i64, Value)],
) -> Result<()> {
    for (msg_id, created, data) in messages {
        let mut db_data = data.clone();
        if let Value::Object(ref mut map) = db_data {
            map.remove("id");
            map.remove("sessionID");
        }
        let data_str = serde_json::to_string(&db_data)?;
        tx.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![msg_id, session_id, created, created, data_str],
        )?;
    }
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
            rusqlite::params![part_id, msg_id, session_id, created, created, data_str],
        )?;
    }
    Ok(())
}

fn write_opencode_json_atomic(path: &Path, value: &Value) -> Result<()> {
    let temp_path = path.with_extension(format!("json.memorph-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = File::create(&temp_path)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        use std::io::Write as _;
        file.flush()?;
        file.sync_all()?;
        std::fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp_path);
    }
    result
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
    write_opencode_json_atomic(&session_file, session_json)?;

    // Write messages
    let msg_dir = storage_dir.join("message").join(session_id);
    std::fs::create_dir_all(&msg_dir)?;
    for (msg_id, _created, data) in messages {
        let msg_file = msg_dir.join(format!("{}.json", msg_id));
        write_opencode_json_atomic(&msg_file, data)?;
    }

    // Write parts
    let parts_base = storage_dir.join("part");
    for (part_id, msg_id, _created, data) in parts {
        let part_dir = parts_base.join(msg_id);
        std::fs::create_dir_all(&part_dir)?;
        let part_file = part_dir.join(format!("{}.json", part_id));
        write_opencode_json_atomic(&part_file, data)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::session_management, storage::local_store};
    use chrono::TimeZone;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn write_multimessage_opencode_db(opencode_dir: &Path, session_id: &str) {
        std::fs::create_dir_all(opencode_dir).unwrap();
        let conn = Connection::open(opencode_dir.join("opencode.db")).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE project (
                id TEXT PRIMARY KEY,
                worktree TEXT NOT NULL
            );
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                parent_id TEXT,
                slug TEXT NOT NULL,
                directory TEXT NOT NULL,
                title TEXT NOT NULL,
                version TEXT NOT NULL,
                share_url TEXT,
                summary_additions INTEGER,
                summary_deletions INTEGER,
                summary_files INTEGER,
                summary_diffs TEXT,
                revert TEXT,
                permission TEXT,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                time_compacting INTEGER,
                time_archived INTEGER,
                workspace_id TEXT,
                path TEXT,
                agent TEXT,
                model TEXT,
                FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY (message_id) REFERENCES message(id) ON DELETE CASCADE
            );
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO project (id, worktree) VALUES ('p1', '/tmp/project')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (
                id, project_id, parent_id, slug, directory, title, version,
                share_url, summary_additions, summary_deletions, summary_files,
                summary_diffs, revert, permission, time_created, time_updated,
                time_compacting, time_archived, workspace_id, path, agent, model
             ) VALUES (
                ?1, 'p1', NULL, 's', '/tmp/project', 'Multi', '1.0',
                NULL, NULL, NULL, NULL, NULL, NULL, NULL,
                1700000000000, 1700000000500, NULL, NULL, NULL, NULL, NULL, NULL
             )",
            [session_id],
        )
        .unwrap();

        let messages = [
            ("msg-a", 1700000000010_i64, "user", "Build feature"),
            ("msg-b", 1700000000020, "assistant", "On it"),
            ("msg-c", 1700000000030, "user", "Thanks"),
        ];
        for (msg_id, created, role, text) in messages {
            let data = serde_json::json!({ "role": role }).to_string();
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, time_updated, data)
                 VALUES (?1, ?2, ?3, ?3, ?4)",
                rusqlite::params![msg_id, session_id, created, data],
            )
            .unwrap();
            let part_data = serde_json::json!({ "type": "text", "text": text }).to_string();
            conn.execute(
                "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                rusqlite::params![
                    format!("{msg_id}-p1"),
                    msg_id,
                    session_id,
                    created,
                    part_data
                ],
            )
            .unwrap();
        }
    }

    #[test]
    fn import_session_page_paginates_messages_and_keeps_full_counts() {
        assert_eq!(
            OpenCodeProvider.capabilities().page_strategy,
            PageStrategy::NativePage
        );
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        write_multimessage_opencode_db(opencode_dir.path(), "ses-paged");

        let locator = format!(
            "{}#session=ses-paged",
            opencode_dir.path().join("opencode.db").display()
        );

        // Full import baseline.
        let full_import = OpenCodeProvider.import_session(&locator).unwrap();
        assert_eq!(full_import.session.events.len(), 3);

        // Full page: counts match a full import, all events present.
        let full = import_opencode_session_page(&locator, 0, None).unwrap();
        assert_eq!(full.event_count, 3);
        assert_eq!(full.imported.session.events.len(), 3);
        let expected_visible = full_import
            .session
            .events
            .iter()
            .filter(|event| canonical_event_is_visible_message(event))
            .count();
        assert_eq!(full.message_count, expected_visible);
        assert_eq!(full.message_count, 3);
        assert_eq!(full.turn_count, Some(full.turns.len()));

        // Page with limit returns a strict subset but keeps total counts.
        let page1 = import_opencode_session_page(&locator, 0, Some(2)).unwrap();
        assert_eq!(page1.imported.session.events.len(), 2);
        assert_eq!(page1.event_count, 3);
        assert_eq!(page1.message_count, full.message_count);
        assert_eq!(page1.turn_count, None);
        assert_eq!(page1.imported.session.events[0].id, "msg-a");
        assert_eq!(page1.imported.session.events[1].id, "msg-b");

        // Second page starts at offset 2.
        let page2 = import_opencode_session_page(&locator, 2, Some(2)).unwrap();
        assert_eq!(page2.imported.session.events.len(), 1);
        assert_eq!(page2.event_count, 3);
        assert_eq!(page2.turn_count, None);
        assert_eq!(page2.imported.session.events[0].id, "msg-c");

        // Identity and title carry across pages.
        assert_eq!(page1.imported.session.identity.canonical_id, "ses-paged");
        assert_eq!(
            page1.imported.session.identity.source_title.as_deref(),
            Some("Multi")
        );
    }

    #[test]
    fn opencode_malformed_parts_are_preserved_and_reported() {
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let mut artifacts = Vec::new();
        let blocks = canonical_blocks_from_parts(
            "message-1",
            &[
                serde_json::json!({"type": "text"}),
                serde_json::json!({"type": "reasoning"}),
                serde_json::json!({
                    "type": "file",
                    "mime": "image/png",
                    "filename": "image.png",
                    "url": "data:image/png,not-valid"
                }),
                serde_json::json!({"type": "file", "filename": "missing.txt"}),
            ],
            &mut report,
            &mut artifacts,
        );

        assert_eq!(blocks.len(), 4);
        assert!(blocks
            .iter()
            .all(|block| matches!(block, EventBlock::ProviderPayload { .. })));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "opencode_text_part_missing_text"));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "opencode_reasoning_part_missing_text"));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "opencode_image_part_invalid_data_uri"));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "opencode_file_part_missing_url"));
    }

    #[test]
    fn opencode_message_without_parts_is_preserved_as_an_event() {
        let imported = imported_session_from_data(
            "session-1",
            (
                serde_json::json!({"id": "session-1", "title": "Empty message"}),
                vec![(
                    Some(1),
                    serde_json::json!({"id": "message-1", "role": "assistant"}),
                )],
                HashMap::new(),
            ),
        )
        .unwrap();

        assert!(matches!(
            imported.session.events[0].blocks.as_slice(),
            [EventBlock::ProviderPayload { kind, .. }]
                if kind == "message_without_mappable_parts"
        ));
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "opencode_message_without_mappable_parts"));
    }

    #[test]
    fn maps_opencode_error_finish_to_failed_boundary() {
        let mut parts = HashMap::new();
        parts.insert(
            "user-1".to_string(),
            vec![serde_json::json!({"type": "text", "text": "Build it"})],
        );
        parts.insert(
            "assistant-1".to_string(),
            vec![serde_json::json!({"type": "text", "text": "Failed"})],
        );
        let imported = imported_session_from_data(
            "session-1",
            (
                serde_json::json!({"id": "session-1", "title": "Build it"}),
                vec![
                    (Some(1), serde_json::json!({"id": "user-1", "role": "user"})),
                    (
                        Some(2),
                        serde_json::json!({
                            "id": "assistant-1",
                            "role": "assistant",
                            "finish": "error"
                        }),
                    ),
                ],
                parts,
            ),
        )
        .unwrap();

        let assistant = imported
            .session
            .events
            .iter()
            .find(|event| event.id == "assistant-1")
            .unwrap();
        assert_eq!(assistant.links.provider_turn_id, None);
        assert_eq!(assistant.links.turn_boundary, Some(TurnBoundary::Failed));
    }

    struct TestOpenCodeDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestOpenCodeDirGuard {
        fn drop(&mut self) {
            set_test_opencode_mutation_failure(None);
            set_test_opencode_dir(None);
        }
    }

    fn use_test_opencode_dir(path: PathBuf) -> TestOpenCodeDirGuard {
        let lock = lock_test_opencode_state();
        set_test_opencode_dir(Some(path));
        TestOpenCodeDirGuard { _lock: lock }
    }

    struct NativeOpenCodeFixture {
        session_path: PathBuf,
        message_path: PathBuf,
        part_path: PathBuf,
        orphan_part_path: PathBuf,
        original_session_bytes: Vec<u8>,
    }

    fn write_native_opencode_fixture(
        opencode_dir: &Path,
        session_id: &str,
    ) -> NativeOpenCodeFixture {
        std::fs::create_dir_all(opencode_dir).unwrap();
        let conn = Connection::open(opencode_dir.join("opencode.db")).unwrap();
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE project (
                id TEXT PRIMARY KEY,
                worktree TEXT NOT NULL
            );
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                parent_id TEXT,
                slug TEXT NOT NULL,
                directory TEXT NOT NULL,
                title TEXT NOT NULL,
                version TEXT NOT NULL,
                share_url TEXT,
                summary_additions INTEGER,
                summary_deletions INTEGER,
                summary_files INTEGER,
                summary_diffs TEXT,
                revert TEXT,
                permission TEXT,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                time_compacting INTEGER,
                time_archived INTEGER,
                workspace_id TEXT,
                path TEXT,
                agent TEXT,
                model TEXT,
                FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY (message_id) REFERENCES message(id) ON DELETE CASCADE
            );
            CREATE TABLE todo (
                session_id TEXT NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL,
                priority TEXT NOT NULL,
                position INTEGER NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                PRIMARY KEY (session_id, position),
                FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
            );
            CREATE TABLE session_share (
                session_id TEXT PRIMARY KEY,
                id TEXT NOT NULL,
                secret TEXT NOT NULL,
                url TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
            );
            CREATE TABLE session_message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                type TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
            );
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO project (id, worktree) VALUES (?1, ?2)",
            ["project-1", "/tmp/project"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (
                id, project_id, parent_id, slug, directory, title, version,
                share_url, summary_additions, summary_deletions, summary_files,
                summary_diffs, revert, permission, time_created, time_updated,
                time_compacting, time_archived, workspace_id, path, agent, model
             ) VALUES (
                ?1, 'project-1', NULL, 'before-slug', '/tmp/project', 'Before',
                '1.14.39', 'https://share.test', 3, 4, 5, 'diffs', 'revert',
                'permission', 1700000000000, 1700000000100, NULL, NULL,
                'workspace-1', '/tmp/project', 'build', 'gpt-5.4'
             )",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (
                id, session_id, time_created, time_updated, data
             ) VALUES (
                'msg-1', ?1, 1700000000010, 1700000000011, '{\"role\":\"user\"}'
             )",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part (
                id, message_id, session_id, time_created, time_updated, data
             ) VALUES (
                'part-1', 'msg-1', ?1, 1700000000020, 1700000000021,
                '{\"type\":\"text\",\"text\":\"original\"}'
             )",
            [session_id],
        )
        .unwrap();
        for (position, content) in [(0_i64, "first"), (1_i64, "second")] {
            conn.execute(
                "INSERT INTO todo (
                    session_id, content, status, priority, position,
                    time_created, time_updated
                 ) VALUES (?1, ?2, 'pending', 'high', ?3, 1700000000030, 1700000000031)",
                rusqlite::params![session_id, content, position],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO session_share (
                session_id, id, secret, url, time_created, time_updated
             ) VALUES (?1, 'share-1', 'secret-1', 'https://share.test/1',
                       1700000000040, 1700000000041)",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_message (
                id, session_id, type, time_created, time_updated, data
             ) VALUES ('session-message-1', ?1, 'summary',
                       1700000000050, 1700000000051, '{\"summary\":\"exact\"}')",
            [session_id],
        )
        .unwrap();

        let session_dir = opencode_dir
            .join("storage")
            .join("session")
            .join("project-1");
        let message_dir = opencode_dir
            .join("storage")
            .join("message")
            .join(session_id);
        let part_dir = opencode_dir.join("storage").join("part").join("msg-1");
        let orphan_part_dir = opencode_dir.join("storage").join("part").join("msg-orphan");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::create_dir_all(&message_dir).unwrap();
        std::fs::create_dir_all(&part_dir).unwrap();
        std::fs::create_dir_all(&orphan_part_dir).unwrap();

        let original_session_bytes = format!(
            "{{\n  \"id\": \"{session_id}\",\n  \"projectID\": \"project-1\",\n  \"directory\": \"/tmp/project\",\n  \"title\": \"Before\",\n  \"time\": {{\"created\": 1700000000000, \"updated\": 1700000000100}}\n}}\n"
        )
        .into_bytes();
        let session_path = session_dir.join(format!("{session_id}.json"));
        let message_path = message_dir.join("msg-1.json");
        let part_path = part_dir.join("part-1.json");
        let orphan_part_path = orphan_part_dir.join("part-orphan.json");
        std::fs::write(&session_path, &original_session_bytes).unwrap();
        std::fs::write(
            &message_path,
            format!("{{\"id\":\"msg-1\",\"sessionID\":\"{session_id}\",\"role\":\"user\"}}\n"),
        )
        .unwrap();
        std::fs::write(
            &part_path,
            format!(
                "{{\"id\":\"part-1\",\"messageID\":\"msg-1\",\"sessionID\":\"{session_id}\",\"type\":\"text\",\"text\":\"original\"}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            &orphan_part_path,
            format!(
                "{{\"id\":\"part-orphan\",\"messageID\":\"msg-orphan\",\"sessionID\":\"{session_id}\",\"type\":\"text\",\"text\":\"orphan\"}}\n"
            ),
        )
        .unwrap();

        NativeOpenCodeFixture {
            session_path,
            message_path,
            part_path,
            orphan_part_path,
            original_session_bytes,
        }
    }

    #[test]
    fn scan_sessions_uses_fingerprintable_database_source_locator() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        write_native_opencode_fixture(opencode_dir.path(), "ses_locator");

        let sessions = OpenCodeProvider.scan_sessions().unwrap();
        let session = sessions
            .iter()
            .find(|session| session.session_id == "ses_locator")
            .unwrap();

        assert_eq!(
            session.source_path.as_deref(),
            Some(
                format!(
                    "{}#session=ses_locator",
                    opencode_dir.path().join("opencode.db").to_string_lossy()
                )
                .as_str()
            )
        );
        assert!(OpenCodeProvider
            .session_source_fingerprint(session.source_path.as_deref().unwrap())
            .unwrap()
            .is_some());
        let imported = OpenCodeProvider
            .import_session(session.source_path.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            imported.session.provenance.primary_source.session_id,
            "ses_locator"
        );
        assert_eq!(
            imported.session.provenance.primary_source.source_path,
            session.source_path
        );
    }

    #[test]
    fn scan_sessions_discovers_filesystem_only_source_plane() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let fixture = write_native_opencode_fixture(opencode_dir.path(), "ses_filesystem_only");
        std::fs::remove_file(opencode_dir.path().join("opencode.db")).unwrap();

        let sessions = OpenCodeProvider.scan_sessions().unwrap();
        let session = sessions
            .iter()
            .find(|session| session.session_id == "ses_filesystem_only")
            .unwrap();

        assert_eq!(
            session.source_path.as_deref(),
            Some(fixture.session_path.to_string_lossy().as_ref())
        );
        let meta = OpenCodeProvider
            .get_session_meta("ses_filesystem_only")
            .unwrap()
            .unwrap();
        assert_eq!(meta.session_id, session.session_id);
        assert_eq!(meta.source_path, session.source_path);
    }

    #[test]
    fn scan_sessions_reports_corrupt_database() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        std::fs::write(opencode_dir.path().join("opencode.db"), b"not sqlite").unwrap();

        let error = OpenCodeProvider.scan_sessions().unwrap_err();

        assert!(error.to_string().contains("file is not a database"));
    }

    #[test]
    fn import_session_reads_the_explicit_source_plane() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let fixture = write_native_opencode_fixture(opencode_dir.path(), "ses_source_plane");
        std::fs::write(
            &fixture.session_path,
            serde_json::json!({
                "id": "ses_source_plane",
                "projectID": "project-1",
                "directory": "/tmp/project",
                "title": "Filesystem title",
                "time": {"created": 1700000000000_i64, "updated": 1700000000100_i64}
            })
            .to_string(),
        )
        .unwrap();

        let from_database = OpenCodeProvider
            .import_session(&opencode_db_session_source_locator("ses_source_plane"))
            .unwrap();
        let from_filesystem = OpenCodeProvider
            .import_session(fixture.session_path.to_string_lossy().as_ref())
            .unwrap();

        assert_eq!(
            from_database.session.identity.source_title.as_deref(),
            Some("Before")
        );
        assert_eq!(
            from_filesystem.session.identity.source_title.as_deref(),
            Some("Filesystem title")
        );
    }

    #[test]
    fn import_session_uses_database_path_from_locator() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        write_native_opencode_fixture(opencode_dir.path(), "ses_database_path");
        let default_database = opencode_dir.path().join("opencode.db");
        let alternate_database = opencode_dir.path().join("alternate.db");
        std::fs::rename(&default_database, &alternate_database).unwrap();
        std::fs::write(&default_database, b"not sqlite").unwrap();
        let locator = format!(
            "{}#session=ses_database_path",
            alternate_database.to_string_lossy()
        );

        let imported = OpenCodeProvider.import_session(&locator).unwrap();

        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Before")
        );
        assert_eq!(
            imported
                .session
                .provenance
                .primary_source
                .source_path
                .as_deref(),
            Some(locator.as_str())
        );
    }

    #[test]
    fn parse_session_file_keeps_actual_json_source_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ses_file.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "id": "ses_file",
                "directory": "/tmp/project",
                "title": "File session",
                "time": {"updated": 1_790_000_000_000_i64}
            })
            .to_string(),
        )
        .unwrap();

        let session = parse_session_file(&path).unwrap();

        assert_eq!(session.session_id, "ses_file");
        assert_eq!(
            session.source_path.as_deref(),
            Some(path.to_string_lossy().as_ref())
        );
    }

    fn session_owned_row_counts(opencode_dir: &Path, session_id: &str) -> Vec<i64> {
        let conn = Connection::open(opencode_dir.join("opencode.db")).unwrap();
        [
            "session",
            "message",
            "part",
            "todo",
            "session_share",
            "session_message",
        ]
        .into_iter()
        .map(|table| {
            let column = if table == "session" {
                "id"
            } else {
                "session_id"
            };
            conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?1"),
                [session_id],
                |row| row.get(0),
            )
            .unwrap()
        })
        .collect()
    }

    #[test]
    fn delete_backup_restores_exact_opencode_database_and_filesystem() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-delete-backup";
        let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
        let backup_root = opencode_dir.path().join("backups");
        let binary_payload = vec![0_u8, 1, 2, 127, 128, 255];
        let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
        conn.execute(
            "UPDATE session_message SET data = ?1 WHERE id = 'session-message-1'",
            [binary_payload.as_slice()],
        )
        .unwrap();
        drop(conn);

        let backup = create_opencode_session_backup(
            ProviderSourceMutation::Delete,
            "operation-delete-1",
            session_id,
            &backup_root,
        )
        .unwrap();
        let backup_conn =
            Connection::open(backup.backup_path.join(OPENCODE_BACKUP_DB_PATH)).unwrap();
        let backed_up_payload: Vec<u8> = backup_conn
            .query_row(
                "SELECT data FROM session_message WHERE id = 'session-message-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(backed_up_payload, binary_payload);
        drop(backup_conn);
        delete_opencode_session(session_id).unwrap();

        assert_eq!(
            session_owned_row_counts(opencode_dir.path(), session_id),
            vec![0, 0, 0, 0, 0, 0]
        );
        assert!(!fixture.session_path.exists());
        assert!(!fixture.message_path.exists());
        assert!(!fixture.part_path.exists());
        assert!(!fixture.orphan_part_path.exists());

        restore_opencode_session_backup(&backup).unwrap();

        assert_eq!(
            session_owned_row_counts(opencode_dir.path(), session_id),
            vec![1, 1, 1, 2, 1, 1]
        );
        assert_eq!(
            std::fs::read(&fixture.session_path).unwrap(),
            fixture.original_session_bytes
        );
        assert!(fixture.message_path.exists());
        assert!(fixture.part_path.exists());
        assert!(fixture.orphan_part_path.exists());
        let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
        let restored_payload: Vec<u8> = conn
            .query_row(
                "SELECT data FROM session_message WHERE id = 'session-message-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(restored_payload, binary_payload);

        let metadata: OpenCodeSessionBackupMetadata = serde_json::from_slice(
            &std::fs::read(backup.backup_path.join("metadata.json")).unwrap(),
        )
        .unwrap();
        let tables = metadata
            .sqlite_tables
            .iter()
            .map(|table| table.table.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(
            tables,
            HashSet::from([
                "session",
                "message",
                "part",
                "todo",
                "session_share",
                "session_message",
            ])
        );
        let row_counts = metadata
            .sqlite_tables
            .iter()
            .map(|table| (table.table.as_str(), table.row_count))
            .collect::<HashMap<_, _>>();
        assert_eq!(row_counts["session"], 1);
        assert_eq!(row_counts["message"], 1);
        assert_eq!(row_counts["part"], 1);
        assert_eq!(row_counts["todo"], 2);
        assert_eq!(row_counts["session_share"], 1);
        assert_eq!(row_counts["session_message"], 1);
    }

    #[test]
    fn native_replace_preserves_opencode_identity_and_session_rows() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-native-replace";
        let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
        let source = opencode_db_session_source_locator(session_id);
        let mut session = import_canonical_session_from_source(session_id, &source)
            .unwrap()
            .session;
        session.events.clear();

        OpenCodeProvider
            .replace_session(session_id, &session)
            .unwrap();

        assert_eq!(
            import_canonical_session_from_source(session_id, &source)
                .unwrap()
                .session
                .identity
                .canonical_id,
            session_id
        );
        assert_eq!(
            session_owned_row_counts(opencode_dir.path(), session_id),
            vec![1, 0, 0, 2, 1, 1]
        );
        assert!(fixture.session_path.exists());
        assert!(!fixture.message_path.exists());
        assert!(!fixture.part_path.exists());
        assert!(!fixture.orphan_part_path.exists());
    }

    #[test]
    fn replace_failure_can_restore_exact_opencode_source() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-replace-rollback";
        let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
        let source = opencode_db_session_source_locator(session_id);
        let mut session = import_canonical_session_from_source(session_id, &source)
            .unwrap()
            .session;
        session.events.clear();
        let backup = create_opencode_session_backup(
            ProviderSourceMutation::Replace,
            "operation-replace-rollback",
            session_id,
            &opencode_dir.path().join("backups"),
        )
        .unwrap();

        set_test_opencode_mutation_failure(Some(ProviderSourceMutation::Replace));
        assert!(OpenCodeProvider
            .replace_session(session_id, &session)
            .is_err());
        restore_opencode_session_backup(&backup).unwrap();

        assert_eq!(
            session_owned_row_counts(opencode_dir.path(), session_id),
            vec![1, 1, 1, 2, 1, 1]
        );
        assert_eq!(
            std::fs::read(&fixture.session_path).unwrap(),
            fixture.original_session_bytes
        );
        assert!(fixture.message_path.exists());
        assert!(fixture.part_path.exists());
        assert!(fixture.orphan_part_path.exists());
    }

    #[test]
    fn replace_backup_restores_exact_opencode_source() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-replace-backup";
        let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
        let backup = create_opencode_session_backup(
            ProviderSourceMutation::Replace,
            "operation-replace-1",
            session_id,
            &opencode_dir.path().join("backups"),
        )
        .unwrap();

        delete_opencode_session(session_id).unwrap();
        restore_opencode_session_backup(&backup).unwrap();

        assert_eq!(
            session_owned_row_counts(opencode_dir.path(), session_id),
            vec![1, 1, 1, 2, 1, 1]
        );
        assert_eq!(
            std::fs::read(&fixture.session_path).unwrap(),
            fixture.original_session_bytes
        );
        assert!(fixture.message_path.exists());
        assert!(fixture.part_path.exists());
        assert!(fixture.orphan_part_path.exists());
    }

    #[test]
    fn rename_backup_restores_only_opencode_session_owned_resources() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-rename-backup";
        let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
        let backup_root = opencode_dir.path().join("backups");

        let backup = create_opencode_session_backup(
            ProviderSourceMutation::Rename,
            "operation-rename-1",
            session_id,
            &backup_root,
        )
        .unwrap();
        rename_opencode_session(session_id, "After").unwrap();

        let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
        conn.execute(
            "UPDATE message SET data = '{\"role\":\"user\",\"changed\":true}' WHERE id = 'msg-1'",
            [],
        )
        .unwrap();
        drop(conn);
        std::fs::write(&fixture.message_path, b"changed message state").unwrap();
        std::fs::write(&fixture.part_path, b"changed part state").unwrap();

        restore_opencode_session_backup(&backup).unwrap();

        let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
        let session: (String, i64) = conn
            .query_row(
                "SELECT title, time_updated FROM session WHERE id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let message_data: String = conn
            .query_row("SELECT data FROM message WHERE id = 'msg-1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(session, ("Before".to_string(), 1_700_000_000_100));
        assert_eq!(message_data, "{\"role\":\"user\",\"changed\":true}");
        assert_eq!(
            std::fs::read(&fixture.session_path).unwrap(),
            fixture.original_session_bytes
        );
        assert_eq!(
            std::fs::read(&fixture.message_path).unwrap(),
            b"changed message state"
        );
        assert_eq!(
            std::fs::read(&fixture.part_path).unwrap(),
            b"changed part state"
        );

        let metadata: OpenCodeSessionBackupMetadata = serde_json::from_slice(
            &std::fs::read(backup.backup_path.join("metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata.sqlite_tables.len(), 1);
        assert_eq!(metadata.sqlite_tables[0].table, "session");
        assert_eq!(metadata.filesystem_entries.len(), 1);
    }

    #[test]
    fn opencode_backup_contract_and_capabilities_are_truthful() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-backup-contract";
        write_native_opencode_fixture(opencode_dir.path(), session_id);
        let backup = create_opencode_session_backup(
            ProviderSourceMutation::Delete,
            "operation-contract-1",
            session_id,
            &opencode_dir.path().join("backups"),
        )
        .unwrap();

        let capabilities = OpenCodeProvider.capabilities();
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert_eq!(backup.mutation, ProviderSourceMutation::Delete);
        assert_eq!(backup.operation_id, "operation-contract-1");
        assert_eq!(backup.provider_session_id, session_id);
        assert_eq!(
            backup.source_path,
            opencode_dir.path().canonicalize().unwrap()
        );
        assert_eq!(backup.format, OPENCODE_BACKUP_FORMAT);
        assert_eq!(backup.mime_type, OPENCODE_BACKUP_MIME);
        assert_eq!(
            backup
                .restore_metadata
                .get("restore_mode")
                .and_then(Value::as_str),
            Some("opencode_session_restore")
        );
    }

    #[test]
    fn delete_backup_rejects_non_cascade_session_relationships_before_write() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-non-cascade";
        write_native_opencode_fixture(opencode_dir.path(), session_id);
        let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE retained_session_reference (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE SET NULL
            );
            INSERT INTO retained_session_reference (id, session_id)
            VALUES ('reference-1', 'ses-non-cascade');
            ",
        )
        .unwrap();
        drop(conn);

        let error = create_opencode_session_backup(
            ProviderSourceMutation::Delete,
            "operation-non-cascade",
            session_id,
            &opencode_dir.path().join("backups"),
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("unsupported ON DELETE SET NULL behavior"));
        assert_eq!(
            session_owned_row_counts(opencode_dir.path(), session_id),
            vec![1, 1, 1, 2, 1, 1]
        );
    }

    #[test]
    fn backup_registration_failure_prevents_opencode_provider_write() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-registration-failure";
        let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
        let backup_root = opencode_dir.path().join("backups");
        let mut artifact_conn = Connection::open_in_memory().unwrap();

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-registration-failure".to_string()],
            &backup_root,
            &mut artifact_conn,
        );

        let error = results.into_iter().next().unwrap().unwrap_err();
        assert!(error
            .to_string()
            .contains("Delete cancelled before provider write"));
        assert_eq!(
            session_owned_row_counts(opencode_dir.path(), session_id),
            vec![1, 1, 1, 2, 1, 1]
        );
        assert_eq!(
            std::fs::read(&fixture.session_path).unwrap(),
            fixture.original_session_bytes
        );
        assert!(backup_root
            .join(PROVIDER_ID)
            .join("operation-registration-failure")
            .exists());
    }

    #[test]
    fn partial_opencode_delete_failure_restores_registered_backup() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-partial-delete";
        let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
        let backup_root = opencode_dir.path().join("backups");
        let mut artifact_conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();
        set_test_opencode_mutation_failure(Some(ProviderSourceMutation::Delete));

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-partial-delete".to_string()],
            &backup_root,
            &mut artifact_conn,
        );

        let error = results.into_iter().next().unwrap().unwrap_err();
        assert!(error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            session_owned_row_counts(opencode_dir.path(), session_id),
            vec![1, 1, 1, 2, 1, 1]
        );
        assert_eq!(
            std::fs::read(&fixture.session_path).unwrap(),
            fixture.original_session_bytes
        );
        assert!(fixture.message_path.exists());
        assert!(fixture.part_path.exists());
        assert!(fixture.orphan_part_path.exists());
    }

    #[test]
    fn database_import_uses_stable_message_and_part_order() {
        let opencode_dir = tempdir().unwrap();
        let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
        let session_id = "ses-stable-order";
        write_native_opencode_fixture(opencode_dir.path(), session_id);
        let db_path = opencode_dir.path().join("opencode.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute("DELETE FROM part WHERE session_id = ?1", [session_id])
            .unwrap();
        conn.execute("DELETE FROM message WHERE session_id = ?1", [session_id])
            .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data)
             VALUES ('msg-b', ?1, 1700000000010, 1700000000011,
                     '{\"role\":\"assistant\"}')",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data)
             VALUES ('msg-a', ?1, 1700000000010, 1700000000011,
                     '{\"role\":\"user\",\"time\":{\"created\":9223372036854775807}}')",
            [session_id],
        )
        .unwrap();
        for (part_id, message_id, text) in [
            ("part-z", "msg-a", "second block"),
            ("part-a", "msg-a", "first block"),
            ("part-b", "msg-b", "assistant block"),
        ] {
            conn.execute(
                "INSERT INTO part (
                    id, message_id, session_id, time_created, time_updated, data
                 ) VALUES (?1, ?2, ?3, 1700000000020, 1700000000021, ?4)",
                rusqlite::params![
                    part_id,
                    message_id,
                    session_id,
                    serde_json::json!({ "type": "text", "text": text }).to_string()
                ],
            )
            .unwrap();
        }
        drop(conn);

        let first = imported_session_from_data(
            session_id,
            load_session_from_db_path(&db_path, session_id).unwrap(),
        )
        .unwrap();
        let second = imported_session_from_data(
            session_id,
            load_session_from_db_path(&db_path, session_id).unwrap(),
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(&first.session.events).unwrap(),
            serde_json::to_value(&second.session.events).unwrap()
        );
        assert_eq!(
            first
                .session
                .events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["msg-a", "msg-b"]
        );
        assert_eq!(
            first.session.events[0].timestamp.timestamp_millis(),
            1_700_000_000_010
        );
        assert_eq!(
            first.session.events[1].timestamp.timestamp_millis(),
            1_700_000_000_010
        );
        assert!(matches!(
            first.session.events[0].blocks.as_slice(),
            [EventBlock::Text { text: first }, EventBlock::Text { text: second }]
                if first == "first block" && second == "second block"
        ));
    }

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
    fn provider_payload_block_is_skipped_in_opencode_part_export() {
        let block = EventBlock::ProviderPayload {
            kind: "internal".to_string(),
            payload: serde_json::json!({"id": "hidden"}),
        };

        assert!(canonical_block_to_opencode_part(
            "ses_test",
            "msg_test",
            "prt_test",
            &block,
            1_700_000_000_001,
        )
        .is_none());
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
        let segment =
            compression::compressed_segment(&event).expect("canonical compressed segment");

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
            Some(
                "Retrieve specific details with: memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"
            )
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
