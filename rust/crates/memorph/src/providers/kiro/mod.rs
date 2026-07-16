pub mod adapter;
pub mod hook;

use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ImportedSession, MappingDirection, MappingDisposition, MappingIssue,
    MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext, SessionEvent,
    SessionEventKind, SessionIdentity, SessionProvenance, TurnBoundary,
};
use crate::provider::{
    canonical_event_is_visible_message, PageStrategy, Provider, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionImportPage, ProviderSessionSummary,
    ProviderSourceFingerprint, ScanStrategy, StorageShape, TurnQuality,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct KiroProvider;

const PROVIDER_ID: &str = "kiro";
const CURRENT_SCHEMA_VERSION: &str = "1.0.0";
const CURRENT_DATA_MODEL_VERSION: u64 = 1;

#[cfg(test)]
static TEST_KIRO_SESSIONS_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

impl Provider for KiroProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Kiro"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            import: true,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Directory,
            turn_quality: TurnQuality::Exact,
            import_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Preserved),
                tool_call: Some(MappingDisposition::Preserved),
                tool_result: Some(MappingDisposition::Preserved),
                patch: Some(MappingDisposition::Unsupported),
                image: Some(MappingDisposition::Unsupported),
                file: Some(MappingDisposition::Unsupported),
                compressed: Some(MappingDisposition::Unsupported),
                provider_payload: Some(MappingDisposition::Preserved),
            },
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let sessions_root = kiro_sessions_dir()?;
        if !sessions_root.exists() {
            return Ok(Vec::new());
        }
        scan_sessions_in(&sessions_root)
    }

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        let Some(session_dir) = find_session_dir(session_id)? else {
            return Ok(None);
        };
        session_summary_from_dir(&session_dir).map(Some)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        import_canonical_session_from_dir(Path::new(source_path))
    }

    fn import_session_page(
        &self,
        source_path: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<ProviderSessionImportPage> {
        import_kiro_session_page(Path::new(source_path), event_offset, event_limit)
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        kiro_session_source_fingerprint(Path::new(source_path))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let Some(session_dir) = find_session_dir(session_id)? else {
            return Ok(0);
        };
        let mut total = 0_u64;
        for entry in WalkDir::new(&session_dir).follow_links(false) {
            let entry = entry.with_context(|| {
                format!("Failed to walk Kiro session: {}", session_dir.display())
            })?;
            if entry.file_type().is_file() {
                total = total.saturating_add(entry.metadata()?.len());
            }
        }
        Ok(total)
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        kiro_sessions_dir().ok().into_iter().collect()
    }
}

#[derive(Debug, Deserialize)]
struct KiroSessionMetadata {
    #[serde(rename = "schemaVersion")]
    schema_version: String,
    #[serde(rename = "dataModelVersion")]
    data_model_version: u64,
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(rename = "workspacePaths", default)]
    workspace_paths: Vec<String>,
    #[serde(rename = "createdAt", default)]
    created_at: Option<String>,
    #[serde(rename = "lastModifiedAt", default)]
    last_modified_at: Option<String>,
}

fn kiro_sessions_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_KIRO_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        return Ok(path);
    }

    let home = dirs::home_dir().context("Unable to locate user home directory")?;
    Ok(home.join(".kiro").join("sessions"))
}

fn scan_sessions_in(sessions_root: &Path) -> Result<Vec<ProviderSessionSummary>> {
    let mut seen_session_ids = BTreeMap::new();
    let mut sessions = Vec::new();

    for bucket_dir in sorted_child_directories(sessions_root)? {
        for session_dir in sorted_child_directories(&bucket_dir)? {
            if !has_current_source_files(&session_dir)? {
                continue;
            }
            let summary = session_summary_from_dir(&session_dir)?;
            if let Some(previous) =
                seen_session_ids.insert(summary.session_id.clone(), session_dir.to_path_buf())
            {
                anyhow::bail!(
                    "Ambiguous Kiro session id {}: {} and {}",
                    summary.session_id,
                    previous.display(),
                    session_dir.display()
                );
            }
            sessions.push(summary);
        }
    }

    sessions.sort_by(|left, right| {
        right
            .last_active_at
            .cmp(&left.last_active_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(sessions)
}

fn sorted_child_directories(parent: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("Failed to read Kiro source directory: {}", parent.display())
            })
        }
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry
            .with_context(|| format!("Failed to read Kiro source entry: {}", parent.display()))?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

fn has_current_source_files(session_dir: &Path) -> Result<bool> {
    Ok(required_regular_file(&session_dir.join("session.json"))?
        && required_regular_file(&session_dir.join("messages.jsonl"))?)
}

fn required_regular_file(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Ok(true)
        }
        Ok(_) => anyhow::bail!("Kiro source is not a regular file: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to inspect Kiro source: {}", path.display()))
        }
    }
}

fn find_session_dirs(session_id: &str) -> Result<Vec<PathBuf>> {
    validate_session_id(session_id)?;
    let sessions_root = kiro_sessions_dir()?;
    if !sessions_root.exists() {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    for bucket_dir in sorted_child_directories(&sessions_root)? {
        let session_dir = bucket_dir.join(session_id);
        let metadata = match std::fs::symlink_metadata(&session_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to inspect Kiro session: {}", session_dir.display())
                })
            }
        };
        if metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && has_current_source_files(&session_dir)?
        {
            matches.push(session_dir);
        }
    }
    matches.sort();
    Ok(matches)
}

fn validate_session_id(session_id: &str) -> Result<()> {
    let mut components = Path::new(session_id).components();
    let is_single_normal_component = matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    );
    if !is_single_normal_component {
        anyhow::bail!("Invalid Kiro session id: {session_id}");
    }
    Ok(())
}

fn find_session_dir(session_id: &str) -> Result<Option<PathBuf>> {
    let matches = find_session_dirs(session_id)?;
    match matches.as_slice() {
        [] => Ok(None),
        [session_dir] => Ok(Some(session_dir.clone())),
        _ => anyhow::bail!("Kiro session id is ambiguous: {session_id}"),
    }
}

fn session_summary_from_dir(session_dir: &Path) -> Result<ProviderSessionSummary> {
    let metadata = read_validated_session_metadata(session_dir)?;
    let title = metadata
        .title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    let project_dir = metadata.workspace_paths.first().cloned();
    let last_active_at = metadata
        .last_modified_at
        .as_deref()
        .and_then(parse_timestamp_ms)
        .or_else(|| metadata.created_at.as_deref().and_then(parse_timestamp_ms))
        .or_else(|| source_file_modified_ms(&session_dir.join("messages.jsonl")));

    Ok(ProviderSessionSummary {
        session_id: metadata.id,
        title,
        project_dir,
        last_active_at,
        source_path: Some(session_dir.to_string_lossy().to_string()),
    })
}

fn read_validated_session_metadata(session_dir: &Path) -> Result<KiroSessionMetadata> {
    let session_id = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|id| !id.is_empty())
        .context("Kiro session directory has no valid session id")?;
    let bucket = session_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .context("Kiro session directory has no workspace bucket")?;
    let metadata_path = session_dir.join("session.json");
    let raw = std::fs::read_to_string(&metadata_path)
        .with_context(|| format!("Failed to read Kiro metadata: {}", metadata_path.display()))?;
    let metadata: KiroSessionMetadata = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse Kiro metadata: {}", metadata_path.display()))?;

    if metadata.schema_version != CURRENT_SCHEMA_VERSION
        || metadata.data_model_version != CURRENT_DATA_MODEL_VERSION
    {
        anyhow::bail!(
            "Unsupported Kiro session schema {}/{}: {}",
            metadata.schema_version,
            metadata.data_model_version,
            metadata_path.display()
        );
    }
    if metadata.id != session_id {
        anyhow::bail!(
            "Kiro metadata id {} does not match session directory {}",
            metadata.id,
            session_id
        );
    }
    let expected_bucket = workspace_bucket(&metadata.workspace_paths)?;
    if expected_bucket != bucket {
        anyhow::bail!(
            "Kiro workspace bucket {} does not match metadata workspacePaths (expected {})",
            bucket,
            expected_bucket
        );
    }
    Ok(metadata)
}

fn workspace_bucket(workspace_paths: &[String]) -> Result<String> {
    if workspace_paths.is_empty() {
        return Ok("_global".to_string());
    }
    let mut normalized = workspace_paths
        .iter()
        .map(|path| normalize_workspace_path(path))
        .collect::<Result<Vec<_>>>()?;
    normalized.sort();
    let joined = normalized.join("\0");
    let digest = format!("{:x}", Sha256::digest(joined.as_bytes()));
    Ok(digest[..16].to_string())
}

fn normalize_workspace_path(path: &str) -> Result<String> {
    if path.is_empty() {
        anyhow::bail!("Kiro workspace path must not be empty");
    }
    if !Path::new(path).is_absolute() {
        anyhow::bail!("Kiro workspace path must be absolute: {path}");
    }
    let normalized = path.replace('\\', "/");
    #[cfg(target_os = "windows")]
    let normalized = normalized.to_lowercase();
    Ok(normalized)
}

fn parse_timestamp_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn source_file_modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
}

struct KiroImportedEvent {
    event: SessionEvent,
    source_file: String,
    source_line: usize,
}

#[derive(Default)]
struct KiroStreamImport {
    events: Vec<KiroImportedEvent>,
    sub_execution_parents: BTreeMap<String, String>,
}

struct KiroStreamState {
    default_turn_id: Option<String>,
    model_id: Option<String>,
    timestamp_base: DateTime<Utc>,
    active_turn_id: Option<String>,
    tool_call_events: BTreeMap<String, String>,
}

fn import_kiro_session_page(
    session_dir: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let mut imported = import_canonical_session_from_dir(session_dir)?;
    let full_events = imported.session.events.clone();
    let event_count = full_events.len();
    let message_count = full_events
        .iter()
        .filter(|event| canonical_event_is_visible_message(event))
        .count();
    let full_turns = crate::session_projection::project_session_turns(
        &imported.session.identity.canonical_id,
        &full_events,
        TurnQuality::Exact,
    );
    let offset = event_offset.min(event_count);
    imported.session.events = match event_limit {
        Some(limit) => full_events
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect(),
        None => full_events.iter().skip(offset).cloned().collect(),
    };

    let mut turns = crate::session_projection::project_session_turns(
        &imported.session.identity.canonical_id,
        &imported.session.events,
        TurnQuality::Inferred,
    );
    let full_event_counts =
        full_events
            .iter()
            .fold(BTreeMap::<String, usize>::new(), |mut counts, event| {
                if let Some(turn_id) = event.links.provider_turn_id.as_ref() {
                    *counts.entry(turn_id.clone()).or_default() += 1;
                }
                counts
            });
    let page_event_counts = imported.session.events.iter().fold(
        BTreeMap::<String, usize>::new(),
        |mut counts, event| {
            if let Some(turn_id) = event.links.provider_turn_id.as_ref() {
                *counts.entry(turn_id.clone()).or_default() += 1;
            }
            counts
        },
    );
    for turn in &mut turns {
        if let Some(turn_id) = turn.provider_turn_id.as_ref() {
            if page_event_counts.get(turn_id) != full_event_counts.get(turn_id) {
                turn.confidence = crate::session_projection::TurnConfidence::Inferred;
            }
        }
    }

    Ok(ProviderSessionImportPage {
        imported,
        event_count,
        message_count,
        turn_count: Some(full_turns.len()),
        turns,
    })
}

fn import_canonical_session_from_dir(session_dir: &Path) -> Result<ImportedSession> {
    if kiro_session_source_fingerprint(session_dir)?.is_none() {
        anyhow::bail!(
            "Current Kiro session source is missing required files: {}",
            session_dir.display()
        );
    }

    let metadata = read_validated_session_metadata(session_dir)?;
    let metadata_path = session_dir.join("session.json");
    let raw_metadata: Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).with_context(|| {
            format!("Failed to read Kiro metadata: {}", metadata_path.display())
        })?)
        .with_context(|| format!("Failed to parse Kiro metadata: {}", metadata_path.display()))?;
    let created_at = metadata.created_at.as_deref().and_then(parse_utc_timestamp);
    let metadata_last_active_at = metadata
        .last_modified_at
        .as_deref()
        .and_then(parse_utc_timestamp);
    let timestamp_base = created_at.unwrap_or_else(|| DateTime::<Utc>::from(std::time::UNIX_EPOCH));
    let model_id = raw_metadata
        .get("modelId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);

    let messages_path = session_dir.join("messages.jsonl");
    let mut imported = read_kiro_event_stream(
        &messages_path,
        "messages",
        None,
        model_id.as_deref(),
        timestamp_base,
        &mut report,
    )?;

    let sub_executions_dir = session_dir.join("sub-executions");
    if sub_executions_dir.exists() {
        for sub_execution_path in sorted_jsonl_files(&sub_executions_dir)? {
            let sub_execution_id = sub_execution_path
                .file_stem()
                .and_then(|value| value.to_str())
                .context("Kiro sub-execution file has no valid execution id")?
                .to_string();
            let source_file = format!(
                "sub-executions/{}",
                sub_execution_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .context("Kiro sub-execution file has no valid name")?
            );
            let parent_event_id = imported
                .sub_execution_parents
                .get(&sub_execution_id)
                .cloned();
            let mut sub_import = read_kiro_event_stream(
                &sub_execution_path,
                &source_file,
                Some(&sub_execution_id),
                model_id.as_deref(),
                timestamp_base,
                &mut report,
            )?;
            if let Some(parent_event_id) = parent_event_id {
                for event in &mut sub_import.events {
                    event.event.links.parent_event_id = Some(parent_event_id.clone());
                    event.event.links.provider_parent_id = Some(parent_event_id.clone());
                }
            } else if !sub_import.events.is_empty() {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Preserved,
                    code: "sub_execution_parent_unresolved".to_string(),
                    message: format!(
                        "Preserved Kiro sub-execution {sub_execution_id} by native timestamp without inventing a parent relation"
                    ),
                    path: Some(source_file),
                    raw: None,
                });
            }
            imported.events.extend(sub_import.events);
        }
    }

    imported.events.sort_by(|left, right| {
        left.event
            .timestamp
            .cmp(&right.event.timestamp)
            .then_with(|| left.source_file.cmp(&right.source_file))
            .then_with(|| left.source_line.cmp(&right.source_line))
    });
    let events = imported
        .events
        .into_iter()
        .map(|event| event.event)
        .collect::<Vec<_>>();
    let event_last_active_at = events.iter().map(|event| event.timestamp).max();
    let source_title = metadata
        .title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    let workspace_dir = metadata.workspace_paths.first().cloned();
    let mut extensions = BTreeMap::new();
    extensions.insert("kiro_session_metadata".to_string(), raw_metadata);

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: metadata.id.clone(),
                source_title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: metadata.id,
                    source_path: Some(session_dir.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir,
                created_at,
                last_active_at: metadata_last_active_at.max(event_last_active_at),
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn sorted_jsonl_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = std::fs::read_dir(directory)
        .with_context(|| {
            format!(
                "Failed to read Kiro source directory: {}",
                directory.display()
            )
        })?
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort_by_key(|entry| entry.file_name());
    files
        .into_iter()
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl"))
        .map(|entry| {
            let path = entry.path();
            source_file_marker(&path)?.with_context(|| {
                format!(
                    "Kiro source disappeared while importing: {}",
                    path.display()
                )
            })?;
            Ok(path)
        })
        .collect()
}

fn read_kiro_event_stream(
    path: &Path,
    source_file: &str,
    default_turn_id: Option<&str>,
    model_id: Option<&str>,
    timestamp_base: DateTime<Utc>,
    report: &mut MappingReport,
) -> Result<KiroStreamImport> {
    let file = File::open(path)
        .with_context(|| format!("Failed to read Kiro event stream: {}", path.display()))?;
    let mut imported = KiroStreamImport::default();
    let mut state = KiroStreamState {
        default_turn_id: default_turn_id.map(str::to_string),
        model_id: model_id.map(str::to_string),
        timestamp_base,
        active_turn_id: default_turn_id.map(str::to_string),
        tool_call_events: BTreeMap::new(),
    };

    for (line_index, line) in BufReader::new(file).lines().enumerate() {
        let line_number = line_index + 1;
        let line = line.with_context(|| {
            format!(
                "Failed to read Kiro event stream line: {}:{}",
                path.display(),
                line_number
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = match serde_json::from_str(&line) {
            Ok(record) => record,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Kiro event stream line: {error}"),
                    path: Some(format!("{source_file}:line:{line_number}")),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };
        let event =
            canonical_event_from_kiro_record(record, source_file, line_number, &mut state, report);
        if let Some(sub_execution_id) = event
            .metadata
            .provider_ext
            .get("kiro_record")
            .and_then(|record| record.get("payload"))
            .and_then(|payload| payload.get("subExecutionId"))
            .and_then(Value::as_str)
        {
            imported
                .sub_execution_parents
                .insert(sub_execution_id.to_string(), event.id.clone());
        }
        imported.events.push(KiroImportedEvent {
            event,
            source_file: source_file.to_string(),
            source_line: line_number,
        });
    }

    Ok(imported)
}

fn canonical_event_from_kiro_record(
    record: Value,
    source_file: &str,
    line_number: usize,
    state: &mut KiroStreamState,
    report: &mut MappingReport,
) -> SessionEvent {
    let path = format!("{source_file}:line:{line_number}");
    let payload = record.get("payload").cloned().unwrap_or(Value::Null);
    let payload_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let original_id = record.get("id").and_then(Value::as_str).map(str::to_string);
    let event_id = match (source_file, original_id.as_deref()) {
        ("messages", Some(id)) => id.to_string(),
        (_, Some(id)) => format!("kiro:{source_file}:{id}"),
        _ => format!("kiro:{source_file}:line:{line_number}"),
    };
    if original_id.is_none() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: MappingDisposition::Normalized,
            code: "missing_event_id".to_string(),
            message: "Generated a stable Kiro canonical event id from source location".to_string(),
            path: Some(path.clone()),
            raw: Some(record.clone()),
        });
    }
    let timestamp = record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_utc_timestamp)
        .unwrap_or_else(|| {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Normalized,
                code: "invalid_event_timestamp".to_string(),
                message: "Used the session timestamp plus source line offset for a Kiro event"
                    .to_string(),
                path: Some(path.clone()),
                raw: record.get("timestamp").cloned(),
            });
            state.timestamp_base + chrono::Duration::milliseconds(line_number as i64)
        });

    // `subExecutionId` identifies a child stream from the main stream; it is
    // not the event's own turn id. Child stream events inherit their native
    // turn id from the file name through `default_turn_id`.
    let explicit_turn_id = payload
        .get("executionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    if payload_type == "turn_start" {
        state.active_turn_id = explicit_turn_id
            .clone()
            .or_else(|| state.default_turn_id.clone());
    }
    let provider_turn_id = explicit_turn_id
        .or_else(|| state.active_turn_id.clone())
        .or_else(|| state.default_turn_id.clone());
    let mut links = EventLinks {
        provider_turn_id,
        ..EventLinks::default()
    };
    let mut fidelity = MappingDisposition::Preserved;

    let (kind, role, blocks) = match payload_type {
        "user" => (
            SessionEventKind::Message,
            EventRole::User,
            kiro_message_blocks(&payload, false, payload_type, &path, report),
        ),
        "assistant" => {
            let operation_type = payload
                .get("operationType")
                .and_then(Value::as_str)
                .unwrap_or("Say");
            let reasoning = operation_type.eq_ignore_ascii_case("reasoning");
            if !reasoning && !operation_type.eq_ignore_ascii_case("say") {
                fidelity = MappingDisposition::Normalized;
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Normalized,
                    code: "unknown_assistant_operation".to_string(),
                    message: format!(
                        "Mapped Kiro assistant operation {operation_type} as visible text"
                    ),
                    path: Some(path.clone()),
                    raw: Some(payload.clone()),
                });
            }
            (
                SessionEventKind::Message,
                EventRole::Assistant,
                kiro_message_blocks(&payload, reasoning, payload_type, &path, report),
            )
        }
        "system" => (
            SessionEventKind::Message,
            EventRole::System,
            kiro_message_blocks(&payload, false, payload_type, &path, report),
        ),
        "agent_note" => (
            SessionEventKind::Message,
            EventRole::Assistant,
            kiro_message_blocks(&payload, false, payload_type, &path, report),
        ),
        "tool_call" => {
            let tool_call_id = payload
                .get("toolCallId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "missing_tool_call_id".to_string(),
                        message: "Generated a stable tool call id from the Kiro event id"
                            .to_string(),
                        path: Some(path.clone()),
                        raw: Some(payload.clone()),
                    });
                    event_id.clone()
                });
            let name = payload
                .get("toolName")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "missing_tool_name".to_string(),
                        message: "Used `unknown` because the Kiro tool call had no tool name"
                            .to_string(),
                        path: Some(path.clone()),
                        raw: Some(payload.clone()),
                    });
                    "unknown".to_string()
                });
            state
                .tool_call_events
                .insert(tool_call_id.clone(), event_id.clone());
            (
                SessionEventKind::ToolCall,
                EventRole::Assistant,
                vec![EventBlock::ToolCall {
                    tool_call_id,
                    name,
                    input: payload.get("args").cloned(),
                }],
            )
        }
        "tool_result" => {
            let tool_call_id = payload
                .get("toolCallId")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Warning,
                        disposition: MappingDisposition::Normalized,
                        code: "missing_tool_result_call_id".to_string(),
                        message: "Used `unknown` because the Kiro tool result had no tool call id"
                            .to_string(),
                        path: Some(path.clone()),
                        raw: Some(payload.clone()),
                    });
                    "unknown".to_string()
                });
            links.provider_parent_id = Some(tool_call_id.clone());
            if let Some(parent_event_id) = state.tool_call_events.get(&tool_call_id) {
                links.parent_event_id = Some(parent_event_id.clone());
            }
            let content = kiro_tool_result_content(payload.get("content"), &path, report);
            let is_error = payload.get("success").and_then(Value::as_bool) == Some(false)
                || matches!(
                    payload.get("status").and_then(Value::as_str),
                    Some("error" | "failed")
                );
            (
                SessionEventKind::ToolResult,
                EventRole::Tool,
                vec![EventBlock::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                }],
            )
        }
        "turn_start" => {
            links.turn_boundary = Some(TurnBoundary::Started);
            (
                SessionEventKind::Lifecycle,
                EventRole::System,
                vec![EventBlock::ProviderPayload {
                    kind: payload_type.to_string(),
                    payload: payload.clone(),
                }],
            )
        }
        "turn_end" => {
            links.turn_boundary = Some(kiro_turn_end_boundary(&payload));
            (
                SessionEventKind::Lifecycle,
                EventRole::System,
                vec![EventBlock::ProviderPayload {
                    kind: payload_type.to_string(),
                    payload: payload.clone(),
                }],
            )
        }
        "session_start"
        | "usage_summary"
        | "session_metadata"
        | "session_event"
        | "sub_agent_start"
        | "sub_agent_complete"
        | "sub_agent_progress"
        | "steering_inclusion"
        | "tombstone"
        | "ContextualHookInvoked"
        | "pending_interaction"
        | "interaction_resolved" => (
            SessionEventKind::Lifecycle,
            EventRole::System,
            vec![EventBlock::ProviderPayload {
                kind: payload_type.to_string(),
                payload: payload.clone(),
            }],
        ),
        unknown => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Preserved,
                code: "unknown_payload_preserved".to_string(),
                message: format!("Preserved unknown Kiro payload type {unknown}"),
                path: Some(path.clone()),
                raw: Some(payload.clone()),
            });
            (
                SessionEventKind::Unknown,
                EventRole::Unknown,
                vec![EventBlock::ProviderPayload {
                    kind: unknown.to_string(),
                    payload: payload.clone(),
                }],
            )
        }
    };

    if payload_type == "session_start" {
        if let Some(message_id) = payload.get("messageId").and_then(Value::as_str) {
            links.related_event_ids.push(message_id.to_string());
        }
    }
    if payload_type == "turn_end" {
        state.active_turn_id = state.default_turn_id.clone();
    }

    SessionEvent {
        id: event_id,
        kind,
        role,
        timestamp,
        links,
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id,
                original_role: Some(payload_type.to_string()),
                phase: payload
                    .get("operationType")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            model: state.model_id.clone(),
            usage: None,
            fidelity,
            provider_ext: BTreeMap::from([
                ("kiro_record".to_string(), record),
                (
                    "kiro_source".to_string(),
                    json!({"file": source_file, "line": line_number}),
                ),
            ]),
        },
    }
}

fn kiro_message_blocks(
    payload: &Value,
    reasoning: bool,
    payload_type: &str,
    path: &str,
    report: &mut MappingReport,
) -> Vec<EventBlock> {
    let Some(content) = payload.get("content") else {
        return vec![EventBlock::ProviderPayload {
            kind: payload_type.to_string(),
            payload: payload.clone(),
        }];
    };
    let mut blocks = Vec::new();
    match content {
        Value::String(text) => push_kiro_text_block(&mut blocks, text, reasoning),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(text) => push_kiro_text_block(&mut blocks, text, reasoning),
                    Value::Object(object) => {
                        if let Some(text) = object.get("text").and_then(Value::as_str) {
                            push_kiro_text_block(&mut blocks, text, reasoning);
                        } else if let Some(thinking) =
                            object.get("thinking").and_then(Value::as_str)
                        {
                            blocks.push(EventBlock::Thinking {
                                text: thinking.to_string(),
                                signature: object
                                    .get("signature")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                            });
                        } else {
                            blocks.push(EventBlock::ProviderPayload {
                                kind: object
                                    .get("type")
                                    .and_then(Value::as_str)
                                    .unwrap_or("content")
                                    .to_string(),
                                payload: item.clone(),
                            });
                        }
                    }
                    _ => blocks.push(EventBlock::Unknown { raw: item.clone() }),
                }
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                push_kiro_text_block(&mut blocks, text, reasoning);
            } else {
                blocks.push(EventBlock::ProviderPayload {
                    kind: object
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("content")
                        .to_string(),
                    payload: content.clone(),
                });
            }
        }
        _ => blocks.push(EventBlock::Unknown {
            raw: content.clone(),
        }),
    }
    if blocks.is_empty() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: MappingDisposition::Normalized,
            code: "empty_message_content".to_string(),
            message: "Preserved an empty Kiro message as provider payload".to_string(),
            path: Some(path.to_string()),
            raw: Some(payload.clone()),
        });
        blocks.push(EventBlock::ProviderPayload {
            kind: payload_type.to_string(),
            payload: payload.clone(),
        });
    }
    blocks
}

fn push_kiro_text_block(blocks: &mut Vec<EventBlock>, text: &str, reasoning: bool) {
    if text.is_empty() {
        return;
    }
    if reasoning {
        blocks.push(EventBlock::Thinking {
            text: text.to_string(),
            signature: None,
        });
    } else {
        blocks.push(EventBlock::Text {
            text: text.to_string(),
        });
    }
}

fn kiro_tool_result_content(
    content: Option<&Value>,
    path: &str,
    report: &mut MappingReport,
) -> String {
    match content {
        Some(Value::String(content)) => content.clone(),
        Some(Value::Null) | None => String::new(),
        Some(content) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Normalized,
                code: "structured_tool_result_normalized".to_string(),
                message: "Serialized structured Kiro tool result content as JSON".to_string(),
                path: Some(path.to_string()),
                raw: Some(content.clone()),
            });
            serde_json::to_string(content).unwrap_or_default()
        }
    }
}

fn kiro_turn_end_boundary(payload: &Value) -> TurnBoundary {
    match payload
        .get("stopReason")
        .or_else(|| payload.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("end_turn")
    {
        "error" | "failed" | "failure" => TurnBoundary::Failed,
        "interrupted" | "cancelled" | "canceled" | "aborted" => TurnBoundary::Interrupted,
        _ => TurnBoundary::Completed,
    }
}

fn parse_utc_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

struct SourceFileMarker {
    value: String,
    modified_at_ms: i64,
    size_bytes: i64,
}

fn source_file_marker(path: &Path) -> Result<Option<SourceFileMarker>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to inspect Kiro source: {}", path.display()))
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!("Kiro source is not a regular file: {}", path.display());
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok());
    let modified_at_ms = modified
        .as_ref()
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    let modified_at_ns = modified.map(|duration| duration.as_nanos()).unwrap_or(0);
    let size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
    Ok(Some(SourceFileMarker {
        value: format!("present:{modified_at_ns}:{size_bytes}"),
        modified_at_ms,
        size_bytes,
    }))
}

fn kiro_session_source_fingerprint(
    session_dir: &Path,
) -> Result<Option<ProviderSourceFingerprint>> {
    let sessions_root = kiro_sessions_dir()?;
    if session_dir.parent().and_then(Path::parent) != Some(sessions_root.as_path()) {
        anyhow::bail!(
            "Kiro session source locator is outside the configured sessions root: {}",
            session_dir.display()
        );
    }

    let directory_metadata = match std::fs::symlink_metadata(session_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to inspect Kiro session source: {}",
                    session_dir.display()
                )
            })
        }
    };
    if !directory_metadata.file_type().is_dir() || directory_metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kiro session source locator must be a directory: {}",
            session_dir.display()
        );
    }

    let Some(session_marker) = source_file_marker(&session_dir.join("session.json"))? else {
        return Ok(None);
    };
    let Some(messages_marker) = source_file_marker(&session_dir.join("messages.jsonl"))? else {
        return Ok(None);
    };
    read_validated_session_metadata(session_dir)?;

    let sub_executions_dir = session_dir.join("sub-executions");
    let mut sub_execution_markers = Vec::new();
    let mut modified_at_ms = session_marker
        .modified_at_ms
        .max(messages_marker.modified_at_ms);
    let mut size_bytes = session_marker
        .size_bytes
        .saturating_add(messages_marker.size_bytes);

    match std::fs::symlink_metadata(&sub_executions_dir) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "Kiro sub-executions source is not a directory: {}",
                    sub_executions_dir.display()
                );
            }
            let mut entries = std::fs::read_dir(&sub_executions_dir)
                .with_context(|| {
                    format!(
                        "Failed to read Kiro sub-executions: {}",
                        sub_executions_dir.display()
                    )
                })?
                .collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
                    continue;
                }
                let marker = source_file_marker(&path)?.with_context(|| {
                    format!(
                        "Kiro sub-execution disappeared while scanning: {}",
                        path.display()
                    )
                })?;
                let name = entry.file_name().to_string_lossy().to_string();
                modified_at_ms = modified_at_ms.max(marker.modified_at_ms);
                size_bytes = size_bytes.saturating_add(marker.size_bytes);
                sub_execution_markers.push(format!("{name}:{}", marker.value));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to inspect Kiro sub-executions: {}",
                    sub_executions_dir.display()
                )
            })
        }
    }

    let sub_execution_value = if sub_execution_markers.is_empty() {
        "absent".to_string()
    } else {
        let joined = sub_execution_markers.join("\0");
        format!(
            "{}:{:x}",
            sub_execution_markers.len(),
            Sha256::digest(joined.as_bytes())
        )
    };

    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes,
        value: format!(
            "kiro-v2:session:{}:messages:{}:sub-executions:{}",
            session_marker.value, messages_marker.value, sub_execution_value
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{PageStrategy, ProviderBackupSupport};
    use crate::storage::local_store;
    use serde_json::{json, Value};
    use std::collections::BTreeSet;
    use std::fs;
    use std::io::Write;
    use std::sync::{MutexGuard, OnceLock};

    static TEST_KIRO_TEST_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

    struct TestKiroSessionsDirGuard {
        _lock: MutexGuard<'static, ()>,
    }

    struct TestConfigHomeGuard;

    impl TestConfigHomeGuard {
        fn new(path: &Path) -> Self {
            crate::config::set_test_home_dir(path.to_path_buf());
            Self
        }
    }

    impl Drop for TestConfigHomeGuard {
        fn drop(&mut self) {
            crate::config::reset_test_home_dir();
        }
    }

    impl Drop for TestKiroSessionsDirGuard {
        fn drop(&mut self) {
            crate::cache::global_cache().invalidate(PROVIDER_ID);
            *TEST_KIRO_SESSIONS_DIR
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }

    fn use_test_kiro_sessions_dir(path: PathBuf) -> TestKiroSessionsDirGuard {
        let lock = TEST_KIRO_TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *TEST_KIRO_SESSIONS_DIR
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        TestKiroSessionsDirGuard { _lock: lock }
    }

    fn kiro_audit_fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/providers/kiro/fixtures/v1_0_138")
    }

    fn read_jsonl_values(path: &Path) -> Vec<Result<Value, serde_json::Error>> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect()
    }

    fn copy_tree(source: &Path, target: &Path) -> Result<()> {
        for entry in WalkDir::new(source).follow_links(false) {
            let entry = entry?;
            let relative = entry.path().strip_prefix(source)?;
            let destination = target.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir_all(&destination)?;
            } else if entry.file_type().is_file() {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(entry.path(), destination)?;
            }
        }
        Ok(())
    }

    fn copy_fixture_sessions() -> Result<tempfile::TempDir> {
        let temp = tempfile::tempdir()?;
        copy_tree(&kiro_audit_fixture_root().join("sessions"), temp.path())?;
        Ok(temp)
    }

    #[test]
    fn kiro_v2_audit_fixture_matches_official_session_directory_contract() {
        let root = kiro_audit_fixture_root();
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("fixture.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["provider"], "kiro");
        assert_eq!(manifest["source_plane"], "kiro-agent-v2");
        assert_eq!(manifest["observed_ide_version"], "1.0.138");
        assert_eq!(manifest["observed_extension_version"], "1.0.231");
        assert_eq!(manifest["observed_schema_version"], "1.0.0");
        assert_eq!(manifest["observed_data_model_version"], 1);
        assert_eq!(manifest["raw_user_content_committed"], false);
        assert_eq!(manifest["storage_root"], "~/.kiro/sessions");
        assert_eq!(
            manifest["official_artifact_sha256"],
            "29c7541056b4ca6849d73c1062ae1d215a80a9f7fc74a8240cb2bf9b8e1fd68b"
        );

        let session_id = manifest["normal_session_id"].as_str().unwrap();
        let workspace_path = "/workspace/sanitized-project";
        assert_eq!(
            workspace_bucket(&[workspace_path.to_string()]).unwrap(),
            "8f3d1d8bb1bd8116"
        );

        let session_dir = root
            .join("sessions")
            .join("8f3d1d8bb1bd8116")
            .join(session_id);
        let metadata: Value = serde_json::from_str(
            &std::fs::read_to_string(session_dir.join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["schemaVersion"], "1.0.0");
        assert_eq!(metadata["dataModelVersion"], 1);
        assert_eq!(metadata["id"], session_id);
        assert_eq!(metadata["workspacePaths"], json!([workspace_path]));
        assert_eq!(metadata["title"], "Sanitized Kiro session");
        assert_eq!(metadata["status"], "completed");

        assert!(session_dir.join("messages.jsonl").is_file());
        assert!(session_dir.join("sub-executions/subexec-1.jsonl").is_file());
        assert!(session_dir
            .join("tool-outputs/tool-1-a1b2c3d4.txt")
            .is_file());
        assert!(session_dir
            .join("snapshots/snap0001/src/example.rs")
            .is_file());
        assert!(session_dir.join("snapshots/snap0001/.hash").is_file());

        let messages = read_jsonl_values(&session_dir.join("messages.jsonl"));
        assert_eq!(messages.len(), 10);
        assert!(messages.iter().all(Result::is_ok));
        let payload_types = messages
            .into_iter()
            .map(Result::unwrap)
            .map(|message| message["payload"]["type"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            payload_types,
            [
                "session_start",
                "turn_start",
                "user",
                "assistant",
                "tool_call",
                "tool_result",
                "assistant",
                "usage_summary",
                "turn_end",
                "session_metadata",
            ]
        );

        let global_id = manifest["global_session_id"].as_str().unwrap();
        let global_dir = root.join("sessions").join("_global").join(global_id);
        let global_metadata: Value = serde_json::from_str(
            &std::fs::read_to_string(global_dir.join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(global_metadata["id"], global_id);
        assert_eq!(global_metadata["workspacePaths"], json!([]));
        assert_eq!(
            read_jsonl_values(&global_dir.join("messages.jsonl")).len(),
            4
        );
    }

    #[test]
    fn kiro_v2_audit_fixture_covers_projection_changes_and_invalid_records() {
        let root = kiro_audit_fixture_root();
        let variants = root.join("variants");
        let normal_dir = root
            .join("sessions/8f3d1d8bb1bd8116")
            .join("sess_11111111-1111-4111-8111-111111111111");

        let original_metadata: Value = serde_json::from_str(
            &std::fs::read_to_string(normal_dir.join("session.json")).unwrap(),
        )
        .unwrap();
        let updated_metadata: Value = serde_json::from_str(
            &std::fs::read_to_string(variants.join("session.updated.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(original_metadata["id"], updated_metadata["id"]);
        assert_ne!(original_metadata["title"], updated_metadata["title"]);
        assert_ne!(
            original_metadata["lastModifiedAt"],
            updated_metadata["lastModifiedAt"]
        );

        assert_eq!(
            read_jsonl_values(&normal_dir.join("messages.jsonl")).len(),
            10
        );
        assert_eq!(
            read_jsonl_values(&variants.join("messages.updated.jsonl")).len(),
            14
        );
        assert_eq!(
            read_jsonl_values(&normal_dir.join("sub-executions/subexec-1.jsonl")).len(),
            2
        );
        assert_eq!(
            read_jsonl_values(&variants.join("sub-execution.updated.jsonl")).len(),
            3
        );

        let malformed = read_jsonl_values(&variants.join("messages.malformed.jsonl"));
        assert_eq!(malformed.len(), 3);
        assert_eq!(malformed.iter().filter(|value| value.is_ok()).count(), 2);
        assert_eq!(malformed.iter().filter(|value| value.is_err()).count(), 1);

        let unknown = read_jsonl_values(&variants.join("messages.unknown.jsonl"));
        assert_eq!(unknown.len(), 1);
        assert_eq!(
            unknown[0].as_ref().unwrap()["payload"]["type"],
            "future_kiro_payload"
        );
        assert_eq!(
            unknown[0].as_ref().unwrap()["payload"]["futureField"]["preserve"],
            true
        );
    }

    #[test]
    fn current_format_scan_uses_directory_locators_and_truthful_capabilities() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());

        let capabilities = KiroProvider.capabilities();
        assert!(capabilities.scan);
        assert!(capabilities.import);
        assert!(!capabilities.export);
        assert!(!capabilities.delete);
        assert!(!capabilities.rename);
        assert!(!capabilities.resume);
        assert_eq!(capabilities.scan_strategy, ScanStrategy::FullScan);
        assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);
        assert_eq!(capabilities.storage_shape, StorageShape::Directory);
        assert_eq!(capabilities.turn_quality, TurnQuality::Exact);
        assert_eq!(
            capabilities.import_fidelity.tool_call,
            Some(MappingDisposition::Preserved)
        );
        assert_eq!(
            capabilities.import_fidelity.provider_payload,
            Some(MappingDisposition::Preserved)
        );
        assert_eq!(
            capabilities.backup_support,
            ProviderBackupSupport {
                before_write: false,
                restore: false,
                sync_only: false,
            }
        );

        let sessions = KiroProvider.scan_sessions()?;
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            sessions[0].session_id,
            "sess_22222222-2222-4222-8222-222222222222"
        );
        assert_eq!(sessions[0].project_dir, None);
        assert_eq!(
            sessions[1].session_id,
            "sess_11111111-1111-4111-8111-111111111111"
        );
        assert_eq!(sessions[1].title.as_deref(), Some("Sanitized Kiro session"));
        assert_eq!(
            sessions[1].project_dir.as_deref(),
            Some("/workspace/sanitized-project")
        );
        let source_path = PathBuf::from(sessions[1].source_path.as_ref().unwrap());
        assert!(source_path.is_dir());
        assert_eq!(
            source_path.file_name().and_then(|name| name.to_str()),
            Some(sessions[1].session_id.as_str())
        );
        assert_eq!(
            KiroProvider
                .get_session_meta(&sessions[1].session_id)?
                .unwrap()
                .source_path,
            sessions[1].source_path
        );
        assert!(KiroProvider.session_size(&sessions[1].session_id)? > 0);
        assert_eq!(
            KiroProvider
                .import_session(source_path.to_str().unwrap())?
                .session
                .identity
                .canonical_id,
            sessions[1].session_id
        );
        assert_eq!(KiroProvider.data_source_paths(), vec![temp.path()]);
        Ok(())
    }

    #[test]
    fn current_format_full_import_page_keeps_total_counts_and_marks_partial_turns_inferred(
    ) -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_dir = temp
            .path()
            .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
        let full = KiroProvider.import_session_page(session_dir.to_str().unwrap(), 0, None)?;
        assert_eq!(full.event_count, full.imported.session.events.len());
        assert_eq!(full.turn_count, Some(full.turns.len()));
        assert!(full
            .turns
            .iter()
            .all(|turn| { turn.confidence == crate::session_projection::TurnConfidence::Exact }));

        let page = KiroProvider.import_session_page(session_dir.to_str().unwrap(), 3, Some(2))?;
        assert_eq!(page.event_count, full.event_count);
        assert_eq!(page.message_count, full.message_count);
        assert_eq!(page.turn_count, full.turn_count);
        assert_eq!(page.imported.session.events.len(), 2);
        assert_eq!(page.turns.len(), 1);
        assert_eq!(page.turns[0].provider_turn_id.as_deref(), Some("exec-1"));
        assert_eq!(
            page.turns[0].confidence,
            crate::session_projection::TurnConfidence::Inferred
        );
        Ok(())
    }

    #[test]
    fn current_format_index_and_detail_are_idempotent_source_backed_and_bodyless() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let home = temp.path().join("home");
        fs::create_dir_all(&home)?;
        let _home_guard = TestConfigHomeGuard::new(&home);
        let _kiro_guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_id = "sess_11111111-1111-4111-8111-111111111111";
        let session_dir = temp.path().join("8f3d1d8bb1bd8116").join(session_id);
        let summary = KiroProvider
            .scan_sessions()?
            .into_iter()
            .find(|summary| summary.session_id == session_id)
            .unwrap();
        assert_eq!(
            summary.source_path.as_deref(),
            Some(session_dir.to_string_lossy().as_ref())
        );
        let fingerprint = KiroProvider
            .session_source_fingerprint(summary.source_path.as_deref().unwrap())?
            .unwrap();
        assert!(fingerprint.value.starts_with("kiro-v2:"));
        let full =
            KiroProvider.import_session_page(summary.source_path.as_deref().unwrap(), 0, None)?;
        let expected_turn_count = full.turn_count.unwrap();

        let mut conn = local_store::open_database()?;
        let first = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                PROVIDER_ID,
                &summary,
                KiroProvider.capabilities(),
                &fingerprint,
            )?;
        let counts_after_first: (i64, i64, i64, i64) = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'kiro'),
                (SELECT COUNT(*) FROM sessions WHERE provider_id = 'kiro'),
                (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'kiro'),
                (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'kiro')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let second = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                PROVIDER_ID,
                &summary,
                KiroProvider.capabilities(),
                &fingerprint,
            )?;
        let counts_after_second: (i64, i64, i64, i64) = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'kiro'),
                (SELECT COUNT(*) FROM sessions WHERE provider_id = 'kiro'),
                (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'kiro'),
                (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'kiro')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(first, second);
        assert_eq!(counts_after_first, counts_after_second);
        assert_eq!(counts_after_second.0, 1);
        assert_eq!(counts_after_second.1, 1);
        assert_eq!(counts_after_second.2, 1);

        let (source_path, storage_shape, source_cursor): (String, String, String) = conn
            .query_row(
                "SELECT source_path, storage_shape, source_cursor
                 FROM session_sources WHERE id = ?1",
                [&first.source_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        assert_eq!(source_path, session_dir.to_string_lossy());
        assert_eq!(storage_shape, "directory");
        assert_eq!(source_cursor, fingerprint.value);
        let snapshot_json: String = conn.query_row(
            "SELECT snapshot_json FROM session_snapshots WHERE session_id = ?1",
            [&first.canonical_session_id],
            |row| row.get(0),
        )?;
        let snapshot_json: Value = serde_json::from_str(&snapshot_json)?;
        let snapshot_keys = snapshot_json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            snapshot_keys,
            BTreeSet::from(["index_version", "source_fingerprint"])
        );
        let body_table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(body_table_count, 0);
        drop(conn);

        let detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        assert!(detail.events.is_empty());
        assert!(detail.turns.is_empty());
        assert_eq!(detail.event_count, full.event_count);
        assert_eq!(detail.message_count, full.message_count);
        assert!(!detail.stale);
        assert_eq!(
            detail.source_path.as_deref(),
            Some(session_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            detail.projection_report.as_ref().unwrap().id,
            format!("source-read:{PROVIDER_ID}:{session_id}")
        );

        let conn = local_store::open_database()?;
        let cached_counts: (i64, i64, i64, i64) = conn.query_row(
            "SELECT event_count, message_count, turn_count, counts_complete
             FROM session_snapshots WHERE session_id = ?1",
            [&first.canonical_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(cached_counts.0, full.event_count as i64);
        assert_eq!(cached_counts.1, full.message_count as i64);
        assert_eq!(cached_counts.2, expected_turn_count as i64);
        assert_eq!(cached_counts.3, 1);
        drop(conn);

        fs::OpenOptions::new()
            .append(true)
            .open(session_dir.join("messages.jsonl"))?
            .write_all(b"\n")?;
        let stale_detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        assert!(stale_detail.stale);

        fs::remove_dir_all(&session_dir)?;
        let groups = crate::core::list_sessions(&crate::core::SessionListParams {
            all: true,
            providers: vec![PROVIDER_ID.to_string()],
            cwd: None,
            include_message_counts: true,
            limit: None,
            offset: None,
            sort: crate::core::SessionListSort::Recent,
            hook_filter: crate::core::SessionHookFilter::All,
        })?;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].sessions.len(), 1);
        assert_eq!(groups[0].sessions[0].session_id, session_id);
        assert_eq!(
            groups[0].sessions[0].message_count,
            Some(full.message_count)
        );
        let error = crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(1))
            .unwrap_err();
        assert!(format!("{error:#}").contains("Session source is missing"));
        Ok(())
    }

    #[test]
    fn current_format_bootstrap_stale_and_system_sync_are_incremental_and_bodyless() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let home = tempfile::tempdir()?;
        let _home_guard = TestConfigHomeGuard::new(home.path());
        let _kiro_guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_id = "sess_11111111-1111-4111-8111-111111111111";
        let session_dir = temp.path().join("8f3d1d8bb1bd8116").join(session_id);

        let first = crate::core::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(first.scanned_providers, 1);
        assert_eq!(first.discovered_sessions, 2);
        assert_eq!(first.projected_sessions, 2);
        assert_eq!(first.unchanged_sessions, 0);
        assert!(first.failures.is_empty());

        let detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        assert!(detail.events.is_empty());
        assert!(detail.turns.is_empty());
        assert!(!detail.stale);

        let conn = local_store::open_database()?;
        let initial: (String, String, i64, i64, i64, i64) = conn.query_row(
            "SELECT ss.title, ss.workspace_dir, ss.counts_complete, ss.stale,
                    src.scan_generation,
                    (SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'table'
                       AND name IN ('session_turns', 'session_events', 'session_event_blocks'))
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kiro' AND s.provider_session_id = ?1",
            [session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        let initial_fingerprint: String = conn.query_row(
            "SELECT src.source_cursor
             FROM session_sources src
             WHERE src.provider_id = 'kiro' AND src.provider_session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        assert_eq!(initial.0, "Sanitized Kiro session");
        assert_eq!(initial.1, "/workspace/sanitized-project");
        assert_eq!(initial.2, 1);
        assert_eq!(initial.3, 0);
        assert_eq!(initial.4, 1);
        assert_eq!(initial.5, 0);
        drop(conn);

        let unchanged = crate::core::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(unchanged.scanned_providers, 1);
        assert_eq!(unchanged.discovered_sessions, 2);
        assert_eq!(unchanged.projected_sessions, 0);
        assert_eq!(unchanged.unchanged_sessions, 2);
        assert!(unchanged.failures.is_empty());

        let conn = local_store::open_database()?;
        let unchanged_state: (i64, i64) = conn.query_row(
            "SELECT src.scan_generation, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kiro' AND s.provider_session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(unchanged_state, (1, 1));
        drop(conn);

        fs::OpenOptions::new()
            .append(true)
            .open(session_dir.join("messages.jsonl"))?
            .write_all(b"\n")?;
        let stale = crate::core::refresh_projected_session_staleness(
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(stale.checked_sources, 2);
        assert_eq!(stale.fresh_snapshots, 1);
        assert_eq!(stale.stale_snapshots, 1);
        assert_eq!(stale.missing_sources, 0);

        let refreshed = crate::core::reproject_stale_sessions(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(refreshed.candidate_snapshots, 1);
        assert_eq!(refreshed.reprojected_snapshots, 1);
        assert_eq!(refreshed.missing_sources, 0);
        assert!(refreshed.failures.is_empty());

        let conn = local_store::open_database()?;
        let after_messages: (String, i64, i64) = conn.query_row(
            "SELECT src.source_cursor, ss.stale, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kiro' AND s.provider_session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_ne!(after_messages.0, initial_fingerprint);
        assert_eq!(after_messages.1, 0);
        assert_eq!(after_messages.2, 0);
        drop(conn);

        let detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        assert!(!detail.stale);
        let session_path = session_dir.join("session.json");
        fs::copy(
            kiro_audit_fixture_root().join("variants/session.updated.json"),
            &session_path,
        )?;
        let metadata_sync = crate::core::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(metadata_sync.projected_sessions, 1);
        assert_eq!(metadata_sync.unchanged_sessions, 1);

        let conn = local_store::open_database()?;
        let after_metadata: (String, String) = conn.query_row(
            "SELECT ss.title, src.source_cursor
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kiro' AND s.provider_session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(after_metadata.0, "Sanitized Kiro session (updated)");
        assert_ne!(after_metadata.1, after_messages.0);
        drop(conn);

        fs::OpenOptions::new()
            .append(true)
            .open(session_dir.join("sub-executions/subexec-1.jsonl"))?
            .write_all(b"\n")?;
        let sub_execution_sync = crate::core::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(sub_execution_sync.projected_sessions, 1);
        assert_eq!(sub_execution_sync.unchanged_sessions, 1);

        let conn = local_store::open_database()?;
        let after_sub_execution: String = conn.query_row(
            "SELECT src.source_cursor
             FROM session_sources src
             WHERE src.provider_id = 'kiro' AND src.provider_session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        assert_ne!(after_sub_execution, after_metadata.1);
        drop(conn);

        fs::remove_dir_all(&session_dir)?;
        let missing = crate::core::refresh_projected_session_staleness(
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(missing.checked_sources, 1);
        assert_eq!(missing.fresh_snapshots, 1);
        assert_eq!(missing.missing_sources, 1);
        assert_eq!(missing.stale_snapshots, 1);

        let missing_reprojection = crate::core::reproject_stale_sessions(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(missing_reprojection.candidate_snapshots, 1);
        assert_eq!(missing_reprojection.reprojected_snapshots, 0);
        assert_eq!(missing_reprojection.missing_sources, 1);

        let groups = crate::core::list_sessions(&crate::core::SessionListParams {
            all: true,
            providers: vec![PROVIDER_ID.to_string()],
            cwd: None,
            include_message_counts: true,
            limit: None,
            offset: None,
            sort: crate::core::SessionListSort::Recent,
            hook_filter: crate::core::SessionHookFilter::All,
        })?;
        let session = groups
            .iter()
            .flat_map(|group| &group.sessions)
            .find(|session| session.session_id == session_id)
            .unwrap();
        assert!(session.stale);

        let error = crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(1))
            .unwrap_err();
        assert!(format!("{error:#}").contains("Session source is missing"));

        let conn = local_store::open_database()?;
        let system_scan_activities: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_activity
             WHERE actor = 'system' AND operation_kind = 'scan' AND status != 'running'",
            [],
            |row| row.get(0),
        )?;
        assert!(system_scan_activities >= 7);
        let body_table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(body_table_count, 0);
        Ok(())
    }

    #[test]
    fn current_format_import_maps_main_and_sub_execution_events_without_fake_artifacts(
    ) -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_dir = temp
            .path()
            .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");

        let imported = KiroProvider.import_session(session_dir.to_str().unwrap())?;
        assert_eq!(
            imported.session.identity.canonical_id,
            "sess_11111111-1111-4111-8111-111111111111"
        );
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Sanitized Kiro session")
        );
        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some("/workspace/sanitized-project")
        );
        assert_eq!(imported.session.events.len(), 12);
        assert!(imported.session.artifacts.is_empty());
        assert_eq!(imported.report.overall, MappingDisposition::Preserved);
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "sub_execution_parent_unresolved"));
        assert_eq!(
            imported.session.extensions["kiro_session_metadata"]["modelId"],
            "sanitized-model"
        );

        let event = |id: &str| {
            imported
                .session
                .events
                .iter()
                .find(|event| event.metadata.source.original_id.as_deref() == Some(id))
                .unwrap()
        };
        assert_eq!(
            event("msg-user-1").links.provider_turn_id.as_deref(),
            Some("exec-1")
        );
        assert!(matches!(
            event("msg-reasoning-1").blocks.as_slice(),
            [EventBlock::Thinking { text, .. }] if text == "[sanitized reasoning]"
        ));
        assert!(matches!(
            event("msg-assistant-1").blocks.as_slice(),
            [EventBlock::Text { text }] if text == "[sanitized assistant response]"
        ));
        assert!(matches!(
            event("msg-tool-call-1").blocks.as_slice(),
            [EventBlock::ToolCall { tool_call_id, name, input: Some(input) }]
                if tool_call_id == "tool-1"
                    && name == "read_file"
                    && input["path"] == "src/example.rs"
        ));
        assert_eq!(
            event("msg-tool-result-1").links.parent_event_id.as_deref(),
            Some("msg-tool-call-1")
        );
        assert!(matches!(
            event("msg-tool-result-1").blocks.as_slice(),
            [EventBlock::ToolResult { tool_call_id, content, is_error }]
                if tool_call_id == "tool-1"
                    && content == "[sanitized tool output]"
                    && !is_error
        ));
        assert_eq!(
            event("exec-1-turn-start").links.turn_boundary,
            Some(TurnBoundary::Started)
        );
        assert_eq!(
            event("exec-1-turn-end").links.turn_boundary,
            Some(TurnBoundary::Completed)
        );
        assert_eq!(
            event("sub-msg-user-1").links.provider_turn_id.as_deref(),
            Some("subexec-1")
        );
        assert_eq!(
            event("sub-msg-assistant-1").metadata.provider_ext["kiro_source"]["file"],
            "sub-executions/subexec-1.jsonl"
        );

        let ordered_ids = imported
            .session
            .events
            .iter()
            .map(|event| event.metadata.source.original_id.as_deref().unwrap())
            .collect::<Vec<_>>();
        let tool_call_index = ordered_ids
            .iter()
            .position(|id| *id == "msg-tool-call-1")
            .unwrap();
        let sub_user_index = ordered_ids
            .iter()
            .position(|id| *id == "sub-msg-user-1")
            .unwrap();
        let tool_result_index = ordered_ids
            .iter()
            .position(|id| *id == "msg-tool-result-1")
            .unwrap();
        assert!(tool_call_index < sub_user_index && sub_user_index < tool_result_index);
        assert!(!serde_json::to_string(&imported.session.events)?
            .contains("sanitized external tool output"));
        Ok(())
    }

    #[test]
    fn current_format_import_keeps_exact_multi_turn_ids_and_explicit_sub_parent() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_dir = temp
            .path()
            .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
        let messages_path = session_dir.join("messages.jsonl");
        let mut messages =
            fs::read_to_string(kiro_audit_fixture_root().join("variants/messages.updated.jsonl"))?;
        messages.push_str(
            "{\"id\":\"sub-parent\",\"timestamp\":\"2026-07-16T00:00:04.050Z\",\"payload\":{\"type\":\"sub_agent_start\",\"executionId\":\"exec-1\",\"subExecutionId\":\"subexec-1\"}}\n",
        );
        fs::write(&messages_path, messages)?;

        let imported = KiroProvider.import_session(session_dir.to_str().unwrap())?;
        let event = |id: &str| {
            imported
                .session
                .events
                .iter()
                .find(|event| event.metadata.source.original_id.as_deref() == Some(id))
                .unwrap()
        };
        assert_eq!(
            event("msg-user-2").links.provider_turn_id.as_deref(),
            Some("exec-2")
        );
        assert_eq!(
            event("exec-2-turn-start").links.turn_boundary,
            Some(TurnBoundary::Started)
        );
        assert_eq!(
            event("exec-2-turn-end").links.turn_boundary,
            Some(TurnBoundary::Completed)
        );
        assert_eq!(
            event("sub-parent").links.provider_turn_id.as_deref(),
            Some("exec-1")
        );
        assert_eq!(
            event("sub-msg-user-1").links.parent_event_id.as_deref(),
            Some("sub-parent")
        );
        assert_eq!(
            event("sub-msg-assistant-1")
                .links
                .parent_event_id
                .as_deref(),
            Some("sub-parent")
        );
        assert!(!imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "sub_execution_parent_unresolved"));
        Ok(())
    }

    #[test]
    fn current_format_import_reports_malformed_and_preserves_unknown_payloads() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_dir = temp
            .path()
            .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
        fs::remove_dir_all(session_dir.join("sub-executions"))?;
        let variants = kiro_audit_fixture_root().join("variants");
        let messages_path = session_dir.join("messages.jsonl");

        fs::copy(variants.join("messages.malformed.jsonl"), &messages_path)?;
        let malformed = KiroProvider.import_session(session_dir.to_str().unwrap())?;
        assert_eq!(malformed.session.events.len(), 2);
        assert_eq!(malformed.report.overall, MappingDisposition::Dropped);
        let issue = malformed
            .report
            .issues
            .iter()
            .find(|issue| issue.code == "invalid_jsonl_line")
            .unwrap();
        assert_eq!(issue.path.as_deref(), Some("messages:line:2"));
        assert!(matches!(issue.raw, Some(Value::String(_))));

        fs::copy(variants.join("messages.unknown.jsonl"), &messages_path)?;
        let unknown = KiroProvider.import_session(session_dir.to_str().unwrap())?;
        assert_eq!(unknown.session.events.len(), 1);
        assert_eq!(unknown.report.overall, MappingDisposition::Preserved);
        assert!(unknown
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "unknown_payload_preserved"));
        assert_eq!(unknown.session.events[0].kind, SessionEventKind::Unknown);
        assert_eq!(unknown.session.events[0].role, EventRole::Unknown);
        assert!(matches!(
            unknown.session.events[0].blocks.as_slice(),
            [EventBlock::ProviderPayload { kind, payload }]
                if kind == "future_kiro_payload"
                    && payload["futureField"]["preserve"] == true
        ));
        Ok(())
    }

    #[test]
    fn current_format_import_reports_missing_tool_identifiers() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_dir = temp
            .path()
            .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
        fs::remove_dir_all(session_dir.join("sub-executions"))?;
        fs::write(
            session_dir.join("messages.jsonl"),
            concat!(
                "{\"id\":\"tool-call-missing-id\",\"timestamp\":\"2026-07-16T00:00:01.000Z\",\"payload\":{\"type\":\"tool_call\",\"args\":{\"path\":\"src/example.rs\"}}}\n",
                "{\"id\":\"tool-result-missing-id\",\"timestamp\":\"2026-07-16T00:00:02.000Z\",\"payload\":{\"type\":\"tool_result\",\"content\":\"[sanitized result]\"}}\n",
            ),
        )?;

        let imported = KiroProvider.import_session(session_dir.to_str().unwrap())?;
        assert_eq!(imported.report.overall, MappingDisposition::Normalized);
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "missing_tool_call_id"));
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "missing_tool_name"));
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "missing_tool_result_call_id"));
        assert!(matches!(
            imported.session.events[0].blocks.as_slice(),
            [EventBlock::ToolCall { tool_call_id, name, .. }]
                if tool_call_id == "tool-call-missing-id" && name == "unknown"
        ));
        assert!(matches!(
            imported.session.events[1].blocks.as_slice(),
            [EventBlock::ToolResult { tool_call_id, .. }]
                if tool_call_id == "unknown"
        ));
        Ok(())
    }

    #[test]
    fn current_format_import_classifies_known_payload_matrix() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_dir = temp
            .path()
            .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
        fs::remove_dir_all(session_dir.join("sub-executions"))?;
        fs::copy(
            kiro_audit_fixture_root().join("variants/messages.payload-matrix.jsonl"),
            session_dir.join("messages.jsonl"),
        )?;

        let imported = KiroProvider.import_session(session_dir.to_str().unwrap())?;
        assert_eq!(imported.session.events.len(), 11);
        assert_eq!(imported.report.overall, MappingDisposition::Preserved);
        assert_eq!(imported.session.events[0].kind, SessionEventKind::Message);
        assert_eq!(imported.session.events[0].role, EventRole::System);
        assert_eq!(imported.session.events[1].kind, SessionEventKind::Message);
        assert_eq!(imported.session.events[1].role, EventRole::Assistant);
        assert!(imported.session.events[2..]
            .iter()
            .all(|event| event.kind == SessionEventKind::Lifecycle));
        assert!(imported.session.events[2..].iter().all(|event| {
            matches!(
                event.blocks.as_slice(),
                [EventBlock::ProviderPayload { .. }]
            )
        }));
        Ok(())
    }

    #[test]
    fn current_format_fingerprint_covers_metadata_messages_and_sub_executions() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_dir = temp
            .path()
            .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
        let variants = kiro_audit_fixture_root().join("variants");
        let fingerprint = || {
            KiroProvider
                .session_source_fingerprint(session_dir.to_str().unwrap())
                .unwrap()
                .unwrap()
                .value
        };

        let baseline = fingerprint();
        assert!(baseline.starts_with("kiro-v2:"));
        assert!(baseline.contains(":sub-executions:1:"));

        let session_path = session_dir.join("session.json");
        let original_session = fs::read(&session_path)?;
        fs::copy(variants.join("session.updated.json"), &session_path)?;
        assert_ne!(fingerprint(), baseline);
        fs::write(&session_path, original_session)?;

        let messages_path = session_dir.join("messages.jsonl");
        let original_messages = fs::read(&messages_path)?;
        let restored_session_fingerprint = fingerprint();
        fs::copy(variants.join("messages.updated.jsonl"), &messages_path)?;
        assert_ne!(fingerprint(), restored_session_fingerprint);
        fs::write(&messages_path, original_messages)?;

        let sub_execution_path = session_dir.join("sub-executions/subexec-1.jsonl");
        let original_sub_execution = fs::read(&sub_execution_path)?;
        let restored_messages_fingerprint = fingerprint();
        fs::copy(
            variants.join("sub-execution.updated.jsonl"),
            &sub_execution_path,
        )?;
        assert_ne!(fingerprint(), restored_messages_fingerprint);
        fs::write(&sub_execution_path, original_sub_execution)?;

        let source_fingerprint = fingerprint();
        fs::write(
            session_dir.join("tool-outputs/tool-1-a1b2c3d4.txt"),
            "[changed artifact outside C2 canonical source scope]",
        )?;
        assert_eq!(fingerprint(), source_fingerprint);

        assert!(KiroProvider
            .session_source_fingerprint(session_path.to_str().unwrap())
            .unwrap_err()
            .to_string()
            .contains("outside the configured sessions root"));
        fs::remove_file(&messages_path)?;
        assert!(KiroProvider
            .session_source_fingerprint(session_dir.to_str().unwrap())?
            .is_none());
        assert!(KiroProvider
            .session_source_fingerprint(
                temp.path()
                    .join("missing/session")
                    .to_string_lossy()
                    .as_ref()
            )?
            .is_none());
        Ok(())
    }

    #[test]
    fn current_format_rejects_duplicate_ids_and_invalid_identity_buckets() -> Result<()> {
        let temp = copy_fixture_sessions()?;
        let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
        let session_id = "sess_11111111-1111-4111-8111-111111111111";
        let source_dir = temp.path().join("8f3d1d8bb1bd8116").join(session_id);
        let duplicate_workspace = "/workspace/duplicate".to_string();
        let duplicate_bucket = workspace_bucket(std::slice::from_ref(&duplicate_workspace))?;
        let duplicate_dir = temp.path().join(duplicate_bucket).join(session_id);
        copy_tree(&source_dir, &duplicate_dir)?;
        let metadata_path = duplicate_dir.join("session.json");
        let mut metadata: Value = serde_json::from_slice(&fs::read(&metadata_path)?)?;
        metadata["workspacePaths"] = json!([duplicate_workspace]);
        fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)?;

        assert!(KiroProvider
            .scan_sessions()
            .unwrap_err()
            .to_string()
            .contains("Ambiguous Kiro session id"));
        assert!(KiroProvider
            .get_session_meta(session_id)
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
        assert!(KiroProvider
            .session_size(session_id)
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
        assert!(KiroProvider
            .get_session_meta("../outside")
            .unwrap_err()
            .to_string()
            .contains("Invalid Kiro session id"));

        fs::remove_dir_all(duplicate_dir)?;
        let original_metadata = fs::read(source_dir.join("session.json"))?;
        let mut invalid_id: Value = serde_json::from_slice(&original_metadata)?;
        invalid_id["id"] = Value::String("different-session-id".to_string());
        fs::write(
            source_dir.join("session.json"),
            serde_json::to_vec_pretty(&invalid_id)?,
        )?;
        assert!(KiroProvider
            .scan_sessions()
            .unwrap_err()
            .to_string()
            .contains("does not match session directory"));

        fs::write(source_dir.join("session.json"), &original_metadata)?;
        let mut invalid_bucket: Value = serde_json::from_slice(&original_metadata)?;
        invalid_bucket["workspacePaths"] = json!(["/workspace/different"]);
        fs::write(
            source_dir.join("session.json"),
            serde_json::to_vec_pretty(&invalid_bucket)?,
        )?;
        assert!(KiroProvider
            .scan_sessions()
            .unwrap_err()
            .to_string()
            .contains("does not match metadata workspacePaths"));
        Ok(())
    }
}
