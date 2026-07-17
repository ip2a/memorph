pub mod adapter;
pub mod hook;

use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ImportedSession, MappingDirection, MappingDisposition, MappingIssue,
    MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext, SessionEvent,
    SessionEventKind, SessionIdentity, SessionProvenance, UsageStats,
};
use crate::provider::{
    PageStrategy, Provider, ProviderCapabilities, ProviderContentFidelity, ProviderSessionSummary,
    ProviderSourceFingerprint, ResumeQuality, ScanStrategy, StorageShape, TurnQuality,
};
use crate::utils::{extract_text, parse_timestamp_to_ms, truncate_summary};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct QwenProvider;

const PROVIDER_ID: &str = "qwen";
const SESSION_FILE_EXTENSION: &str = "jsonl";
const SESSION_FILE_PREFIX_LENGTH: usize = 32;
const SESSION_FILE_MAX_LENGTH: usize = 36;

#[cfg(test)]
static TEST_QWEN_RUNTIME_BASE: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[derive(Debug)]
struct QwenReaderIssue {
    line_number: usize,
    reason: String,
    raw: String,
}

#[derive(Debug)]
struct ParsedQwenSession {
    session_id: String,
    project_dir: Option<String>,
    records: Vec<Value>,
    reader_issues: Vec<QwenReaderIssue>,
}

impl Provider for QwenProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Qwen Code"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: false,
            delete: false,
            rename: false,
            resume: true,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Jsonl,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Preserved),
                tool_call: Some(MappingDisposition::Preserved),
                tool_result: Some(MappingDisposition::Preserved),
                provider_payload: Some(MappingDisposition::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            resume_quality: ResumeQuality::Native,
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let mut sessions = Vec::new();
        let mut seen = BTreeMap::<String, PathBuf>::new();

        for projects_dir in qwen_projects_dirs() {
            let mut project_dirs = direct_child_directories(&projects_dir)?;
            project_dirs.sort();
            for project_dir in project_dirs {
                let chats_dir = project_dir.join("chats");
                let mut files = direct_session_files(&chats_dir)?;
                files.sort();
                for path in files {
                    let path = match canonical_source_path(&path) {
                        Ok(path) => path,
                        Err(_) => continue,
                    };
                    let parsed = match parse_jsonl_session(&path) {
                        Ok(parsed) => parsed,
                        Err(_) => continue,
                    };
                    if let Some(previous_path) =
                        seen.insert(parsed.session_id.clone(), path.clone())
                    {
                        bail!(
                            "Ambiguous Qwen Code session identity {}: found in {} and {}",
                            parsed.session_id,
                            previous_path.display(),
                            path.display()
                        );
                    }
                    sessions.push(summary_from_parsed(&path, &parsed));
                }
            }
        }

        sessions.sort_by_key(|session| std::cmp::Reverse(session.last_active_at.unwrap_or(0)));
        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let path = canonical_source_path(Path::new(source_path))?;
        let parsed = parse_jsonl_session(&path).with_context(|| {
            format!("Failed to read Qwen Code JSONL session: {}", path.display())
        })?;
        import_parsed_session(&path, parsed)
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        let source_path = Path::new(source_path);
        if !source_path.exists() {
            return Ok(None);
        }
        let path = canonical_source_path(source_path)?;
        let metadata = std::fs::metadata(&path)?;
        let raw = std::fs::read(&path)?;
        let digest = Sha256::digest(&raw);
        let modified_at_ms = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0);
        let size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);

        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms,
            size_bytes,
            value: format!("qwen-code-jsonl-v1:{digest:x}"),
        }))
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("qwen --resume {session_id}"))
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        qwen_projects_dirs()
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let Some(source_path) = self
            .scan_sessions()?
            .into_iter()
            .find(|session| session.session_id == session_id)
            .and_then(|session| session.source_path)
        else {
            return Ok(0);
        };
        Ok(std::fs::metadata(source_path)?.len())
    }
}

fn qwen_runtime_base() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(base) = TEST_QWEN_RUNTIME_BASE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
    {
        return Some(base);
    }

    std::env::var_os("QWEN_RUNTIME_DIR")
        .or_else(|| std::env::var_os("QWEN_HOME"))
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".qwen")))
}

fn qwen_projects_dirs() -> Vec<PathBuf> {
    qwen_runtime_base()
        .map(|base| vec![base.join("projects")])
        .unwrap_or_default()
}

fn direct_child_directories(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut directories = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}

fn direct_session_files(chats_dir: &Path) -> Result<Vec<PathBuf>> {
    if !chats_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(chats_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_file()
            && is_session_id(name.trim_end_matches(".jsonl"))
            && name.ends_with(".jsonl")
        {
            files.push(entry.path());
        }
    }
    Ok(files)
}

fn is_session_id(value: &str) -> bool {
    (SESSION_FILE_PREFIX_LENGTH..=SESSION_FILE_MAX_LENGTH).contains(&value.len())
        && value.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-')
}

fn canonical_source_path(path: &Path) -> Result<PathBuf> {
    if path.symlink_metadata()?.file_type().is_symlink() {
        bail!(
            "Qwen Code session source must not be a symlink: {}",
            path.display()
        );
    }
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "Qwen Code session source does not exist: {}",
            path.display()
        )
    })?;
    let runtime_base = qwen_runtime_base()
        .and_then(|base| base.canonicalize().ok())
        .with_context(|| "Qwen Code runtime root is not configured")?;
    let relative = canonical.strip_prefix(&runtime_base).with_context(|| {
        format!(
            "Qwen Code session source is outside the configured runtime root {}: {}",
            runtime_base.display(),
            canonical.display()
        )
    })?;
    let file_name = canonical.file_name().and_then(|name| name.to_str());
    let session_id = file_name
        .and_then(|name| name.strip_suffix(".jsonl"))
        .filter(|name| is_session_id(name));
    let is_current_layout = relative
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        == Some("chats")
        && relative
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("projects");
    if !canonical.is_file()
        || canonical
            .extension()
            .and_then(|extension| extension.to_str())
            != Some(SESSION_FILE_EXTENSION)
        || session_id.is_none()
        || !is_current_layout
    {
        bail!(
            "Not a Qwen Code current session JSONL source: {}",
            canonical.display()
        );
    }
    Ok(canonical)
}

fn parse_jsonl_session(path: &Path) -> Result<ParsedQwenSession> {
    let raw = std::fs::read_to_string(path)?;
    let filename_session_id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|value| is_session_id(value))
        .ok_or_else(|| anyhow::anyhow!("Invalid Qwen Code session filename: {}", path.display()))?;
    let mut records = Vec::new();
    let mut reader_issues = Vec::new();

    for (line_index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let line_number = line_index + 1;
        let mut values = serde_json::Deserializer::from_str(line).into_iter::<Value>();
        while let Some(value) = values.next() {
            let record = match value {
                Ok(record) => record,
                Err(error) => {
                    reader_issues.push(QwenReaderIssue {
                        line_number,
                        reason: format!("malformed JSON skipped: {error}"),
                        raw: reader_issue_raw(line),
                    });
                    break;
                }
            };
            if !record.is_object() {
                reader_issues.push(QwenReaderIssue {
                    line_number,
                    reason: "non-object JSON value skipped".to_string(),
                    raw: reader_issue_raw(line),
                });
                continue;
            }
            let record_session_id = record
                .get("sessionId")
                .and_then(Value::as_str)
                .filter(|value| is_session_id(value))
                .ok_or_else(|| {
                    anyhow::anyhow!("Qwen Code JSONL record {line_number} has no valid sessionId")
                })?;
            if record_session_id != filename_session_id {
                bail!(
                    "Qwen Code sessionId mismatch in {}: expected {}, got {}",
                    path.display(),
                    filename_session_id,
                    record_session_id
                );
            }
            records.push(record);
        }
    }

    if records.is_empty() {
        bail!(
            "Qwen Code JSONL source has no valid records: {}",
            path.display()
        );
    }

    let project_dir = records.iter().find_map(|record| {
        record
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|cwd| !cwd.trim().is_empty())
            .map(ToOwned::to_owned)
    });
    Ok(ParsedQwenSession {
        session_id: filename_session_id.to_string(),
        project_dir,
        records,
        reader_issues,
    })
}

fn reader_issue_raw(line: &str) -> String {
    const MAX_CHARS: usize = 256;
    let mut raw = line.chars().take(MAX_CHARS).collect::<String>();
    if line.chars().count() > MAX_CHARS {
        raw.push('…');
    }
    raw
}

fn summary_from_parsed(path: &Path, parsed: &ParsedQwenSession) -> ProviderSessionSummary {
    let last_active_at = parsed
        .records
        .iter()
        .filter_map(record_datetime)
        .map(|datetime| datetime.timestamp_millis())
        .max()
        .or_else(|| path_mtime_ms(path));
    ProviderSessionSummary {
        session_id: parsed.session_id.clone(),
        title: session_title(&parsed.records),
        project_dir: parsed.project_dir.clone(),
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
    }
}

fn import_parsed_session(path: &Path, parsed: ParsedQwenSession) -> Result<ImportedSession> {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    for issue in &parsed.reader_issues {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: MappingDisposition::Dropped,
            code: "qwen_jsonl_record_skipped".to_string(),
            message: issue.reason.clone(),
            path: Some(format!("line:{}", issue.line_number)),
            raw: Some(Value::String(issue.raw.clone())),
        });
    }
    let events = parsed
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| event_from_record(index, record, &mut report))
        .collect();
    let created_at = parsed
        .records
        .iter()
        .find_map(record_datetime)
        .or_else(|| path_mtime_datetime(path))
        .unwrap_or_else(Utc::now);
    let last_active_at = parsed
        .records
        .iter()
        .filter_map(record_datetime)
        .max()
        .or_else(|| path_mtime_datetime(path))
        .unwrap_or(created_at);
    let mut extensions = BTreeMap::new();
    extensions.insert(
        "qwen_code_session".to_string(),
        Value::Array(parsed.records.clone()),
    );

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: parsed.session_id.clone(),
                source_title: session_title(&parsed.records),
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: parsed.session_id,
                    source_path: Some(path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: parsed.project_dir,
                created_at: Some(created_at),
                last_active_at: Some(last_active_at),
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn event_from_record(index: usize, record: &Value, report: &mut MappingReport) -> SessionEvent {
    let record_type = record
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let original_id = record
        .get("uuid")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let event_id = original_id
        .clone()
        .unwrap_or_else(|| format!("qwen:record:{index}"));
    let blocks = record_blocks(record, report, index);
    let role = match record_type.as_str() {
        "user" => EventRole::User,
        "assistant" => EventRole::Assistant,
        "tool_result" => EventRole::Tool,
        "system" => EventRole::System,
        _ => EventRole::Unknown,
    };
    let kind = event_kind(&blocks);
    SessionEvent {
        id: event_id,
        kind,
        role,
        timestamp: record_datetime(record).unwrap_or_else(Utc::now),
        links: EventLinks {
            parent_event_id: record
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            provider_parent_id: record
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            provider_turn_id: None,
            turn_index: Some(index as u32),
            turn_boundary: None,
            related_event_ids: Vec::new(),
        },
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id,
                original_role: Some(record_type),
                phase: record
                    .get("subtype")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            },
            model: record
                .get("model")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            usage: usage_from_record(record),
            fidelity: MappingDisposition::Normalized,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("raw_record".to_string(), record.clone());
                ext
            },
        },
    }
}

fn record_blocks(record: &Value, report: &mut MappingReport, index: usize) -> Vec<EventBlock> {
    let mut blocks = Vec::new();
    let message = record.get("message");
    if let Some(parts) = message
        .and_then(|message| message.get("parts"))
        .and_then(Value::as_array)
    {
        for (part_index, part) in parts.iter().enumerate() {
            if let Some(text) = part
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                if part
                    .get("thought")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    blocks.push(EventBlock::Thinking {
                        text: text.to_string(),
                        signature: part
                            .get("thoughtSignature")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                    });
                } else {
                    blocks.push(EventBlock::Text {
                        text: text.to_string(),
                    });
                }
            }
            if let Some(function_call) = part.get("functionCall") {
                let call_id = function_call
                    .get("id")
                    .or_else(|| function_call.get("name"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("qwen:tool:{index}:{part_index}"));
                blocks.push(EventBlock::ToolCall {
                    tool_call_id: call_id,
                    name: function_call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string(),
                    input: function_call.get("args").cloned(),
                });
            }
            if let Some(function_response) = part.get("functionResponse") {
                let call_id = function_response
                    .get("id")
                    .or_else(|| function_response.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("qwen:tool:unknown")
                    .to_string();
                blocks.push(EventBlock::ToolResult {
                    tool_call_id: call_id,
                    content: function_response
                        .get("response")
                        .map(extract_text)
                        .unwrap_or_default(),
                    is_error: function_response
                        .get("response")
                        .and_then(|response| response.get("error"))
                        .is_some(),
                });
            }
        }
    }

    if record.get("type").and_then(Value::as_str) == Some("tool_result") {
        if let Some(tool_call_result) = record.get("toolCallResult") {
            let call_id = tool_call_result
                .get("callId")
                .or_else(|| tool_call_result.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("qwen:tool:unknown")
                .to_string();
            blocks.push(EventBlock::ToolResult {
                tool_call_id: call_id,
                content: tool_call_result
                    .get("result")
                    .or_else(|| tool_call_result.get("output"))
                    .map(extract_text)
                    .unwrap_or_else(|| extract_text(tool_call_result)),
                is_error: tool_call_result
                    .get("status")
                    .and_then(Value::as_str)
                    .map(|status| status.eq_ignore_ascii_case("error"))
                    .unwrap_or(false),
            });
        }
    }

    if blocks.is_empty() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: MappingDisposition::Preserved,
            code: "qwen_record_payload_preserved".to_string(),
            message:
                "Preserved a Qwen Code record whose content shape has no canonical block mapping."
                    .to_string(),
            path: Some(format!("records:{index}")),
            raw: Some(record.clone()),
        });
        blocks.push(EventBlock::ProviderPayload {
            kind: "qwen_code_record".to_string(),
            payload: record.clone(),
        });
    }
    blocks
}

fn event_kind(blocks: &[EventBlock]) -> SessionEventKind {
    if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolResult { .. }))
    {
        SessionEventKind::ToolResult
    } else if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolCall { .. }))
    {
        SessionEventKind::ToolCall
    } else if blocks
        .iter()
        .all(|block| matches!(block, EventBlock::ProviderPayload { .. }))
    {
        SessionEventKind::Unknown
    } else {
        SessionEventKind::Message
    }
}

fn usage_from_record(record: &Value) -> Option<UsageStats> {
    let usage = record.get("usageMetadata")?;
    Some(UsageStats {
        input_tokens: usage.get("promptTokenCount").and_then(Value::as_u64),
        output_tokens: usage.get("candidatesTokenCount").and_then(Value::as_u64),
        total_tokens: usage.get("totalTokenCount").and_then(Value::as_u64),
    })
}

fn session_title(records: &[Value]) -> Option<String> {
    records
        .iter()
        .filter_map(|record| {
            (record.get("type").and_then(Value::as_str) == Some("system")
                && record.get("subtype").and_then(Value::as_str) == Some("custom_title"))
            .then(|| {
                record
                    .get("systemPayload")
                    .and_then(|payload| payload.get("customTitle"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .flatten()
        })
        .last()
        .map(|title| truncate_summary(&title, 80))
        .filter(|title| !title.is_empty())
        .or_else(|| {
            records.iter().find_map(|record| {
                if record.get("type").and_then(Value::as_str) != Some("user") {
                    return None;
                }
                let text = record
                    .get("message")
                    .and_then(|message| message.get("parts"))
                    .map(extract_text)
                    .unwrap_or_default();
                let title = truncate_summary(&text, 80);
                (!title.is_empty()).then_some(title)
            })
        })
}

fn record_datetime(record: &Value) -> Option<DateTime<Utc>> {
    record
        .get("timestamp")
        .and_then(parse_timestamp_to_ms)
        .and_then(DateTime::<Utc>::from_timestamp_millis)
}

fn path_mtime_datetime(path: &Path) -> Option<DateTime<Utc>> {
    path_mtime_ms(path).and_then(DateTime::<Utc>::from_timestamp_millis)
}

fn path_mtime_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|datetime| datetime.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    static TEST_QWEN_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    struct TestQwenRuntimeGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestQwenRuntimeGuard {
        fn new(runtime: &Path) -> Self {
            let lock = TEST_QWEN_LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *TEST_QWEN_RUNTIME_BASE
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(runtime.to_path_buf());
            Self { _lock: lock }
        }
    }

    impl Drop for TestQwenRuntimeGuard {
        fn drop(&mut self) {
            *TEST_QWEN_RUNTIME_BASE
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }

    fn session_file(runtime: &Path, session_id: &str) -> PathBuf {
        let project = runtime.join("projects").join("-workspace").join("chats");
        fs::create_dir_all(&project).unwrap();
        project.join(format!("{session_id}.jsonl"))
    }

    fn write_fixture(path: &Path, session_id: &str) {
        // The base ChatRecord shape mirrors QwenLM/qwen-code's
        // session-transcript-reader.test.ts at c56ae42f; provider-specific
        // thinking/tool/title/usage records extend that fixed source contract.
        let records = vec![
            json!({
                "uuid": "u1",
                "parentUuid": null,
                "sessionId": session_id,
                "timestamp": "2026-07-17T00:00:00Z",
                "type": "user",
                "cwd": "/workspace",
                "version": "0.1.0",
                "message": {"role": "user", "parts": [{"text": "inspect the repo"}]}
            }),
            json!({
                "uuid": "a1",
                "parentUuid": "u1",
                "sessionId": session_id,
                "timestamp": "2026-07-17T00:00:01Z",
                "type": "assistant",
                "cwd": "/workspace",
                "version": "0.1.0",
                "model": "qwen-test",
                "usageMetadata": {"promptTokenCount": 3, "candidatesTokenCount": 4, "totalTokenCount": 7},
                "message": {"role": "model", "parts": [
                    {"thought": true, "text": "reasoning"},
                    {"functionCall": {"id": "call-1", "name": "read_file", "args": {"path": "README.md"}}}
                ]}
            }),
            json!({
                "uuid": "t1",
                "parentUuid": "a1",
                "sessionId": session_id,
                "timestamp": "2026-07-17T00:00:02Z",
                "type": "tool_result",
                "cwd": "/workspace",
                "version": "0.1.0",
                "toolCallResult": {"callId": "call-1", "result": "contents"}
            }),
            json!({
                "uuid": "title-1",
                "parentUuid": "t1",
                "sessionId": session_id,
                "timestamp": "2026-07-17T00:00:03Z",
                "type": "system",
                "subtype": "custom_title",
                "cwd": "/workspace",
                "version": "0.1.0",
                "systemPayload": {"customTitle": "Repository inspection"}
            }),
        ];
        let body = records
            .into_iter()
            .map(|record| serde_json::to_string(&record).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, format!("{body}\n")).unwrap();
    }

    #[test]
    fn scans_only_current_direct_chat_sources() {
        let runtime = tempdir().unwrap();
        let _guard = TestQwenRuntimeGuard::new(runtime.path());
        let session_id = "11111111-1111-1111-1111-111111111111";
        let path = session_file(runtime.path(), session_id);
        write_fixture(&path, session_id);
        fs::write(
            runtime
                .path()
                .join("projects/-workspace/chats/22222222-2222-2222-2222-222222222222.jsonl"),
            "{malformed\n",
        )
        .unwrap();
        fs::create_dir_all(runtime.path().join("projects/-workspace/chats/archive")).unwrap();
        fs::write(
            runtime.path().join(
                "projects/-workspace/chats/archive/22222222-2222-2222-2222-222222222222.jsonl",
            ),
            "{}\n",
        )
        .unwrap();
        fs::write(
            runtime
                .path()
                .join("projects/-workspace/chats/not-a-session.jsonl"),
            "{}\n",
        )
        .unwrap();

        let sessions = QwenProvider.scan_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, session_id);
        assert_eq!(sessions[0].title.as_deref(), Some("Repository inspection"));
        assert_eq!(sessions[0].project_dir.as_deref(), Some("/workspace"));
    }

    #[test]
    fn imports_qwen_chat_records_and_preserves_raw_source() {
        let runtime = tempdir().unwrap();
        let _guard = TestQwenRuntimeGuard::new(runtime.path());
        let session_id = "33333333-3333-3333-3333-333333333333";
        let path = session_file(runtime.path(), session_id);
        write_fixture(&path, session_id);
        let imported = QwenProvider.import_session(path.to_str().unwrap()).unwrap();

        assert_eq!(imported.session.identity.canonical_id, session_id);
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Repository inspection")
        );
        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some("/workspace")
        );
        assert_eq!(imported.session.events.len(), 4);
        assert!(matches!(
            imported.session.events[1].blocks[0],
            EventBlock::Thinking { .. }
        ));
        assert!(matches!(
            imported.session.events[1].blocks[1],
            EventBlock::ToolCall { .. }
        ));
        assert!(matches!(
            imported.session.events[2].blocks[0],
            EventBlock::ToolResult { .. }
        ));
        assert_eq!(
            imported.session.extensions["qwen_code_session"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(imported.report.overall, MappingDisposition::Preserved);
    }

    #[test]
    fn tolerates_concatenated_objects_and_reports_skipped_jsonl_content() {
        let runtime = tempdir().unwrap();
        let _guard = TestQwenRuntimeGuard::new(runtime.path());
        let session_id = "55555555-5555-5555-5555-555555555555";
        let path = session_file(runtime.path(), session_id);
        let first = json!({
            "uuid": "u1",
            "sessionId": session_id,
            "timestamp": "2026-07-17T00:00:00Z",
            "type": "user",
            "cwd": "/workspace",
            "message": {"parts": [{"text": "first"}]}
        });
        let second = json!({
            "uuid": "a1",
            "parentUuid": "u1",
            "sessionId": session_id,
            "timestamp": "2026-07-17T00:00:01Z",
            "type": "assistant",
            "cwd": "/workspace",
            "message": {"parts": [{"text": "second"}]}
        });
        fs::write(
            &path,
            format!(
                "{}{}\nnull\n{{malformed-tail\n\n",
                serde_json::to_string(&first).unwrap(),
                serde_json::to_string(&second).unwrap()
            ),
        )
        .unwrap();

        let imported = QwenProvider.import_session(path.to_str().unwrap()).unwrap();
        assert_eq!(imported.session.events.len(), 2);
        assert_eq!(
            imported.session.extensions["qwen_code_session"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(imported.report.overall, MappingDisposition::Dropped);
        assert_eq!(
            imported
                .report
                .issues
                .iter()
                .filter(|issue| issue.code == "qwen_jsonl_record_skipped")
                .count(),
            2
        );
    }

    #[test]
    fn rejects_duplicate_session_identity_across_project_sources() {
        let runtime = tempdir().unwrap();
        let _guard = TestQwenRuntimeGuard::new(runtime.path());
        let session_id = "66666666-6666-6666-6666-666666666666";
        write_fixture(&session_file(runtime.path(), session_id), session_id);
        let second_path = runtime
            .path()
            .join("projects/other-project/chats")
            .join(format!("{session_id}.jsonl"));
        fs::create_dir_all(second_path.parent().unwrap()).unwrap();
        write_fixture(&second_path, session_id);

        let error = QwenProvider.scan_sessions().unwrap_err().to_string();
        assert!(error.contains("Ambiguous Qwen Code session identity"));
        assert!(error.contains("-workspace"));
        assert!(error.contains("other-project"));
    }

    #[test]
    fn rejects_valid_current_layout_outside_configured_runtime_root() {
        let runtime = tempdir().unwrap();
        let outside_runtime = tempdir().unwrap();
        let _guard = TestQwenRuntimeGuard::new(runtime.path());
        let session_id = "77777777-7777-7777-7777-777777777777";
        let path = session_file(outside_runtime.path(), session_id);
        write_fixture(&path, session_id);

        let import_error = QwenProvider
            .import_session(path.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(import_error.contains("outside the configured runtime root"));
        let fingerprint_error = QwenProvider
            .session_source_fingerprint(path.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(fingerprint_error.contains("outside the configured runtime root"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_current_source() {
        let runtime = tempdir().unwrap();
        let _guard = TestQwenRuntimeGuard::new(runtime.path());
        let session_id = "88888888-8888-8888-8888-888888888888";
        let target = session_file(runtime.path(), session_id);
        write_fixture(&target, session_id);
        let link = runtime
            .path()
            .join("projects/-workspace/chats/99999999-9999-9999-9999-999999999999.jsonl");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let error = QwenProvider
            .import_session(link.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains("must not be a symlink"));
    }

    #[test]
    fn rejects_record_session_identity_mismatch() {
        let runtime = tempdir().unwrap();
        let _guard = TestQwenRuntimeGuard::new(runtime.path());
        let session_id = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let path = session_file(runtime.path(), session_id);
        fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&json!({
                    "uuid": "mismatch-1",
                    "sessionId": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                    "type": "user",
                    "message": {"parts": [{"text": "wrong identity"}]}
                }))
                .unwrap()
            ),
        )
        .unwrap();

        let error = QwenProvider
            .import_session(path.to_str().unwrap())
            .unwrap_err();
        assert!(
            format!("{error:?}").contains("sessionId mismatch"),
            "{error:?}"
        );
    }

    #[test]
    fn fingerprint_changes_with_source_bytes_and_rejects_non_current_locator() {
        let runtime = tempdir().unwrap();
        let _guard = TestQwenRuntimeGuard::new(runtime.path());
        let session_id = "44444444-4444-4444-4444-444444444444";
        let path = session_file(runtime.path(), session_id);
        write_fixture(&path, session_id);
        let first = QwenProvider
            .session_source_fingerprint(path.to_str().unwrap())
            .unwrap()
            .unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let appended = json!({
            "uuid": "growth-1",
            "parentUuid": "title-1",
            "sessionId": session_id,
            "timestamp": "2026-07-17T00:00:04Z",
            "type": "assistant",
            "cwd": "/workspace",
            "message": {"parts": [{"text": "appended"}]}
        });
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(format!("{}\n", serde_json::to_string(&appended).unwrap()).as_bytes())
            .unwrap();
        let second = QwenProvider
            .session_source_fingerprint(path.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_ne!(first.value, second.value);
        let imported_after_growth = QwenProvider.import_session(path.to_str().unwrap()).unwrap();
        assert_eq!(imported_after_growth.session.events.len(), 5);
        let archive_path = runtime
            .path()
            .join("projects/-workspace/chats/archive/x.jsonl");
        fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
        fs::write(&archive_path, b"{}\n").unwrap();
        assert!(QwenProvider
            .session_source_fingerprint(archive_path.to_str().unwrap())
            .is_err());
    }
}
