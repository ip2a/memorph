pub mod adapter;
pub mod hook;
mod management;

use crate::canonical::{
    Block, Context, Event, EventKind, Fidelity, Identity, ImportedSession, Links, MappingDirection,
    MappingIssue, MappingIssueLevel, MappingReport, Metadata, Provenance, ProviderRef, Role,
    Schema, Session, Source, Usage,
};
use crate::provider::{
    PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionBackup, ProviderSessionSummary,
    ProviderSourceFingerprint, ProviderSourceMutation, ProviderWriteRisk, ResumeQuality,
    ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::utils::{extract_text, parse_timestamp_to_ms, truncate_summary};
use anyhow::{bail, Context as _, Result};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct GeminiProvider;

const PROVIDER_ID: &str = "gemini";
const SESSION_FILE_PREFIX: &str = "session-";
const SESSION_FILE_EXTENSION: &str = "jsonl";

#[cfg(test)]
static TEST_GEMINI_HOME: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_GEMINI_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

fn fail_gemini_mutation_after_write(_mutation: ProviderSourceMutation) -> Result<()> {
    #[cfg(test)]
    {
        let configured = TEST_GEMINI_MUTATION_FAILURE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if configured.as_ref() == Some(&_mutation) {
            anyhow::bail!("configured Gemini mutation failure after native write");
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ParsedGeminiSession {
    metadata: Map<String, Value>,
    messages: Vec<Value>,
    raw_records: Vec<Value>,
}

impl Provider for GeminiProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Gemini CLI"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: false,
            delete: true,
            rename: false,
            resume: true,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Jsonl,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Preserved),
                tool_result: Some(Fidelity::Preserved),
                patch: Some(Fidelity::Unsupported),
                image: Some(Fidelity::Unsupported),
                file: Some(Fidelity::Unsupported),
                compressed: Some(Fidelity::Unsupported),
                provider_payload: Some(Fidelity::Preserved),
            },
            export_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Unsupported),
                thinking: Some(Fidelity::Unsupported),
                tool_call: Some(Fidelity::Unsupported),
                tool_result: Some(Fidelity::Unsupported),
                patch: Some(Fidelity::Unsupported),
                image: Some(Fidelity::Unsupported),
                file: Some(Fidelity::Unsupported),
                compressed: Some(Fidelity::Unsupported),
                provider_payload: Some(Fidelity::Unsupported),
            },
            resume_quality: ResumeQuality::Native,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::Medium,
                multiple_files: true,
                sqlite: false,
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
                runtime_endpoint: false,
                session_activity: false,
            },
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let mut sessions = Vec::new();
        let mut seen_session_ids = HashSet::new();

        for root in gemini_roots() {
            let mut project_dirs = read_child_directories(&root)?;
            project_dirs.sort();
            for project_dir in project_dirs {
                let chats_dir = project_dir.join("chats");
                let mut files = read_session_files(&chats_dir)?;
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
                    let Some(session_id) = metadata_string(&parsed.metadata, "sessionId") else {
                        continue;
                    };
                    if !seen_session_ids.insert(session_id.clone()) {
                        continue;
                    }
                    sessions.push(summary_from_parsed(&path, &session_id, &parsed));
                }
            }
        }

        sessions.sort_by_key(|session| std::cmp::Reverse(session.last_active_at.unwrap_or(0)));
        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let path = canonical_source_path(Path::new(source_path))?;
        let parsed = parse_jsonl_session(&path).with_context(|| {
            format!(
                "Failed to read Gemini CLI JSONL session: {}",
                path.display()
            )
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
            value: format!("gemini-jsonl-v1:{digest:x}"),
        }))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        management::delete_session(session_id)
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        management::create_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        management::restore_session_backup(backup)
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("gemini --resume {session_id}"))
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        gemini_roots()
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

fn gemini_roots() -> Vec<PathBuf> {
    gemini_home_dir()
        .map(|home| vec![home.join(".gemini").join("tmp")])
        .unwrap_or_default()
}

fn gemini_home_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(home) = TEST_GEMINI_HOME
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("Gemini test home lock poisoned")
        .clone()
    {
        return Some(home);
    }

    std::env::var_os("GEMINI_CLI_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

fn canonical_source_path(path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("Gemini session source does not exist: {}", path.display()))?;
    if !canonical.is_file()
        || canonical
            .extension()
            .and_then(|extension| extension.to_str())
            != Some(SESSION_FILE_EXTENSION)
        || !canonical
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with(SESSION_FILE_PREFIX))
            .unwrap_or(false)
    {
        bail!(
            "Not a Gemini CLI session JSONL source: {}",
            canonical.display()
        );
    }
    Ok(canonical)
}

fn read_child_directories(root: &Path) -> Result<Vec<PathBuf>> {
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

fn read_session_files(chats_dir: &Path) -> Result<Vec<PathBuf>> {
    if !chats_dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(chats_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_file() && name.starts_with(SESSION_FILE_PREFIX) && name.ends_with(".jsonl")
        {
            files.push(entry.path());
        }
    }
    Ok(files)
}

fn parse_jsonl_session(path: &Path) -> Result<ParsedGeminiSession> {
    let raw = std::fs::read_to_string(path)?;
    let mut metadata = Map::new();
    let mut messages = Vec::new();
    let mut raw_records = Vec::new();

    for (line_index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let is_first_record = raw_records.is_empty();
        let record: Value = serde_json::from_str(line).with_context(|| {
            format!(
                "Failed to parse Gemini JSONL record {} in {}",
                line_index + 1,
                path.display()
            )
        })?;
        raw_records.push(record.clone());

        if is_first_record && is_session_metadata(&record) {
            metadata = record.as_object().cloned().unwrap_or_default();
            if let Some(initial_messages) = metadata.get("messages").and_then(Value::as_array) {
                messages = initial_messages.clone();
            }
            continue;
        }

        if let Some(rewind_id) = record.get("$rewindTo").and_then(Value::as_str) {
            rewind_messages(&mut messages, rewind_id);
            continue;
        }

        if let Some(updates) = record.get("$set").and_then(Value::as_object) {
            if let Some(updated_messages) = updates.get("messages").and_then(Value::as_array) {
                messages = updated_messages.clone();
            }
            for (key, value) in updates {
                metadata.insert(key.clone(), value.clone());
            }
            continue;
        }

        if is_message_record(&record) {
            upsert_message(&mut messages, record);
        }
    }

    if metadata_string(&metadata, "sessionId").is_none() {
        bail!("Gemini JSONL source has no sessionId: {}", path.display());
    }

    Ok(ParsedGeminiSession {
        metadata,
        messages,
        raw_records,
    })
}

fn is_session_metadata(value: &Value) -> bool {
    value.get("sessionId").and_then(Value::as_str).is_some()
        && (value.get("projectHash").is_some()
            || value.get("startTime").is_some()
            || value.get("messages").is_some())
}

fn is_message_record(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some("user" | "info" | "error" | "warning" | "gemini")
    ) && (value.get("id").is_some() || value.get("content").is_some())
}

fn upsert_message(messages: &mut Vec<Value>, message: Value) {
    let Some(message_id) = message.get("id").and_then(Value::as_str) else {
        messages.push(message);
        return;
    };
    if let Some(existing) = messages
        .iter_mut()
        .find(|existing| existing.get("id").and_then(Value::as_str) == Some(message_id))
    {
        *existing = message;
    } else {
        messages.push(message);
    }
}

fn rewind_messages(messages: &mut Vec<Value>, rewind_id: &str) {
    let Some(index) = messages
        .iter()
        .position(|message| message.get("id").and_then(Value::as_str) == Some(rewind_id))
    else {
        messages.clear();
        return;
    };
    messages.truncate(index);
}

fn summary_from_parsed(
    path: &Path,
    session_id: &str,
    parsed: &ParsedGeminiSession,
) -> ProviderSessionSummary {
    ProviderSessionSummary {
        session_id: session_id.to_string(),
        title: session_title(parsed),
        project_dir: None,
        created_at: None,
        last_active_at: metadata_timestamp(&parsed.metadata, "lastUpdated")
            .or_else(|| path_mtime_ms(path)),
        source_path: Some(path.to_string_lossy().to_string()),
    }
}

fn import_parsed_session(path: &Path, parsed: ParsedGeminiSession) -> Result<ImportedSession> {
    let session_id = metadata_string(&parsed.metadata, "sessionId")
        .context("Gemini session metadata has no sessionId")?;
    let created_at = metadata_datetime(&parsed.metadata, "startTime")
        .or_else(|| path_mtime_datetime(path))
        .unwrap_or_else(Utc::now);
    let last_active_at = metadata_datetime(&parsed.metadata, "lastUpdated")
        .or_else(|| path_mtime_datetime(path))
        .unwrap_or(created_at);
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let events = parsed
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| event_from_message(index, message, created_at, &mut report))
        .collect();

    for (index, record) in parsed.raw_records.iter().enumerate() {
        if !is_session_metadata(record)
            && record.get("$rewindTo").is_none()
            && record.get("$set").is_none()
            && !is_message_record(record)
        {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Preserved,
                code: "gemini_unknown_jsonl_record".to_string(),
                message: "Preserved an unrecognized Gemini JSONL record in the provider extension."
                    .to_string(),
                path: Some(format!("records:{index}")),
                raw: Some(record.clone()),
            });
        }
    }

    let mut extensions = std::collections::BTreeMap::new();
    extensions.insert(
        "gemini_session".to_string(),
        serde_json::json!({
            "metadata": parsed.metadata,
            "records": parsed.raw_records,
        }),
    );

    Ok(ImportedSession {
        session: Session {
            schema: Schema::default(),
            identity: Identity {
                canonical_id: session_id.clone(),
                source_title: session_title(&parsed),
            },
            provenance: Provenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id,
                    source_path: Some(path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: Context {
                workspace_dir: None,
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

fn event_from_message(
    index: usize,
    message: &Value,
    fallback_timestamp: DateTime<Utc>,
    report: &mut MappingReport,
) -> Event {
    let role_raw = message
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let role = match role_raw.as_str() {
        "user" => Role::User,
        "gemini" => Role::Assistant,
        "info" | "error" | "warning" => Role::System,
        _ => Role::Unknown,
    };
    let original_id = message
        .get("id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let event_id = original_id
        .clone()
        .unwrap_or_else(|| format!("gemini:message:{index}"));
    let blocks = message_blocks(message, report, index);
    let kind = event_kind(&blocks);

    Event {
        id: event_id,
        kind,
        role,
        timestamp: message_datetime(message, "timestamp").unwrap_or(fallback_timestamp),
        links: Links {
            parent_event_id: None,
            provider_parent_id: None,
            provider_turn_id: None,
            turn_index: Some(index as u32),
            turn_boundary: None,
            related_event_ids: Vec::new(),
        },
        blocks,
        metadata: Metadata {
            source: Source {
                provider_id: PROVIDER_ID.to_string(),
                original_id,
                original_role: Some(role_raw),
                phase: None,
            },
            model: message
                .get("model")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            usage: usage_from_message(message),
            fidelity: Fidelity::Normalized,
            provider_ext: {
                let mut ext = std::collections::BTreeMap::new();
                ext.insert("raw_message".to_string(), message.clone());
                ext
            },
        },
    }
}

fn message_blocks(message: &Value, report: &mut MappingReport, index: usize) -> Vec<Block> {
    let mut blocks = Vec::new();
    if let Some(content) = message.get("content") {
        let text = extract_text(content);
        if !text.trim().is_empty() {
            blocks.push(Block::Text { text });
        }
    }

    if let Some(thoughts) = message.get("thoughts").and_then(Value::as_array) {
        for thought in thoughts {
            let text = thought
                .get("subject")
                .or_else(|| thought.get("description"))
                .or_else(|| thought.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !text.trim().is_empty() {
                blocks.push(Block::Thinking {
                    text: text.to_string(),
                    signature: None,
                });
            }
        }
    }

    if let Some(tool_calls) = message.get("toolCalls").and_then(Value::as_array) {
        for (tool_index, tool_call) in tool_calls.iter().enumerate() {
            let tool_call_id = tool_call
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("gemini:tool:{index}:{tool_index}"));
            let name = tool_call
                .get("name")
                .or_else(|| tool_call.get("displayName"))
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            blocks.push(Block::ToolCall {
                tool_call_id: tool_call_id.clone(),
                name,
                input: tool_call.get("args").cloned(),
            });
            if let Some(result) = tool_call.get("result") {
                blocks.push(Block::ToolResult {
                    tool_call_id,
                    content: extract_text(result),
                    is_error: tool_call
                        .get("status")
                        .and_then(Value::as_str)
                        .map(|status| status.eq_ignore_ascii_case("error"))
                        .unwrap_or(false),
                });
            }
        }
    }

    if blocks.is_empty() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: Fidelity::Preserved,
            code: "gemini_message_payload_preserved".to_string(),
            message:
                "Preserved a Gemini message whose content shape has no canonical block mapping."
                    .to_string(),
            path: Some(format!("messages:{index}")),
            raw: Some(message.clone()),
        });
        blocks.push(Block::ProviderPayload {
            kind: "gemini_message".to_string(),
            payload: message.clone(),
        });
    }

    blocks
}

fn event_kind(blocks: &[Block]) -> EventKind {
    if blocks
        .iter()
        .any(|block| matches!(block, Block::ToolResult { .. }))
    {
        EventKind::ToolResult
    } else if blocks
        .iter()
        .any(|block| matches!(block, Block::ToolCall { .. }))
    {
        EventKind::ToolCall
    } else if blocks
        .iter()
        .all(|block| matches!(block, Block::ProviderPayload { .. }))
    {
        EventKind::Unknown
    } else {
        EventKind::Message
    }
}

fn usage_from_message(message: &Value) -> Option<Usage> {
    let tokens = message.get("tokens")?;
    Some(Usage {
        input_tokens: tokens.get("input").and_then(Value::as_u64),
        output_tokens: tokens.get("output").and_then(Value::as_u64),
        total_tokens: tokens.get("total").and_then(Value::as_u64),
    })
}

fn session_title(parsed: &ParsedGeminiSession) -> Option<String> {
    metadata_string(&parsed.metadata, "summary")
        .map(|summary| truncate_summary(&summary, 80))
        .filter(|summary| !summary.is_empty())
        .or_else(|| {
            parsed.messages.iter().find_map(|message| {
                if message.get("type").and_then(Value::as_str) != Some("user") {
                    return None;
                }
                let text = message.get("content").map(extract_text).unwrap_or_default();
                let title = truncate_summary(&text, 80);
                (!title.is_empty()).then_some(title)
            })
        })
}

fn metadata_string(metadata: &Map<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .filter(|value| !value.trim().is_empty())
}

fn metadata_timestamp(metadata: &Map<String, Value>, key: &str) -> Option<i64> {
    metadata.get(key).and_then(parse_timestamp_to_ms)
}

fn metadata_datetime(metadata: &Map<String, Value>, key: &str) -> Option<DateTime<Utc>> {
    metadata
        .get(key)
        .and_then(parse_timestamp_to_ms)
        .and_then(DateTime::<Utc>::from_timestamp_millis)
}

fn message_datetime(message: &Value, key: &str) -> Option<DateTime<Utc>> {
    message
        .get(key)
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
    use crate::storage::local_store;
    use rusqlite::{params, Connection};
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    static TEST_GEMINI_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    struct TestGeminiHomeGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestGeminiHomeGuard {
        fn new(home: &Path) -> Self {
            let lock = TEST_GEMINI_LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *TEST_GEMINI_HOME
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
                .expect("Gemini test home lock poisoned") = Some(home.to_path_buf());
            Self { _lock: lock }
        }
    }

    impl Drop for TestGeminiHomeGuard {
        fn drop(&mut self) {
            *TEST_GEMINI_HOME
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
                .expect("Gemini test home lock poisoned") = None;
        }
    }

    fn set_test_gemini_mutation_failure(mutation: Option<ProviderSourceMutation>) {
        *TEST_GEMINI_MUTATION_FAILURE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .expect("Gemini mutation failure lock poisoned") = mutation;
    }

    struct TestConfigHomeGuard;

    impl TestConfigHomeGuard {
        fn new(home: &Path) -> Self {
            crate::config::set_test_home_dir(home.to_path_buf());
            Self
        }
    }

    impl Drop for TestConfigHomeGuard {
        fn drop(&mut self) {
            crate::config::reset_test_home_dir();
        }
    }

    fn with_test_home<T>(home: &Path, action: impl FnOnce() -> T) -> T {
        let _guard = TestGeminiHomeGuard::new(home);
        action()
    }

    fn write_session(
        home: &Path,
        project_hash: &str,
        file_name: &str,
        records: &[Value],
    ) -> PathBuf {
        let chats = home
            .join(".gemini")
            .join("tmp")
            .join(project_hash)
            .join("chats");
        fs::create_dir_all(&chats).unwrap();
        let path = chats.join(file_name);
        let body = records
            .iter()
            .map(|record| serde_json::to_string(record).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{body}\n")).unwrap();
        path
    }

    fn metadata(session_id: &str) -> Value {
        serde_json::json!({
            "sessionId": session_id,
            "projectHash": "project-hash",
            "startTime": "2026-07-17T01:00:00Z",
            "lastUpdated": "2026-07-17T01:02:00Z",
            "messages": []
        })
    }

    #[test]
    fn scans_only_current_gemini_session_jsonl_files() {
        let home = tempdir().unwrap();
        let path = write_session(
            home.path(),
            "project-hash",
            "session-2026-07-17-01-00-abc12345.jsonl",
            &[metadata("gemini-session-1")],
        );
        fs::write(
            home.path()
                .join(".gemini")
                .join("tmp")
                .join("project-hash")
                .join("chats")
                .join("settings.json"),
            "{}",
        )
        .unwrap();

        let sessions = with_test_home(home.path(), || GeminiProvider.scan_sessions().unwrap());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "gemini-session-1");
        let canonical_path = path.canonicalize().unwrap();
        assert_eq!(
            sessions[0].source_path.as_deref(),
            Some(canonical_path.to_str().unwrap())
        );
    }

    #[test]
    fn replays_jsonl_rewind_and_metadata_update_semantics() {
        let home = tempdir().unwrap();
        let path = write_session(
            home.path(),
            "project-hash",
            "session-2026-07-17-01-00-abc12345.jsonl",
            &[
                metadata("gemini-session-2"),
                serde_json::json!({"id": "u1", "type": "user", "timestamp": "2026-07-17T01:00:01Z", "content": "first"}),
                serde_json::json!({"id": "g1", "type": "gemini", "timestamp": "2026-07-17T01:00:02Z", "content": [{"text": "answer"}], "thoughts": [{"text": "reason"}], "toolCalls": [{"id": "tc1", "name": "read_file", "args": {"path": "a.txt"}, "result": [{"text": "ok"}], "status": "success"}]}),
                serde_json::json!({"$rewindTo": "g1"}),
                serde_json::json!({"$set": {"summary": "updated summary"}}),
                serde_json::json!({"id": "u2", "type": "user", "timestamp": "2026-07-17T01:00:03Z", "content": "second"}),
            ],
        );

        let imported = with_test_home(home.path(), || {
            GeminiProvider
                .import_session(path.to_str().unwrap())
                .unwrap()
        });
        assert_eq!(imported.session.identity.canonical_id, "gemini-session-2");
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("updated summary")
        );
        assert_eq!(imported.session.events.len(), 2);
        assert!(imported.session.events.iter().all(|event| event.id != "g1"));
        assert!(imported
            .session
            .extensions
            .get("gemini_session")
            .and_then(|value| value.get("records"))
            .is_some());
    }

    #[test]
    fn imports_thinking_tool_calls_and_tool_results_without_sqlite_projection() {
        let home = tempdir().unwrap();
        let path = write_session(
            home.path(),
            "project-hash",
            "session-2026-07-17-01-00-abc12345.jsonl",
            &[
                metadata("gemini-session-3"),
                serde_json::json!({
                    "id": "g1",
                    "type": "gemini",
                    "timestamp": "2026-07-17T01:00:02Z",
                    "content": [{"text": "answer"}],
                    "thoughts": [{"text": "reason"}],
                    "tokens": {"input": 2, "output": 3, "total": 5},
                    "toolCalls": [{"id": "tc1", "name": "read_file", "args": {"path": "a.txt"}, "result": [{"text": "ok"}], "status": "success"}]
                }),
            ],
        );

        let imported = GeminiProvider
            .import_session(path.to_str().unwrap())
            .unwrap();
        let event = &imported.session.events[0];
        assert!(event
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Thinking { text, .. } if text == "reason")));
        assert!(event
            .blocks
            .iter()
            .any(|block| matches!(block, Block::ToolCall { name, .. } if name == "read_file")));
        assert!(event
            .blocks
            .iter()
            .any(|block| matches!(block, Block::ToolResult { content, .. } if content == "ok")));
        assert_eq!(event.metadata.usage.as_ref().unwrap().total_tokens, Some(5));
    }

    #[test]
    fn gemini_projection_bootstrap_is_bodyless_and_detail_reads_jsonl_source() -> Result<()> {
        let source_home = tempdir()?;
        let config_home = tempdir()?;
        let _gemini_guard = TestGeminiHomeGuard::new(source_home.path());
        let _config_guard = TestConfigHomeGuard::new(config_home.path());
        let session_id = "gemini-projection";
        let path = write_session(
            source_home.path(),
            "project-hash",
            "session-2026-07-17-01-00-abc12345.jsonl",
            &[
                serde_json::json!({
                    "sessionId": session_id,
                    "projectHash": "project-hash",
                    "startTime": "2026-07-17T01:00:00Z",
                    "lastUpdated": "2026-07-17T01:00:02Z",
                    "summary": "projection fixture",
                    "messages": []
                }),
                serde_json::json!({
                    "id": "u1",
                    "type": "user",
                    "timestamp": "2026-07-17T01:00:01Z",
                    "content": "first question"
                }),
                serde_json::json!({
                    "id": "g1",
                    "type": "gemini",
                    "timestamp": "2026-07-17T01:00:02Z",
                    "content": [{"text": "first answer"}]
                }),
            ],
        );
        let expected_locator = path.canonicalize()?.to_string_lossy().to_string();

        let first = crate::core::projection::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(first.scanned_providers, 1);
        assert_eq!(first.discovered_sessions, 1);
        assert_eq!(first.projected_sessions, 1);
        assert_eq!(first.unchanged_sessions, 0);
        assert!(first.failures.is_empty());

        let conn = local_store::open_database()?;
        let stored: (String, String, String, String, i64) = conn.query_row(
            "SELECT s.id, src.source_path, src.storage_shape, src.source_cursor, ss.stale
             FROM sessions s
             JOIN session_sources src ON src.id = s.primary_source_id
             JOIN session_snapshots ss ON ss.session_id = s.id
             WHERE s.provider_id = ?1 AND s.provider_session_id = ?2",
            params![PROVIDER_ID, session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
        let initial_fingerprint = GeminiProvider
            .session_source_fingerprint(&expected_locator)?
            .expect("fixture fingerprint");
        assert_eq!(stored.1, expected_locator);
        assert_eq!(stored.2, "jsonl");
        assert_eq!(stored.3, initial_fingerprint.value);
        assert_eq!(stored.4, 0);

        let body_table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(body_table_count, 0);
        drop(conn);

        let detail = crate::core::sessions::get_session_detail_view_page(
            PROVIDER_ID,
            session_id,
            0,
            Some(0),
        )?;
        assert!(detail.events.is_empty());
        assert!(detail.turns.is_empty());
        assert_eq!(detail.event_count, 2);
        assert_eq!(detail.message_count, 2);
        assert!(!detail.stale);
        assert_eq!(
            detail.source_path.as_deref(),
            Some(expected_locator.as_str())
        );
        assert_eq!(
            detail.projection_report.as_ref().unwrap().id,
            format!("source-read:{PROVIDER_ID}:{session_id}")
        );

        let conn = local_store::open_database()?;
        let cached_counts: (i64, i64, i64, i64) = conn.query_row(
            "SELECT event_count, message_count, turn_count, counts_complete
             FROM session_snapshots WHERE session_id = ?1",
            [stored.0.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(cached_counts, (2, 2, 1, 1));
        drop(conn);

        let second = crate::core::projection::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(second.projected_sessions, 0);
        assert_eq!(second.unchanged_sessions, 1);
        assert!(second.failures.is_empty());

        let mut file = fs::OpenOptions::new().append(true).open(&path)?;
        writeln!(
            file,
            "{}",
            serde_json::json!({"$set": {"lastUpdated": "2026-07-17T01:00:03Z"}})
        )?;
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "id": "u2",
                "type": "user",
                "timestamp": "2026-07-17T01:00:03Z",
                "content": "second question"
            })
        )?;
        file.flush()?;

        let changed_fingerprint = GeminiProvider
            .session_source_fingerprint(&expected_locator)?
            .expect("changed fixture fingerprint");
        assert_ne!(changed_fingerprint.value, initial_fingerprint.value);

        let stale_scan = crate::core::projection::refresh_projected_session_staleness(
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(stale_scan.checked_sources, 1);
        assert_eq!(stale_scan.stale_snapshots, 1);
        assert_eq!(stale_scan.missing_sources, 0);

        let stale_detail = crate::core::sessions::get_session_detail_view_page(
            PROVIDER_ID,
            session_id,
            0,
            Some(0),
        )?;
        assert!(stale_detail.stale);
        assert_eq!(stale_detail.event_count, 3);
        assert_eq!(stale_detail.message_count, 3);

        let refreshed = crate::core::projection::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(refreshed.projected_sessions, 1);
        assert_eq!(refreshed.unchanged_sessions, 0);
        assert!(refreshed.failures.is_empty());

        let fresh_detail = crate::core::sessions::get_session_detail_view_page(
            PROVIDER_ID,
            session_id,
            0,
            Some(0),
        )?;
        assert!(!fresh_detail.stale);
        assert_eq!(fresh_detail.event_count, 3);
        assert_eq!(fresh_detail.message_count, 3);

        let conn = local_store::open_database()?;
        let refreshed_counts: (i64, i64, i64, i64, i64) = conn.query_row(
            "SELECT event_count, message_count, turn_count, counts_complete, stale
             FROM session_snapshots WHERE session_id = ?1",
            [stored.0.as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
        assert_eq!(refreshed_counts, (3, 3, 2, 1, 0));
        drop(conn);

        fs::remove_file(&path)?;
        fs::write(
            path.with_extension("json"),
            "{\"sessionId\":\"gemini-projection\"}\n",
        )?;
        let missing_scan = crate::core::projection::refresh_projected_session_staleness(
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(missing_scan.checked_sources, 0);
        assert_eq!(missing_scan.missing_sources, 1);
        assert_eq!(missing_scan.stale_snapshots, 1);
        assert!(GeminiProvider
            .session_source_fingerprint(&expected_locator)?
            .is_none());
        let error = crate::core::sessions::get_session_detail_view_page(
            PROVIDER_ID,
            session_id,
            0,
            Some(0),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Session source is missing"));
        assert!(error.to_string().contains(&expected_locator));

        Ok(())
    }

    #[test]
    fn native_delete_and_backup_cover_main_subagent_and_sidecar_artifacts() -> Result<()> {
        let source_home = tempdir()?;
        let _gemini_guard = TestGeminiHomeGuard::new(source_home.path());
        let session_id = "gemini-delete-session";
        let source_path = write_session(
            source_home.path(),
            "project-hash",
            "session-2026-07-17-01-00-abc12345.jsonl",
            &[metadata(session_id)],
        );
        let temp_dir = source_path.parent().unwrap().parent().unwrap();
        let safe_session_id = "gemini-delete-session";
        let subagent_dir = temp_dir.join("chats").join(safe_session_id);
        let artifact_paths = [
            temp_dir.join("logs/session-gemini-delete-session.jsonl"),
            temp_dir.join("tool-outputs/session-gemini-delete-session/output.txt"),
            temp_dir.join("gemini-delete-session/metadata.json"),
            subagent_dir.join("agent-123.jsonl"),
            temp_dir.join("logs/session-agent-123.jsonl"),
            temp_dir.join("tool-outputs/session-agent-123/output.txt"),
            temp_dir.join("agent-123/state.json"),
        ];
        for artifact in &artifact_paths {
            fs::create_dir_all(artifact.parent().unwrap())?;
            fs::write(artifact, artifact.to_string_lossy().as_bytes())?;
        }

        let backup_root = tempdir()?;
        let backup = GeminiProvider.create_session_backup(
            ProviderSourceMutation::Delete,
            "gemini-delete-op",
            session_id,
            backup_root.path(),
        )?;
        assert_eq!(backup.format, "gemini-current-session-backup-v1");
        assert_eq!(
            backup.mime_type,
            "application/vnd.memorph.gemini-current-session-backup"
        );
        assert_eq!(
            backup.artifact_metadata["mutation"],
            serde_json::json!(ProviderSourceMutation::Delete)
        );
        assert_eq!(
            backup.restore_metadata["mutation"],
            serde_json::json!(ProviderSourceMutation::Delete)
        );
        assert!(backup.backup_path.join("metadata.json").is_file());

        GeminiProvider.delete_session(session_id)?;
        assert!(!source_path.exists());
        for artifact in &artifact_paths {
            assert!(
                !artifact.exists(),
                "artifact was not deleted: {}",
                artifact.display()
            );
        }

        GeminiProvider.restore_session_backup(&backup)?;
        assert!(source_path.is_file());
        for artifact in &artifact_paths {
            assert!(
                artifact.is_file(),
                "artifact was not restored: {}",
                artifact.display()
            );
        }
        Ok(())
    }

    #[test]
    fn delete_registered_backup_restore_round_trip() -> Result<()> {
        let source_home = tempdir()?;
        let config_home = tempdir()?;
        let _gemini_guard = TestGeminiHomeGuard::new(source_home.path());
        let _config_guard = TestConfigHomeGuard::new(config_home.path());
        let session_id = "gemini-api-restore";
        let source_path = write_session(
            source_home.path(),
            "project-hash",
            "session-2026-07-17-01-00-abc12345.jsonl",
            &[metadata(session_id)],
        );

        use crate::core::session_management::{list_registered_backups, restore_registered_backup};
        use crate::core::session_mutation::delete_session;
        use crate::storage::activity_store::ActivityActor;
        use crate::storage::artifact_store::BackupQuery;

        delete_session(PROVIDER_ID, session_id, ActivityActor::System)?;

        let views = list_registered_backups(BackupQuery {
            operation_id: None,
            provider_id: Some(PROVIDER_ID.to_string()),
            provider_session_id: Some(session_id.to_string()),
            restore_status: None,
            limit: None,
        })?;
        let backup_id = views
            .first()
            .context("did not return a registered Gemini backup")?
            .entry
            .backup
            .id
            .clone();

        restore_registered_backup(&backup_id, ActivityActor::System)?;

        assert!(source_path.is_file());
        Ok(())
    }

    #[test]
    fn native_delete_rejects_duplicate_and_short_session_identity() {
        let source_home = tempdir().unwrap();
        let _gemini_guard = TestGeminiHomeGuard::new(source_home.path());
        let session_id = "gemini-duplicate-session";
        write_session(
            source_home.path(),
            "project-a",
            "session-2026-07-17-01-00-aaaabbbb.jsonl",
            &[metadata(session_id)],
        );
        write_session(
            source_home.path(),
            "project-b",
            "session-2026-07-17-01-01-ccccdddd.jsonl",
            &[metadata(session_id)],
        );

        let duplicate = GeminiProvider.delete_session(session_id).unwrap_err();
        assert!(duplicate.to_string().contains("multiple current sources"));
        let short_id = GeminiProvider.delete_session("aaaabbbb").unwrap_err();
        assert!(short_id.to_string().contains("session not found"));
        let reserved = GeminiProvider.delete_session("chats").unwrap_err();
        assert!(reserved.to_string().contains("reserved Gemini session ID"));
        let traversal = GeminiProvider.delete_session("../outside").unwrap_err();
        assert!(traversal.to_string().contains("Invalid Gemini session id"));
    }

    #[test]
    fn core_delete_failure_restores_complete_gemini_source_backup() -> Result<()> {
        let source_home = tempdir()?;
        let config_home = tempdir()?;
        let _gemini_guard = TestGeminiHomeGuard::new(source_home.path());
        let _config_guard = TestConfigHomeGuard::new(config_home.path());
        let session_id = "gemini-delete-recovery";
        let source_path = write_session(
            source_home.path(),
            "project-hash",
            "session-2026-07-17-01-00-efgh5678.jsonl",
            &[metadata(session_id)],
        );
        let temp_dir = source_path.parent().unwrap().parent().unwrap();
        let artifact = temp_dir.join("logs/session-gemini-delete-recovery.jsonl");
        fs::create_dir_all(artifact.parent().unwrap())?;
        fs::write(&artifact, b"log")?;

        let mut artifact_conn = Connection::open_in_memory()?;
        local_store::configure_connection(&artifact_conn)?;
        local_store::apply_schema(&mut artifact_conn)?;
        let backup_root = tempdir()?;
        set_test_gemini_mutation_failure(Some(ProviderSourceMutation::Delete));
        let results = crate::core::session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["gemini-delete-recovery-op".to_string()],
            backup_root.path(),
            &mut artifact_conn,
        );
        set_test_gemini_mutation_failure(None);

        assert_eq!(results.len(), 1);
        let error = results[0].as_ref().unwrap_err().to_string();
        assert!(error.contains("Provider source was restored"));
        assert!(source_path.is_file());
        assert!(artifact.is_file());
        Ok(())
    }

    #[test]
    fn locator_is_required_and_missing_sources_have_no_fingerprint() {
        let home = tempdir().unwrap();
        let provider = GeminiProvider;
        let missing = home.path().join("session-missing.jsonl");
        assert!(provider
            .session_source_fingerprint(missing.to_str().unwrap())
            .unwrap()
            .is_none());
        assert!(provider.import_session("gemini-session-raw-id").is_err());
    }

    #[test]
    fn session_size_resolves_raw_session_id_through_native_scan() {
        let home = tempdir().unwrap();
        let path = write_session(
            home.path(),
            "project-hash",
            "session-2026-07-17-01-00-abc12345.jsonl",
            &[metadata("gemini-session-size")],
        );
        let expected = fs::metadata(path).unwrap().len();
        let actual = with_test_home(home.path(), || {
            GeminiProvider.session_size("gemini-session-size").unwrap()
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn fingerprint_changes_with_jsonl_content_and_capabilities_are_truthful() {
        let home = tempdir().unwrap();
        let path = write_session(
            home.path(),
            "project-hash",
            "session-2026-07-17-01-00-abc12345.jsonl",
            &[metadata("gemini-session-4")],
        );
        let provider = GeminiProvider;
        let before = provider
            .session_source_fingerprint(path.to_str().unwrap())
            .unwrap()
            .unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"id\":\"u1\",\"type\":\"user\",\"content\":\"changed\"}\n")
            .unwrap();
        let after = provider
            .session_source_fingerprint(path.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_ne!(before.value, after.value);
        assert_eq!(provider.capabilities().storage_shape, StorageShape::Jsonl);
        assert_eq!(
            provider.capabilities().resume_quality,
            ResumeQuality::Native
        );
        assert!(provider.capabilities().delete);
        assert!(!provider.capabilities().rename);
        assert!(provider.capabilities().backup_support.before_write);
        assert!(provider.capabilities().backup_support.restore);
    }
}
