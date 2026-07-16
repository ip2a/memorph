pub mod adapter;
pub mod hook;

use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance, TurnBoundary,
};
use crate::provider::{
    canonical_event_is_visible_message, canonical_event_visible_message_role,
    canonical_event_visible_message_text, canonical_export_result, canonical_session_title,
    canonical_visible_block_text, PageStrategy, Provider, ProviderBackupSupport,
    ProviderCapabilities, ProviderSessionBackup, ProviderSessionImportPage, ProviderSessionSummary,
    ProviderSourceFingerprint, ProviderSourceMutation, StorageShape, TurnQuality,
};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

mod backup;

pub struct KimiProvider;

const PROVIDER_ID: &str = "kimi";
const TITLE_MAX_CHARS: usize = 80;

#[cfg(test)]
static TEST_KIMI_SESSIONS_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_KIMI_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

impl Provider for KimiProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Kimi"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Directory,
            turn_quality: TurnQuality::Inferred,
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
            },
            ..ProviderCapabilities::full_session_management()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let root = get_kimi_sessions_dir();
        if !root.exists() {
            return Ok(Vec::new());
        }

        let work_dirs = load_work_dir_map()?;
        let mut seen_session_ids = BTreeMap::new();
        let mut sessions = Vec::new();

        for (work_dir_key, work_dir) in &work_dirs {
            let sessions_dir = root.join(work_dir_key);
            let entries = match std::fs::read_dir(&sessions_dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "Failed to read Kimi work-dir sessions: {}",
                            sessions_dir.display()
                        )
                    })
                }
            };
            let mut session_dirs = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.is_dir() && path.join("context.jsonl").is_file())
                .collect::<Vec<_>>();
            session_dirs.sort();

            for session_dir in session_dirs {
                let Some(session_id) = session_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|session_id| !session_id.is_empty())
                    .map(str::to_string)
                else {
                    continue;
                };
                if let Some(previous) =
                    seen_session_ids.insert(session_id.clone(), session_dir.clone())
                {
                    anyhow::bail!(
                        "Ambiguous Kimi session id {session_id}: {} and {}",
                        previous.display(),
                        session_dir.display()
                    );
                }
                if let Some(summary) = kimi_session_summary(
                    &session_dir,
                    session_id,
                    Some(work_dir.project_dir.clone()),
                )? {
                    sessions.push(summary);
                }
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

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        let Some(session_dir) = find_session_dir(session_id)? else {
            return Ok(None);
        };
        if !session_dir.join("context.jsonl").is_file() {
            return Ok(None);
        }

        let work_dir_key = session_dir
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str());
        let work_dirs = load_work_dir_map()?;
        let project_dir = work_dir_key
            .and_then(|key| work_dirs.get(key))
            .map(|work_dir| work_dir.project_dir.clone());
        kimi_session_summary(&session_dir, session_id.to_string(), project_dir)
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
        import_kimi_session_page(Path::new(source_path), event_offset, event_limit)
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        kimi_session_source_fingerprint(Path::new(source_path))
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
        let dir = backup::validate_mutation_source(ProviderSourceMutation::Delete, session_id)?;
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("Failed to delete Kimi session dir: {}", dir.display()))?;
        fail_kimi_mutation_after_write(ProviderSourceMutation::Delete)?;
        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let dir = backup::validate_mutation_source(ProviderSourceMutation::Rename, session_id)?;
        let state_path = dir.join("state.json");

        let raw = std::fs::read_to_string(&state_path)?;
        let mut state: Value = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse state.json: {}", state_path.display()))?;

        if let Some(obj) = state.as_object_mut() {
            obj.insert(
                "custom_title".to_string(),
                Value::String(new_title.to_string()),
            );
        }

        let updated = serde_json::to_vec_pretty(&state)?;
        write_kimi_state_atomically(&state_path, &updated)?;
        fail_kimi_mutation_after_write(ProviderSourceMutation::Rename)?;
        Ok(())
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        backup::create_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        backup::restore_session_backup(backup)
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("kimi resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let dir = find_session_dir(session_id)?
            .with_context(|| format!("Kimi session not found: {}", session_id))?;
        let mut total: u64 = 0;
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
        Ok(total)
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        vec![get_kimi_sessions_dir()]
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_kimi_sessions_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_KIMI_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi sessions dir lock")
        .clone()
    {
        return path;
    }

    dirs::home_dir()
        .map(|h| h.join(".kimi").join("sessions"))
        .unwrap_or_else(|| PathBuf::from(".kimi").join("sessions"))
}

fn get_kimi_json_path() -> PathBuf {
    #[cfg(test)]
    if let Some(sessions_dir) = TEST_KIMI_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi sessions dir lock")
        .clone()
    {
        return sessions_dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("kimi.json");
    }

    dirs::home_dir()
        .map(|h| h.join(".kimi").join("kimi.json"))
        .unwrap_or_else(|| PathBuf::from(".kimi").join("kimi.json"))
}

fn md5_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let hash = md5::compute(data);
    let mut hex = String::with_capacity(32);
    for byte in hash.as_ref() {
        write!(&mut hex, "{:02x}", byte).unwrap();
    }
    hex
}

#[derive(Debug, serde::Deserialize)]
struct KimiState {
    #[serde(default)]
    custom_title: Option<String>,
    #[serde(default)]
    archived: bool,
}

#[derive(Debug, Clone)]
struct KimiWorkDir {
    project_dir: String,
    mapping_fingerprint: String,
    mapping_size_bytes: i64,
}

fn read_state_json(path: &Path) -> Result<KimiState> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read state.json: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse state.json: {}", path.display()))
}

fn load_work_dir_map() -> Result<BTreeMap<String, KimiWorkDir>> {
    let path = get_kimi_json_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read kimi.json: {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse kimi.json: {}", path.display()))?;

    let mut map = BTreeMap::new();
    if let Some(dirs) = value.get("work_dirs").and_then(Value::as_array) {
        for entry in dirs {
            let Some(path_str) = entry.get("path").and_then(Value::as_str) else {
                continue;
            };
            let kaos = entry.get("kaos").and_then(Value::as_str).unwrap_or("local");
            let path_hash = md5_hex(path_str.as_bytes());
            let key = if kaos == "local" {
                path_hash
            } else {
                format!("{kaos}_{path_hash}")
            };
            let mapping_bytes = serde_json::to_vec(entry)?;
            let work_dir = KimiWorkDir {
                project_dir: path_str.to_string(),
                mapping_fingerprint: md5_hex(&mapping_bytes),
                mapping_size_bytes: i64::try_from(mapping_bytes.len()).unwrap_or(i64::MAX),
            };
            if map.insert(key.clone(), work_dir).is_some() {
                anyhow::bail!("Duplicate Kimi work-dir key in kimi.json: {key}");
            }
        }
    }
    Ok(map)
}

fn find_session_dirs(session_id: &str) -> Result<Vec<PathBuf>> {
    let root = get_kimi_sessions_dir();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    for entry in WalkDir::new(&root)
        .max_depth(2)
        .min_depth(2)
        .follow_links(false)
    {
        let entry =
            entry.with_context(|| format!("Failed to walk Kimi sessions: {}", root.display()))?;
        if entry.file_type().is_dir()
            && entry.path().file_name().and_then(|name| name.to_str()) == Some(session_id)
        {
            matches.push(entry.path().to_path_buf());
        }
    }
    matches.sort();
    Ok(matches)
}

fn find_session_dir(session_id: &str) -> Result<Option<PathBuf>> {
    let matches = find_session_dirs(session_id)?;
    match matches.as_slice() {
        [] => Ok(None),
        [session_dir] => Ok(Some(session_dir.clone())),
        _ => anyhow::bail!("Kimi session id is ambiguous: {session_id}"),
    }
}

fn kimi_session_summary(
    session_dir: &Path,
    session_id: String,
    project_dir: Option<String>,
) -> Result<Option<ProviderSessionSummary>> {
    let state_path = session_dir.join("state.json");
    let (state_title, archived) = if state_path.exists() {
        match read_state_json(&state_path) {
            Ok(state) => (state.custom_title, state.archived),
            Err(_) => (None, false),
        }
    } else {
        (None, false)
    };
    if archived {
        return Ok(None);
    }

    let title = state_title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
        .or_else(|| first_turn_begin_text(&session_dir.join("wire.jsonl")));
    let last_active_at = file_modified_ms(&session_dir.join("context.jsonl"))?;

    Ok(Some(ProviderSessionSummary {
        session_id,
        title,
        project_dir,
        last_active_at,
        source_path: Some(session_dir.to_string_lossy().to_string()),
    }))
}

fn first_turn_begin_text(wire_path: &Path) -> Option<String> {
    let file = File::open(wire_path).ok()?;
    for line in BufReader::new(file).lines().map_while(|line| line.ok()) {
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("type").and_then(Value::as_str) != Some("TurnBegin") {
            continue;
        }
        let Some(inputs) = message
            .get("payload")
            .and_then(|payload| payload.get("user_input"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for input in inputs {
            if input.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = input
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

fn file_modified_ms(path: &Path) -> Result<Option<i64>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata_modified_ms(&metadata))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to read Kimi source metadata: {}", path.display())),
    }
}

fn metadata_modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn kimi_file_fingerprint(path: &Path, required: bool) -> Result<Option<(String, i64, i64)>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return if required {
                Ok(None)
            } else {
                Ok(Some(("absent".to_string(), 0, 0)))
            };
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("Failed to read Kimi source metadata: {}", path.display())
            })
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!("Kimi source is not a regular file: {}", path.display());
    }
    let modified_at_ms = metadata_modified_ms(&metadata);
    let size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
    Ok(Some((
        format!("present:{modified_at_ms}:{size_bytes}"),
        modified_at_ms,
        size_bytes,
    )))
}

fn kimi_session_source_fingerprint(
    session_dir: &Path,
) -> Result<Option<ProviderSourceFingerprint>> {
    let metadata = match std::fs::symlink_metadata(session_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read Kimi session source metadata: {}",
                    session_dir.display()
                )
            })
        }
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kimi session source locator must be a directory: {}",
            session_dir.display()
        );
    }
    let sessions_root = get_kimi_sessions_dir();
    if session_dir.parent().and_then(Path::parent) != Some(sessions_root.as_path()) {
        anyhow::bail!(
            "Kimi session source locator is outside the configured sessions root: {}",
            session_dir.display()
        );
    }

    let Some((context_marker, context_modified_at_ms, context_size_bytes)) =
        kimi_file_fingerprint(&session_dir.join("context.jsonl"), true)?
    else {
        return Ok(None);
    };
    let (wire_marker, wire_modified_at_ms, wire_size_bytes) =
        kimi_file_fingerprint(&session_dir.join("wire.jsonl"), false)?
            .expect("optional Kimi fingerprint marker");
    let (state_marker, state_modified_at_ms, state_size_bytes) =
        kimi_file_fingerprint(&session_dir.join("state.json"), false)?
            .expect("optional Kimi fingerprint marker");

    let work_dir_key = session_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .context("Kimi session source has no work-dir key")?;
    let work_dirs = load_work_dir_map()?;
    let (mapping_marker, mapping_size_bytes) = work_dirs
        .get(work_dir_key)
        .map(|work_dir| {
            (
                format!("present:{}", work_dir.mapping_fingerprint),
                work_dir.mapping_size_bytes,
            )
        })
        .unwrap_or_else(|| ("absent".to_string(), 0));
    let kimi_json_modified_at_ms = file_modified_ms(&get_kimi_json_path())?.unwrap_or(0);
    let modified_at_ms = context_modified_at_ms
        .max(wire_modified_at_ms)
        .max(state_modified_at_ms)
        .max(kimi_json_modified_at_ms);
    let size_bytes = context_size_bytes
        .saturating_add(wire_size_bytes)
        .saturating_add(state_size_bytes)
        .saturating_add(mapping_size_bytes);

    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes,
        value: format!(
            "kimi-v1:context:{context_marker}:wire:{wire_marker}:state:{state_marker}:mapping:{mapping_marker}"
        ),
    }))
}

fn write_kimi_state_atomically(state_path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = state_path
        .parent()
        .context("Kimi state.json has no parent directory")?;
    let temporary_path = parent.join(format!(".state.json.memorph-{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<()> {
        let mut file = File::create(&temporary_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary_path, state_path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    write_result
}

#[cfg(test)]
fn set_test_kimi_sessions_dir(path: Option<PathBuf>) {
    *TEST_KIMI_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi sessions dir lock") = path;
}

#[cfg(test)]
fn set_test_kimi_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_KIMI_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi mutation failure lock") = mutation;
}

#[cfg(test)]
fn fail_kimi_mutation_after_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_KIMI_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Kimi mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected Kimi mutation failure after provider write");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_kimi_mutation_after_write(_mutation: ProviderSourceMutation) -> Result<()> {
    Ok(())
}

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let project_hash = md5_hex(target_dir.to_string_lossy().as_bytes());
    let session_dir = get_kimi_sessions_dir()
        .join(&project_hash)
        .join(&session_id);
    std::fs::create_dir_all(&session_dir)?;

    let wire_path = session_dir.join("wire.jsonl");
    let context_path = session_dir.join("context.jsonl");
    let state_path = session_dir.join("state.json");
    let mut wire_file = File::create(&wire_path)?;
    let mut context_file = File::create(&context_path)?;

    writeln!(
        wire_file,
        "{}",
        serde_json::json!({"type": "metadata", "protocol_version": "1.9"})
    )?;

    for event in &session.events {
        let Some(visible_role) = canonical_event_visible_message_role(event) else {
            continue;
        };
        let ts = event.timestamp.timestamp_millis() as f64 / 1000.0;
        match visible_role {
            EventRole::Assistant => {
                let Some(text) = canonical_event_visible_message_text(event) else {
                    continue;
                };
                for block in &event.blocks {
                    if let Some(payload) = canonical_block_to_kimi_content_part(block) {
                        writeln!(
                            wire_file,
                            "{}",
                            serde_json::json!({
                                "timestamp": ts,
                                "message": {
                                    "type": "ContentPart",
                                    "payload": payload
                                }
                            })
                        )?;
                    }
                }
                writeln!(
                    context_file,
                    "{}",
                    serde_json::json!({
                        "role": "assistant",
                        "content": text
                    })
                )?;
            }
            _ => {
                let Some(text) = canonical_event_visible_message_text(event) else {
                    continue;
                };
                writeln!(
                    wire_file,
                    "{}",
                    serde_json::json!({
                        "timestamp": ts,
                        "message": {
                            "type": "TurnBegin",
                            "payload": {
                                "user_input": [{"type": "text", "text": text}]
                            }
                        }
                    })
                )?;
                writeln!(
                    wire_file,
                    "{}",
                    serde_json::json!({
                        "timestamp": ts,
                        "message": {
                            "type": "StepBegin",
                            "payload": {"n": 1}
                        }
                    })
                )?;
                writeln!(
                    context_file,
                    "{}",
                    serde_json::json!({"role": "user", "content": text})
                )?;
            }
        }
    }

    let end_ts = session
        .context
        .last_active_at
        .or_else(|| session.events.last().map(|event| event.timestamp))
        .unwrap_or_else(Utc::now)
        .timestamp_millis() as f64
        / 1000.0;
    writeln!(
        wire_file,
        "{}",
        serde_json::json!({
            "timestamp": end_ts,
            "message": {
                "type": "StatusUpdate",
                "payload": {}
            }
        })
    )?;
    writeln!(
        wire_file,
        "{}",
        serde_json::json!({
            "timestamp": end_ts,
            "message": {
                "type": "TurnEnd",
                "payload": {}
            }
        })
    )?;

    let title = canonical_session_title(session)
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect::<String>();
    let state = serde_json::json!({
        "version": 1,
        "approval": {
            "yolo": false,
            "auto_approve_actions": []
        },
        "additional_dirs": [],
        "custom_title": title,
        "title_generated": false,
        "title_generate_attempts": 0,
        "plan_mode": false,
        "plan_session_id": null,
        "plan_slug": null,
        "wire_mtime": null,
        "archived": false,
        "archived_at": null,
        "auto_archive_exempt": false,
        "todos": []
    });
    let mut state_file = File::create(&state_path)?;
    write!(state_file, "{}", serde_json::to_string_pretty(&state)?)?;

    Ok(session_id)
}

fn canonical_block_to_kimi_content_part(block: &EventBlock) -> Option<Value> {
    match block {
        EventBlock::Text { text } => Some(serde_json::json!({
            "type": "text",
            "text": text
        })),
        EventBlock::Thinking { text, .. } => Some(serde_json::json!({
            "type": "think",
            "think": text,
            "encrypted": null
        })),
        _ => canonical_visible_block_text(block).map(|text| {
            serde_json::json!({
                "type": "text",
                "text": text
            })
        }),
    }
}

fn import_kimi_session_page(
    session_dir: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let mut imported = import_canonical_session_from_dir(session_dir)?;
    let event_count = imported.session.events.len();
    let message_count = imported
        .session
        .events
        .iter()
        .filter(|event| canonical_event_is_visible_message(event))
        .count();
    let turn_count = crate::session_projection::project_session_turns(
        &imported.session.identity.canonical_id,
        &imported.session.events,
        TurnQuality::Inferred,
    )
    .len();
    let offset = event_offset.min(event_count);
    imported.session.events = match event_limit {
        Some(limit) => imported
            .session
            .events
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect(),
        None => imported.session.events.into_iter().skip(offset).collect(),
    };
    let turns = crate::session_projection::project_session_turns(
        &imported.session.identity.canonical_id,
        &imported.session.events,
        TurnQuality::Inferred,
    );

    Ok(ProviderSessionImportPage {
        imported,
        event_count,
        message_count,
        turn_count: Some(turn_count),
        turns,
    })
}

fn import_canonical_session_from_dir(session_dir: &Path) -> Result<ImportedSession> {
    let metadata = std::fs::symlink_metadata(session_dir).with_context(|| {
        format!(
            "Failed to read Kimi session directory: {}",
            session_dir.display()
        )
    })?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kimi session source locator must be a directory: {}",
            session_dir.display()
        );
    }
    let context_path = session_dir.join("context.jsonl");
    if !context_path.is_file() {
        anyhow::bail!("Kimi context.jsonl not found: {}", context_path.display());
    }
    let wire_path = session_dir.join("wire.jsonl");
    let state_path = session_dir.join("state.json");
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let state_value = read_optional_kimi_state(&state_path, &mut report)?;
    let title = state_value
        .as_ref()
        .and_then(|state| state.get("custom_title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    let session_id = session_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let project_dir = kimi_project_dir_for_session_dir(session_dir);
    let context_modified_at = kimi_file_modified_at(&context_path)?;

    let mut context_events =
        canonical_events_from_context(&context_path, context_modified_at, &mut report)?;
    let wire = canonical_events_from_wire(&wire_path, &mut report)?;
    reconcile_kimi_context_with_wire(&mut context_events, &wire.visible_events, &mut report);
    context_events.extend(wire.lifecycle_events);

    let mut extensions = BTreeMap::new();
    if let Some(state) = state_value {
        extensions.insert("kimi_state".to_string(), state);
    }
    if !wire.metadata_headers.is_empty() {
        extensions.insert(
            "kimi_wire_metadata".to_string(),
            Value::Array(wire.metadata_headers),
        );
    }
    if !wire.unsequenced_records.is_empty() {
        extensions.insert(
            "kimi_wire_unsequenced_records".to_string(),
            Value::Array(wire.unsequenced_records),
        );
    }

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: session_id.clone(),
                source_title: title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id,
                    source_path: Some(session_dir.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: project_dir,
                created_at: wire.first_timestamp.or(Some(context_modified_at)),
                last_active_at: Some(context_modified_at),
                tags: Vec::new(),
            },
            events: context_events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn read_optional_kimi_state(
    state_path: &Path,
    report: &mut MappingReport,
) -> Result<Option<Value>> {
    let raw = match std::fs::read_to_string(state_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed to read Kimi state: {}", state_path.display()))
        }
    };
    match serde_json::from_str(&raw) {
        Ok(state) => Ok(Some(state)),
        Err(error) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Dropped,
                code: "invalid_state_json".to_string(),
                message: format!("Failed to parse Kimi state.json: {error}"),
                path: Some("state.json".to_string()),
                raw: Some(Value::String(raw)),
            });
            Ok(None)
        }
    }
}

fn kimi_file_modified_at(path: &Path) -> Result<chrono::DateTime<Utc>> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("Failed to read Kimi source metadata: {}", path.display()))?;
    let modified = metadata
        .modified()
        .with_context(|| format!("Failed to read Kimi source mtime: {}", path.display()))?;
    Ok(chrono::DateTime::<Utc>::from(modified))
}

#[derive(Default)]
struct KimiWireImport {
    visible_events: Vec<SessionEvent>,
    lifecycle_events: Vec<SessionEvent>,
    metadata_headers: Vec<Value>,
    unsequenced_records: Vec<Value>,
    first_timestamp: Option<chrono::DateTime<Utc>>,
}

#[derive(Clone)]
struct KimiWireTurn {
    provider_turn_id: String,
    turn_index: u32,
}

fn canonical_events_from_wire(
    wire_path: &Path,
    report: &mut MappingReport,
) -> Result<KimiWireImport> {
    let file = match File::open(wire_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(KimiWireImport::default())
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("Failed to open Kimi wire.jsonl: {}", wire_path.display())
            })
        }
    };
    let reader = BufReader::new(file);
    let mut imported = KimiWireImport::default();
    let mut active_turn: Option<KimiWireTurn> = None;
    let mut assistant_blocks: Vec<EventBlock> = Vec::new();
    let mut assistant_raw_parts: Vec<Value> = Vec::new();
    let mut assistant_timestamp: Option<chrono::DateTime<Utc>> = None;
    let mut assistant_line_number: Option<usize> = None;
    let mut next_turn_index = 0u32;

    for (line_idx, line) in reader.lines().enumerate() {
        let line_number = line_idx + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_wire_jsonl_line".to_string(),
                    message: format!("Failed to parse Kimi wire line: {error}"),
                    path: Some(format!("wire.jsonl:line:{line_number}")),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };

        if value.get("type").and_then(Value::as_str) == Some("metadata")
            && value.get("message").is_none()
        {
            imported.metadata_headers.push(value);
            continue;
        }

        let Some(timestamp) = parse_wire_timestamp(&value) else {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Downgraded,
                code: "wire_record_without_timestamp".to_string(),
                message: "Preserved Kimi wire record without inventing an event timestamp"
                    .to_string(),
                path: Some(format!("wire.jsonl:line:{line_number}")),
                raw: Some(value.clone()),
            });
            imported.unsequenced_records.push(value);
            continue;
        };
        imported.first_timestamp = imported.first_timestamp.or(Some(timestamp));

        let message_type = value
            .get("message")
            .and_then(|message| message.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        match message_type {
            "TurnBegin" => {
                flush_kimi_wire_assistant(
                    &mut imported.visible_events,
                    &mut assistant_blocks,
                    &mut assistant_raw_parts,
                    &mut assistant_timestamp,
                    &mut assistant_line_number,
                    active_turn.as_ref(),
                );
                let turn = KimiWireTurn {
                    provider_turn_id: format!("kimi:wire-turn:{line_number}"),
                    turn_index: next_turn_index,
                };
                next_turn_index = next_turn_index.saturating_add(1);
                active_turn = Some(turn.clone());
                imported.lifecycle_events.push(kimi_wire_event(
                    format!("kimi:wire:TurnBegin:{line_number}"),
                    SessionEventKind::Lifecycle,
                    EventRole::System,
                    timestamp,
                    Some((&turn, Some(TurnBoundary::Started))),
                    vec![EventBlock::ProviderPayload {
                        kind: "TurnBegin".to_string(),
                        payload: value.clone(),
                    }],
                    vec![value.clone()],
                ));
                let payload = value
                    .get("message")
                    .and_then(|message| message.get("payload"));
                let blocks = kimi_user_input_event_blocks(payload, &value, line_number, report);
                if !blocks.is_empty() {
                    imported.visible_events.push(kimi_wire_event(
                        format!("kimi:wire:user:{line_number}"),
                        SessionEventKind::Message,
                        EventRole::User,
                        timestamp,
                        Some((&turn, None)),
                        blocks,
                        vec![value],
                    ));
                }
            }
            "ContentPart" => {
                let payload = value
                    .get("message")
                    .and_then(|message| message.get("payload"));
                if let Some(block) =
                    kimi_content_part_event_block(payload, &value, line_number, report)
                {
                    if matches!(
                        block,
                        EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. }
                    ) {
                        imported.lifecycle_events.push(kimi_wire_event(
                            format!("kimi:wire:ContentPart:{line_number}"),
                            SessionEventKind::Unknown,
                            EventRole::Assistant,
                            timestamp,
                            active_turn.as_ref().map(|turn| (turn, None)),
                            vec![block],
                            vec![value],
                        ));
                    } else {
                        assistant_blocks.push(block);
                        assistant_raw_parts.push(value);
                        assistant_timestamp = assistant_timestamp.or(Some(timestamp));
                        assistant_line_number = assistant_line_number.or(Some(line_number));
                    }
                }
            }
            "TurnEnd" => {
                flush_kimi_wire_assistant(
                    &mut imported.visible_events,
                    &mut assistant_blocks,
                    &mut assistant_raw_parts,
                    &mut assistant_timestamp,
                    &mut assistant_line_number,
                    active_turn.as_ref(),
                );
                imported.lifecycle_events.push(kimi_wire_event(
                    format!("kimi:wire:TurnEnd:{line_number}"),
                    SessionEventKind::Lifecycle,
                    EventRole::System,
                    timestamp,
                    active_turn
                        .as_ref()
                        .map(|turn| (turn, Some(TurnBoundary::Completed))),
                    vec![EventBlock::ProviderPayload {
                        kind: "TurnEnd".to_string(),
                        payload: value.clone(),
                    }],
                    vec![value],
                ));
                active_turn = None;
            }
            "StepBegin" | "StatusUpdate" => {
                imported.lifecycle_events.push(kimi_wire_event(
                    format!("kimi:wire:{message_type}:{line_number}"),
                    SessionEventKind::Lifecycle,
                    EventRole::System,
                    timestamp,
                    active_turn.as_ref().map(|turn| (turn, None)),
                    vec![EventBlock::ProviderPayload {
                        kind: message_type.to_string(),
                        payload: value.clone(),
                    }],
                    vec![value],
                ));
            }
            other => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Preserved,
                    code: "provider_wire_message_preserved".to_string(),
                    message: format!("Preserved unsupported Kimi wire message '{other}'"),
                    path: Some(format!("wire.jsonl:line:{line_number}")),
                    raw: Some(value.clone()),
                });
                imported.lifecycle_events.push(kimi_wire_event(
                    format!("kimi:wire:{other}:{line_number}"),
                    SessionEventKind::Unknown,
                    EventRole::System,
                    timestamp,
                    active_turn.as_ref().map(|turn| (turn, None)),
                    vec![EventBlock::ProviderPayload {
                        kind: other.to_string(),
                        payload: value.clone(),
                    }],
                    vec![value],
                ));
            }
        }
    }

    flush_kimi_wire_assistant(
        &mut imported.visible_events,
        &mut assistant_blocks,
        &mut assistant_raw_parts,
        &mut assistant_timestamp,
        &mut assistant_line_number,
        active_turn.as_ref(),
    );
    Ok(imported)
}

fn flush_kimi_wire_assistant(
    events: &mut Vec<SessionEvent>,
    assistant_blocks: &mut Vec<EventBlock>,
    assistant_raw_parts: &mut Vec<Value>,
    assistant_timestamp: &mut Option<chrono::DateTime<Utc>>,
    assistant_line_number: &mut Option<usize>,
    turn: Option<&KimiWireTurn>,
) {
    if assistant_blocks.is_empty() {
        assistant_raw_parts.clear();
        *assistant_timestamp = None;
        *assistant_line_number = None;
        return;
    }
    let blocks = std::mem::take(assistant_blocks);
    let raw_parts = std::mem::take(assistant_raw_parts);
    let timestamp = assistant_timestamp
        .take()
        .expect("Kimi assistant content has a timestamp");
    let line_number = assistant_line_number
        .take()
        .expect("Kimi assistant content has a line number");
    events.push(kimi_wire_event(
        format!("kimi:wire:assistant:{line_number}"),
        kimi_event_kind(&blocks),
        EventRole::Assistant,
        timestamp,
        turn.map(|turn| (turn, None)),
        blocks,
        raw_parts,
    ));
}

fn canonical_events_from_context(
    context_path: &Path,
    fallback_timestamp: chrono::DateTime<Utc>,
    report: &mut MappingReport,
) -> Result<Vec<SessionEvent>> {
    let file = File::open(context_path).with_context(|| {
        format!(
            "Failed to open Kimi context.jsonl: {}",
            context_path.display()
        )
    })?;
    let mut events = Vec::new();
    for (line_idx, line) in BufReader::new(file).lines().enumerate() {
        let line_number = line_idx + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_context_jsonl_line".to_string(),
                    message: format!("Failed to parse Kimi context line: {error}"),
                    path: Some(format!("context.jsonl:line:{line_number}")),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };
        events.push(kimi_context_event(
            value,
            line_number,
            fallback_timestamp,
            report,
        ));
    }
    Ok(events)
}

fn kimi_context_event(
    value: Value,
    line_number: usize,
    timestamp: chrono::DateTime<Utc>,
    report: &mut MappingReport,
) -> SessionEvent {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (kind, event_role, blocks) = match role.as_deref() {
        Some("_system_prompt") => (
            SessionEventKind::Message,
            EventRole::System,
            kimi_context_content_blocks(value.get("content"), &value, line_number, report),
        ),
        Some("user") => (
            SessionEventKind::Message,
            EventRole::User,
            kimi_context_content_blocks(value.get("content"), &value, line_number, report),
        ),
        Some("assistant") => {
            let blocks =
                kimi_context_content_blocks(value.get("content"), &value, line_number, report);
            (kimi_event_kind(&blocks), EventRole::Assistant, blocks)
        }
        Some(control @ ("_checkpoint" | "_usage")) => (
            SessionEventKind::Lifecycle,
            EventRole::System,
            vec![EventBlock::ProviderPayload {
                kind: control.to_string(),
                payload: value.clone(),
            }],
        ),
        Some(other) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Preserved,
                code: "provider_context_role_preserved".to_string(),
                message: format!("Preserved unsupported Kimi context role '{other}'"),
                path: Some(format!("context.jsonl:line:{line_number}")),
                raw: Some(value.clone()),
            });
            (
                SessionEventKind::Unknown,
                EventRole::Unknown,
                vec![EventBlock::ProviderPayload {
                    kind: other.to_string(),
                    payload: value.clone(),
                }],
            )
        }
        None => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Downgraded,
                code: "context_record_without_role".to_string(),
                message: "Preserved Kimi context record without a role".to_string(),
                path: Some(format!("context.jsonl:line:{line_number}")),
                raw: Some(value.clone()),
            });
            (
                SessionEventKind::Unknown,
                EventRole::Unknown,
                vec![EventBlock::Unknown { raw: value.clone() }],
            )
        }
    };

    let mut provider_ext = BTreeMap::new();
    provider_ext.insert("kimi_context_line".to_string(), value);
    provider_ext.insert(
        "kimi_context_line_number".to_string(),
        Value::from(line_number as u64),
    );
    SessionEvent {
        id: format!("kimi:context:{line_number}"),
        kind,
        role: event_role,
        timestamp,
        links: EventLinks::default(),
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: Some(format!("context.jsonl:{line_number}")),
                original_role: role,
                phase: Some("context".to_string()),
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext,
        },
    }
}

fn kimi_context_content_blocks(
    content: Option<&Value>,
    raw_line: &Value,
    line_number: usize,
    report: &mut MappingReport,
) -> Vec<EventBlock> {
    match content {
        Some(Value::String(text)) => vec![EventBlock::Text { text: text.clone() }],
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                kimi_context_content_block(item, raw_line, line_number, index, report)
            })
            .collect(),
        Some(value) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Preserved,
                code: "provider_context_content_preserved".to_string(),
                message: "Preserved non-string Kimi context content".to_string(),
                path: Some(format!("context.jsonl:line:{line_number}:content")),
                raw: Some(raw_line.clone()),
            });
            vec![EventBlock::ProviderPayload {
                kind: "context_content".to_string(),
                payload: value.clone(),
            }]
        }
        None => Vec::new(),
    }
}

fn kimi_context_content_block(
    item: &Value,
    raw_line: &Value,
    line_number: usize,
    item_index: usize,
    report: &mut MappingReport,
) -> EventBlock {
    if let Some(text) = item.as_str() {
        return EventBlock::Text {
            text: text.to_string(),
        };
    }
    match item.get("type").and_then(Value::as_str) {
        Some("text") => EventBlock::Text {
            text: item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        Some("think") => EventBlock::Thinking {
            text: item
                .get("think")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            signature: item
                .get("encrypted")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        Some("image_url") => EventBlock::Image {
            mime_type: "image/png".to_string(),
            data: item
                .get("image_url")
                .and_then(|image| image.get("url"))
                .and_then(Value::as_str)
                .map(str::to_string),
            path: None,
        },
        Some(kind) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Preserved,
                code: "provider_context_block_preserved".to_string(),
                message: format!("Preserved unsupported Kimi context block '{kind}'"),
                path: Some(format!(
                    "context.jsonl:line:{line_number}:content:{item_index}"
                )),
                raw: Some(raw_line.clone()),
            });
            EventBlock::ProviderPayload {
                kind: kind.to_string(),
                payload: item.clone(),
            }
        }
        None => EventBlock::Unknown { raw: item.clone() },
    }
}

fn reconcile_kimi_context_with_wire(
    context_events: &mut [SessionEvent],
    wire_events: &[SessionEvent],
    report: &mut MappingReport,
) {
    let mut used = vec![false; wire_events.len()];
    let mut cursor = 0usize;
    for context_event in context_events {
        let Some(context_text) = canonical_event_visible_message_text(context_event) else {
            continue;
        };
        let Some((wire_index, wire_event)) =
            wire_events
                .iter()
                .enumerate()
                .skip(cursor)
                .find(|(_, wire_event)| {
                    wire_event.role == context_event.role
                        && canonical_event_visible_message_text(wire_event)
                            .is_some_and(|wire_text| wire_text.trim() == context_text.trim())
                })
        else {
            continue;
        };
        used[wire_index] = true;
        cursor = wire_index + 1;
        context_event.timestamp = wire_event.timestamp;
        context_event.links = wire_event.links.clone();
        if let Some(raw_lines) = wire_event.metadata.provider_ext.get("kimi_wire_lines") {
            context_event
                .metadata
                .provider_ext
                .insert("kimi_wire_lines".to_string(), raw_lines.clone());
        }
        for block in &wire_event.blocks {
            if matches!(
                block,
                EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. }
            ) && !context_event.blocks.iter().any(|existing| {
                serde_json::to_value(existing).ok() == serde_json::to_value(block).ok()
            }) {
                context_event.blocks.push(block.clone());
            }
        }
    }

    for (wire_index, wire_event) in wire_events.iter().enumerate() {
        if used[wire_index] {
            continue;
        }
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: MappingDisposition::Dropped,
            code: "wire_content_not_in_context".to_string(),
            message:
                "Dropped Kimi wire content that was not present in authoritative context.jsonl"
                    .to_string(),
            path: wire_event.metadata.source.original_id.clone(),
            raw: wire_event
                .metadata
                .provider_ext
                .get("kimi_wire_lines")
                .cloned(),
        });
    }
}

fn kimi_wire_event(
    id: String,
    kind: SessionEventKind,
    role: EventRole,
    timestamp: chrono::DateTime<Utc>,
    turn: Option<(&KimiWireTurn, Option<TurnBoundary>)>,
    blocks: Vec<EventBlock>,
    raw_parts: Vec<Value>,
) -> SessionEvent {
    SessionEvent {
        id: id.clone(),
        kind,
        role,
        timestamp,
        links: EventLinks {
            parent_event_id: None,
            provider_parent_id: None,
            provider_turn_id: turn.map(|(turn, _)| turn.provider_turn_id.clone()),
            turn_index: turn.map(|(turn, _)| turn.turn_index),
            turn_boundary: turn.and_then(|(_, boundary)| boundary),
            related_event_ids: Vec::new(),
        },
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: Some(id),
                original_role: Some(
                    match role {
                        EventRole::User => "user",
                        EventRole::Assistant => "assistant",
                        EventRole::Tool => "tool",
                        EventRole::System => "system",
                        EventRole::Developer => "developer",
                        EventRole::Unknown => "unknown",
                    }
                    .to_string(),
                ),
                phase: Some("wire".to_string()),
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("kimi_wire_lines".to_string(), Value::Array(raw_parts));
                ext
            },
        },
    }
}

fn kimi_user_input_event_blocks(
    payload: Option<&Value>,
    raw_line: &Value,
    line_number: usize,
    report: &mut MappingReport,
) -> Vec<EventBlock> {
    let Some(inputs) = payload
        .and_then(|payload| payload.get("user_input"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    inputs
        .iter()
        .enumerate()
        .map(
            |(idx, item)| match item.get("type").and_then(Value::as_str) {
                Some("text") => EventBlock::Text {
                    text: item
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                },
                Some("image_url") => EventBlock::Image {
                    mime_type: "image/png".to_string(),
                    data: item
                        .get("image_url")
                        .and_then(|value| value.get("url"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    path: None,
                },
                Some(kind) => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: MappingDisposition::Preserved,
                        code: "provider_block_preserved".to_string(),
                        message: format!("Preserved unsupported Kimi user input '{kind}'"),
                        path: Some(format!("wire.jsonl:line:{line_number}:input:{idx}")),
                        raw: Some(raw_line.clone()),
                    });
                    EventBlock::ProviderPayload {
                        kind: kind.to_string(),
                        payload: item.clone(),
                    }
                }
                None => EventBlock::Unknown { raw: item.clone() },
            },
        )
        .collect()
}

fn kimi_content_part_event_block(
    payload: Option<&Value>,
    raw_line: &Value,
    line_number: usize,
    report: &mut MappingReport,
) -> Option<EventBlock> {
    let payload = payload?;
    match payload.get("type").and_then(Value::as_str) {
        Some("text") => Some(EventBlock::Text {
            text: payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        Some("think") => Some(EventBlock::Thinking {
            text: payload
                .get("think")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            signature: payload
                .get("encrypted")
                .and_then(Value::as_str)
                .map(str::to_string),
        }),
        Some(kind) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Preserved,
                code: "provider_block_preserved".to_string(),
                message: format!("Preserved unsupported Kimi content part '{kind}'"),
                path: Some(format!("wire.jsonl:line:{line_number}")),
                raw: Some(raw_line.clone()),
            });
            Some(EventBlock::ProviderPayload {
                kind: kind.to_string(),
                payload: payload.clone(),
            })
        }
        None => Some(EventBlock::Unknown {
            raw: payload.clone(),
        }),
    }
}

fn kimi_event_kind(blocks: &[EventBlock]) -> SessionEventKind {
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
    } else if blocks.iter().all(|block| {
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

fn kimi_project_dir_for_session_dir(session_dir: &Path) -> Option<String> {
    let project_hash = session_dir
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())?;
    load_work_dir_map()
        .ok()?
        .get(project_hash)
        .map(|work_dir| work_dir.project_dir.clone())
}

fn parse_wire_timestamp(value: &Value) -> Option<chrono::DateTime<Utc>> {
    let ts = value.get("timestamp").and_then(|v| v.as_f64())?;
    let secs = ts as i64;
    let nanos = ((ts - secs as f64) * 1e9).max(0.0) as u32;
    chrono::DateTime::from_timestamp(secs, nanos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::session_management, storage::local_store};
    use std::collections::{BTreeMap, BTreeSet};
    use tempfile::tempdir;

    static TEST_KIMI_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    struct TestKimiSessionsGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
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

    impl Drop for TestKimiSessionsGuard {
        fn drop(&mut self) {
            crate::cache::global_cache().invalidate(PROVIDER_ID);
            backup::set_test_backup_failure(false);
            set_test_kimi_mutation_failure(None);
            set_test_kimi_sessions_dir(None);
        }
    }

    fn use_test_kimi_sessions_dir(path: PathBuf) -> TestKimiSessionsGuard {
        let lock = TEST_KIMI_TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set_test_kimi_sessions_dir(Some(path));
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        TestKimiSessionsGuard { _lock: lock }
    }

    fn write_native_kimi_fixture(root: &Path, project: &str, session_id: &str) -> PathBuf {
        let project_dir = format!("/workspace/{project}");
        let project_key = md5_hex(project_dir.as_bytes());
        let metadata_path = root.parent().unwrap().join("kimi.json");
        let mut metadata = if metadata_path.exists() {
            serde_json::from_slice::<Value>(&std::fs::read(&metadata_path).unwrap()).unwrap()
        } else {
            serde_json::json!({ "work_dirs": [] })
        };
        let work_dirs = metadata["work_dirs"].as_array_mut().unwrap();
        if !work_dirs
            .iter()
            .any(|work_dir| work_dir["path"] == project_dir)
        {
            work_dirs.push(serde_json::json!({
                "path": project_dir,
                "kaos": "local",
                "last_session_id": session_id
            }));
            std::fs::write(
                &metadata_path,
                serde_json::to_vec_pretty(&metadata).unwrap(),
            )
            .unwrap();
        }

        let session_dir = root.join(project_key).join(session_id);
        std::fs::create_dir_all(session_dir.join("nested")).unwrap();
        std::fs::write(
            session_dir.join("state.json"),
            b"{\n  \"version\": 1,\n  \"custom_title\": \"Before\",\n  \"archived\": false,\n  \"native\": {\"keep\": true}\n}\n",
        )
        .unwrap();
        std::fs::write(
            session_dir.join("wire.jsonl"),
            b"{\"timestamp\":1710000000.0,\"message\":{\"type\":\"metadata\"}}\n",
        )
        .unwrap();
        std::fs::write(
            session_dir.join("context.jsonl"),
            b"{\"role\":\"user\",\"content\":\"hello\"}\n",
        )
        .unwrap();
        std::fs::write(
            session_dir.join("nested").join("native.bin"),
            [0_u8, 1, 127, 128, 255],
        )
        .unwrap();
        session_dir
    }

    fn session_tree_bytes(session_dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        WalkDir::new(session_dir)
            .min_depth(1)
            .into_iter()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                (
                    entry
                        .path()
                        .strip_prefix(session_dir)
                        .unwrap()
                        .to_path_buf(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }

    fn kimi_audit_fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/providers/kimi/fixtures/v1_37_0")
    }

    fn read_jsonl_values(path: &Path) -> Vec<Result<Value, serde_json::Error>> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect()
    }

    fn provider_payload_kind(event: &SessionEvent) -> Option<&str> {
        event.blocks.iter().find_map(|block| match block {
            EventBlock::ProviderPayload { kind, .. } => Some(kind.as_str()),
            _ => None,
        })
    }

    fn copy_kimi_audit_fixture(target: &Path) {
        let source = kimi_audit_fixture_root();
        for entry in WalkDir::new(&source).into_iter().map(Result::unwrap) {
            let relative = entry.path().strip_prefix(&source).unwrap();
            let destination = target.join(relative);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&destination).unwrap();
            } else {
                std::fs::copy(entry.path(), destination).unwrap();
            }
        }
    }

    fn write_kimi_metadata(root: &Path, work_dirs: Value) {
        std::fs::write(
            root.join("kimi.json"),
            serde_json::to_vec_pretty(&serde_json::json!({ "work_dirs": work_dirs })).unwrap(),
        )
        .unwrap();
    }

    fn write_context_only_session(root: &Path, work_dir_key: &str, session_id: &str) -> PathBuf {
        let session_dir = root.join("sessions").join(work_dir_key).join(session_id);
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(
            session_dir.join("context.jsonl"),
            b"{\"role\":\"user\",\"content\":\"sanitized\"}\n",
        )
        .unwrap();
        session_dir
    }

    #[test]
    fn scans_known_kimi_work_dirs_from_context_and_uses_directory_locators() {
        let dir = tempdir().unwrap();
        copy_kimi_audit_fixture(dir.path());
        let sessions_root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(sessions_root.clone());

        let sessions = KimiProvider.scan_sessions().unwrap();
        assert_eq!(sessions.len(), 2);

        let normal = sessions
            .iter()
            .find(|session| session.session_id == "11111111-1111-4111-8111-111111111111")
            .unwrap();
        let normal_dir = sessions_root
            .join("2030c6ce97e98c160351b18f097eb584")
            .join(&normal.session_id);
        assert_eq!(normal.title.as_deref(), Some("Sanitized session"));
        assert_eq!(
            normal.project_dir.as_deref(),
            Some("/workspace/sanitized-project")
        );
        assert_eq!(
            normal.source_path.as_deref(),
            Some(normal_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            normal.last_active_at,
            file_modified_ms(&normal_dir.join("context.jsonl")).unwrap()
        );

        let context_only = sessions
            .iter()
            .find(|session| session.session_id == "22222222-2222-4222-8222-222222222222")
            .unwrap();
        assert_eq!(context_only.title, None);
        assert_eq!(
            context_only.source_path.as_deref(),
            Some(
                sessions_root
                    .join("0017cc2b0eee031e9194d1384b4bcdd8")
                    .join(&context_only.session_id)
                    .to_string_lossy()
                    .as_ref()
            )
        );

        std::fs::remove_file(normal_dir.join("state.json")).unwrap();
        let fallback = KimiProvider
            .get_session_meta(&normal.session_id)
            .unwrap()
            .unwrap();
        assert_eq!(fallback.title.as_deref(), Some("[sanitized user request]"));
    }

    #[test]
    fn scan_supports_local_and_remote_kaos_keys_and_ignores_orphans() {
        let dir = tempdir().unwrap();
        let sessions_root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
        let local_path = "/workspace/local";
        let remote_path = "/workspace/remote";
        let local_key = md5_hex(local_path.as_bytes());
        let remote_key = format!("ssh_{}", md5_hex(remote_path.as_bytes()));
        write_kimi_metadata(
            dir.path(),
            serde_json::json!([
                { "path": local_path, "kaos": "local", "last_session_id": "local-session" },
                { "path": remote_path, "kaos": "ssh", "last_session_id": "remote-session" }
            ]),
        );
        write_context_only_session(dir.path(), &local_key, "local-session");
        write_context_only_session(dir.path(), &remote_key, "remote-session");
        write_context_only_session(dir.path(), "orphan-key", "orphan-session");

        let sessions = KimiProvider.scan_sessions().unwrap();
        assert_eq!(
            sessions
                .iter()
                .map(|session| (
                    session.session_id.as_str(),
                    session.project_dir.as_deref().unwrap()
                ))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                ("local-session", local_path),
                ("remote-session", remote_path),
            ])
        );
        assert!(sessions
            .iter()
            .all(|session| session.session_id != "orphan-session"));
    }

    #[test]
    fn duplicate_kimi_session_ids_are_rejected_by_scan_and_identity_reads() {
        let dir = tempdir().unwrap();
        let sessions_root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(sessions_root);
        let first_path = "/workspace/first";
        let second_path = "/workspace/second";
        let first_key = md5_hex(first_path.as_bytes());
        let second_key = md5_hex(second_path.as_bytes());
        write_kimi_metadata(
            dir.path(),
            serde_json::json!([
                { "path": first_path, "kaos": "local" },
                { "path": second_path, "kaos": "local" }
            ]),
        );
        write_context_only_session(dir.path(), &first_key, "duplicate-session");
        write_context_only_session(dir.path(), &second_key, "duplicate-session");

        assert!(KimiProvider
            .scan_sessions()
            .unwrap_err()
            .to_string()
            .contains("Ambiguous Kimi session id"));
        assert!(KimiProvider
            .get_session_meta("duplicate-session")
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
        assert!(KimiProvider
            .session_size("duplicate-session")
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
    }

    #[test]
    fn kimi_fingerprint_covers_context_wire_state_and_relevant_mapping() {
        let dir = tempdir().unwrap();
        copy_kimi_audit_fixture(dir.path());
        let sessions_root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
        let session_dir = sessions_root
            .join("2030c6ce97e98c160351b18f097eb584")
            .join("11111111-1111-4111-8111-111111111111");
        let variants = dir.path().join("variants");
        let fingerprint = || {
            KimiProvider
                .session_source_fingerprint(session_dir.to_str().unwrap())
                .unwrap()
                .unwrap()
                .value
        };

        let before_state = fingerprint();
        std::fs::copy(
            variants.join("state.updated.json"),
            session_dir.join("state.json"),
        )
        .unwrap();
        assert_ne!(fingerprint(), before_state);

        let before_context = fingerprint();
        std::fs::copy(
            variants.join("context.updated.jsonl"),
            session_dir.join("context.jsonl"),
        )
        .unwrap();
        assert_ne!(fingerprint(), before_context);

        let before_wire = fingerprint();
        std::fs::copy(
            variants.join("wire.updated.jsonl"),
            session_dir.join("wire.jsonl"),
        )
        .unwrap();
        assert_ne!(fingerprint(), before_wire);

        let before_mapping = fingerprint();
        let mut metadata: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("kimi.json")).unwrap()).unwrap();
        metadata["work_dirs"][0]["last_session_id"] = Value::String("changed-session".to_string());
        std::fs::write(
            dir.path().join("kimi.json"),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        assert_ne!(fingerprint(), before_mapping);

        std::fs::copy(
            variants.join("wire.malformed.jsonl"),
            session_dir.join("wire.jsonl"),
        )
        .unwrap();
        assert!(fingerprint().starts_with("kimi-v1:"));
        std::fs::remove_file(session_dir.join("wire.jsonl")).unwrap();
        assert!(fingerprint().contains("wire:absent"));

        assert!(KimiProvider
            .session_source_fingerprint(session_dir.join("state.json").to_str().unwrap())
            .unwrap_err()
            .to_string()
            .contains("must be a directory"));
        std::fs::remove_file(session_dir.join("context.jsonl")).unwrap();
        assert!(KimiProvider
            .session_source_fingerprint(session_dir.to_str().unwrap())
            .unwrap()
            .is_none());
    }

    #[test]
    fn kimi_import_accepts_only_directory_locators_and_context_only_sessions() {
        let dir = tempdir().unwrap();
        copy_kimi_audit_fixture(dir.path());
        let sessions_root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
        let session_dir = sessions_root
            .join("2030c6ce97e98c160351b18f097eb584")
            .join("11111111-1111-4111-8111-111111111111");

        let imported = KimiProvider
            .import_session(session_dir.to_str().unwrap())
            .unwrap();
        assert_eq!(
            imported
                .session
                .provenance
                .primary_source
                .source_path
                .as_deref(),
            Some(session_dir.to_string_lossy().as_ref())
        );
        assert!(KimiProvider
            .import_session(session_dir.join("wire.jsonl").to_str().unwrap())
            .unwrap_err()
            .to_string()
            .contains("must be a directory"));

        let context_only_dir = sessions_root
            .join("0017cc2b0eee031e9194d1384b4bcdd8")
            .join("22222222-2222-4222-8222-222222222222");
        let context_only = KimiProvider
            .import_session(context_only_dir.to_str().unwrap())
            .unwrap();
        assert_eq!(
            context_only
                .session
                .events
                .iter()
                .filter_map(canonical_event_visible_message_text)
                .collect::<Vec<_>>(),
            vec!["[sanitized context-only request]".to_string()]
        );
        assert!(context_only.session.events.iter().any(|event| {
            event.role == EventRole::System
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text })
                        if text == "[sanitized context-only system prompt]"
                )
        }));
        assert!(!context_only.session.extensions.contains_key("kimi_state"));
        assert!(!context_only
            .session
            .extensions
            .contains_key("kimi_wire_metadata"));
        assert_eq!(context_only.report.overall, MappingDisposition::Preserved);
        assert!(!context_only_dir.join("wire.jsonl").exists());
    }

    #[test]
    fn sanitized_kimi_fixture_records_real_source_plane() {
        let root = kimi_audit_fixture_root();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(root.join("fixture.json")).unwrap()).unwrap();
        assert_eq!(manifest["provider"], "kimi");
        assert_eq!(manifest["observed_cli_version"], "1.37.0");
        assert_eq!(manifest["provenance"], "sanitized-local-source");
        assert_eq!(manifest["raw_user_content_committed"], false);

        let metadata: Value =
            serde_json::from_slice(&std::fs::read(root.join("kimi.json")).unwrap()).unwrap();
        let work_dirs = metadata["work_dirs"].as_array().unwrap();
        assert_eq!(work_dirs.len(), 2);
        for work_dir in work_dirs {
            assert_eq!(work_dir["kaos"], "local");
            let path = work_dir["path"].as_str().unwrap();
            let session_id = work_dir["last_session_id"].as_str().unwrap();
            assert!(Uuid::parse_str(session_id).is_ok());
            let session_dir = root
                .join("sessions")
                .join(md5_hex(path.as_bytes()))
                .join(session_id);
            assert!(session_dir.join("context.jsonl").is_file());
        }

        let normal = root
            .join("sessions/2030c6ce97e98c160351b18f097eb584")
            .join("11111111-1111-4111-8111-111111111111");
        assert!(normal.join("wire.jsonl").is_file());
        assert!(normal.join("state.json").is_file());

        let context_only = root
            .join("sessions/0017cc2b0eee031e9194d1384b4bcdd8")
            .join("22222222-2222-4222-8222-222222222222");
        assert!(context_only.join("context.jsonl").is_file());
        assert!(!context_only.join("wire.jsonl").exists());
        assert!(!context_only.join("state.json").exists());
    }

    #[test]
    fn sanitized_kimi_wire_fixture_preserves_observed_v1_37_schema() {
        let path = kimi_audit_fixture_root().join(
            "sessions/2030c6ce97e98c160351b18f097eb584/11111111-1111-4111-8111-111111111111/wire.jsonl",
        );
        let values = read_jsonl_values(&path)
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(values[0]["type"], "metadata");
        assert_eq!(values[0]["protocol_version"], "1.3");
        assert!(values[0].get("timestamp").is_none());
        assert!(values[0].get("message").is_none());

        let message_types: Vec<_> = values[1..]
            .iter()
            .map(|value| value["message"]["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            message_types,
            [
                "TurnBegin",
                "StepBegin",
                "ContentPart",
                "ContentPart",
                "StatusUpdate",
                "TurnEnd",
                "TurnBegin",
                "StepBegin",
                "ContentPart",
                "TurnEnd",
            ]
        );

        let content_part_types: Vec<_> = values[1..]
            .iter()
            .filter(|value| value["message"]["type"] == "ContentPart")
            .map(|value| value["message"]["payload"]["type"].as_str().unwrap())
            .collect();
        assert_eq!(content_part_types, ["think", "text", "text"]);

        let status = values[1..]
            .iter()
            .find(|value| value["message"]["type"] == "StatusUpdate")
            .unwrap();
        let status_keys: BTreeSet<_> = status["message"]["payload"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            status_keys,
            BTreeSet::from([
                "context_tokens",
                "context_usage",
                "max_context_tokens",
                "mcp_status",
                "message_id",
                "plan_mode",
                "token_usage",
            ])
        );

        let timestamps: Vec<_> = values[1..]
            .iter()
            .map(|value| value["timestamp"].as_f64().unwrap())
            .collect();
        assert!(timestamps.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn sanitized_kimi_fixture_covers_context_state_updates_and_damage() {
        let root = kimi_audit_fixture_root();
        let session = root
            .join("sessions/2030c6ce97e98c160351b18f097eb584/11111111-1111-4111-8111-111111111111");
        let context = read_jsonl_values(&session.join("context.jsonl"))
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let roles: Vec<_> = context
            .iter()
            .map(|value| value["role"].as_str().unwrap())
            .collect();
        assert_eq!(
            roles,
            [
                "_system_prompt",
                "_checkpoint",
                "user",
                "_usage",
                "assistant",
                "_checkpoint",
                "user",
                "_usage",
                "assistant",
            ]
        );
        assert!(context
            .iter()
            .filter(|value| value["role"] == "user")
            .all(|value| value["content"].is_string()));
        assert!(context
            .iter()
            .filter(|value| value["role"] == "assistant")
            .all(|value| value["content"].is_array()));

        let state: Value =
            serde_json::from_slice(&std::fs::read(session.join("state.json")).unwrap()).unwrap();
        let updated_state: Value = serde_json::from_slice(
            &std::fs::read(root.join("variants/state.updated.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            state.as_object().unwrap().keys().collect::<BTreeSet<_>>(),
            updated_state
                .as_object()
                .unwrap()
                .keys()
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(state["archived"], false);
        assert_eq!(updated_state["archived"], true);
        assert_ne!(state["custom_title"], updated_state["custom_title"]);

        let updated_context = read_jsonl_values(&root.join("variants/context.updated.jsonl"))
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(&updated_context[..context.len()], context.as_slice());
        assert_eq!(updated_context.len(), context.len() + 2);

        let wire = read_jsonl_values(&session.join("wire.jsonl"))
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let updated_wire = read_jsonl_values(&root.join("variants/wire.updated.jsonl"))
            .into_iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(&updated_wire[..wire.len()], wire.as_slice());
        assert_eq!(updated_wire.len(), wire.len() + 4);

        let malformed = read_jsonl_values(&root.join("variants/wire.malformed.jsonl"));
        assert_eq!(malformed.iter().filter(|line| line.is_ok()).count(), 3);
        assert_eq!(malformed.iter().filter(|line| line.is_err()).count(), 1);
    }

    #[test]
    fn delete_backup_restores_exact_kimi_directory_and_preserves_unrelated_sessions() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-delete";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let unrelated_dir = write_native_kimi_fixture(&root, "project-b", "kimi-other");
        let original = session_tree_bytes(&session_dir);
        let backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-delete",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        KimiProvider.delete_session(session_id).unwrap();
        assert!(!session_dir.exists());
        std::fs::write(unrelated_dir.join("wire.jsonl"), b"changed concurrently\n").unwrap();

        KimiProvider.restore_session_backup(&backup).unwrap();
        KimiProvider.restore_session_backup(&backup).unwrap();

        assert_eq!(session_tree_bytes(&session_dir), original);
        assert_eq!(
            std::fs::read(unrelated_dir.join("wire.jsonl")).unwrap(),
            b"changed concurrently\n"
        );
    }

    #[test]
    fn rename_backup_restores_exact_state_only_and_preserves_other_changes() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-rename";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let original_state = std::fs::read(session_dir.join("state.json")).unwrap();
        let backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-kimi-rename",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        KimiProvider.rename_session(session_id, "After").unwrap();
        std::fs::write(
            session_dir.join("wire.jsonl"),
            b"wire changed concurrently\n",
        )
        .unwrap();
        std::fs::write(session_dir.join("concurrent.txt"), b"keep me").unwrap();

        KimiProvider.restore_session_backup(&backup).unwrap();
        KimiProvider.restore_session_backup(&backup).unwrap();

        assert_eq!(
            std::fs::read(session_dir.join("state.json")).unwrap(),
            original_state
        );
        assert_eq!(
            std::fs::read(session_dir.join("wire.jsonl")).unwrap(),
            b"wire changed concurrently\n"
        );
        assert_eq!(
            std::fs::read(session_dir.join("concurrent.txt")).unwrap(),
            b"keep me"
        );
    }

    #[test]
    fn rename_restore_does_not_recreate_concurrently_deleted_state() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-concurrent-delete";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-kimi-concurrent-delete",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        KimiProvider.rename_session(session_id, "After").unwrap();
        std::fs::remove_file(session_dir.join("state.json")).unwrap();

        KimiProvider.restore_session_backup(&backup).unwrap();

        assert!(!session_dir.join("state.json").exists());
    }

    #[test]
    fn kimi_backup_contract_and_capabilities_are_truthful() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-contract";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-contract",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        let capabilities = KimiProvider.capabilities();
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert_eq!(backup.source_path, session_dir.canonicalize().unwrap());
        assert_eq!(backup.format, "kimi-session-backup-v1");
        assert_eq!(
            backup.mime_type,
            "application/vnd.memorph.kimi-session-backup"
        );
        assert!(backup.backup_path.join("metadata.json").is_file());
        assert!(backup.backup_path.join("session/state.json").is_file());
        assert!(backup
            .backup_path
            .join("session/nested/native.bin")
            .is_file());
    }

    #[test]
    fn backup_registration_failure_prevents_kimi_provider_write() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-registration-failure";
        let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
        let mut artifact_conn = rusqlite::Connection::open_in_memory().unwrap();

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-kimi-registration".to_string()],
            &dir.path().join("backups"),
            &mut artifact_conn,
        );

        assert!(results[0]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("Delete cancelled before provider write"));
        assert!(session_dir.exists());
        assert!(dir
            .path()
            .join("backups/kimi/operation-kimi-registration")
            .exists());
    }

    #[test]
    fn partial_kimi_delete_and_rename_failures_restore_registered_backups() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let delete_id = "kimi-partial-delete";
        let rename_id = "kimi-partial-rename";
        let delete_dir = write_native_kimi_fixture(&root, "project-a", delete_id);
        let rename_dir = write_native_kimi_fixture(&root, "project-a", rename_id);
        let delete_original = session_tree_bytes(&delete_dir);
        let rename_original = std::fs::read(rename_dir.join("state.json")).unwrap();
        let mut artifact_conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();

        set_test_kimi_mutation_failure(Some(ProviderSourceMutation::Delete));
        let delete_results = session_management::delete_sessions(
            PROVIDER_ID,
            &[delete_id],
            &["operation-kimi-partial-delete".to_string()],
            &dir.path().join("backups"),
            &mut artifact_conn,
        );
        assert!(delete_results[0]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(session_tree_bytes(&delete_dir), delete_original);

        set_test_kimi_mutation_failure(Some(ProviderSourceMutation::Rename));
        let rename_error = session_management::rename_session(
            PROVIDER_ID,
            rename_id,
            "After",
            "operation-kimi-partial-rename",
            &dir.path().join("backups"),
            &mut artifact_conn,
        )
        .unwrap_err();
        assert!(rename_error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            std::fs::read(rename_dir.join("state.json")).unwrap(),
            rename_original
        );
    }

    #[test]
    fn kimi_backup_rejects_ambiguous_and_unsafe_sources_before_mutation() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-ambiguous";
        let first = write_native_kimi_fixture(&root, "project-a", session_id);
        let second = write_native_kimi_fixture(&root, "project-b", session_id);

        let error = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-ambiguous",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err();
        assert!(error.to_string().contains("not found or ambiguous"));
        assert!(first.exists());

        std::fs::remove_dir_all(second).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(first.join("wire.jsonl"), first.join("unsafe-wire-link"))
                .unwrap();
            let error = KimiProvider
                .create_session_backup(
                    ProviderSourceMutation::Delete,
                    "operation-kimi-unsafe",
                    session_id,
                    &dir.path().join("backups"),
                )
                .unwrap_err();
            assert!(error.to_string().contains("unsupported filesystem entry"));
        }
    }

    #[test]
    fn kimi_restore_rejects_metadata_and_content_tampering() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-tamper";
        write_native_kimi_fixture(&root, "project-a", session_id);
        let backup_root = dir.path().join("backups");

        let content_backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-content-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        std::fs::write(
            content_backup.backup_path.join("session/state.json"),
            b"tampered",
        )
        .unwrap();
        assert!(KimiProvider
            .restore_session_backup(&content_backup)
            .unwrap_err()
            .to_string()
            .contains("does not match its manifest"));

        let metadata_backup = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-kimi-metadata-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        let metadata_path = metadata_backup.backup_path.join("metadata.json");
        let mut metadata: Value =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        metadata["provider_session_id"] = Value::String("other-session".to_string());
        std::fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        assert!(KimiProvider
            .restore_session_backup(&metadata_backup)
            .unwrap_err()
            .to_string()
            .contains("does not match the registered restore context"));
    }

    #[test]
    fn failed_kimi_backup_creation_removes_operation_directory() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(root.clone());
        let session_id = "kimi-backup-failure";
        write_native_kimi_fixture(&root, "project-a", session_id);
        backup::set_test_backup_failure(true);

        let error = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-backup-failure",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err();

        assert!(error.to_string().contains("injected Kimi backup failure"));
        assert!(!dir
            .path()
            .join("backups/kimi/operation-kimi-backup-failure")
            .exists());
    }

    #[test]
    fn kimi_full_import_pages_keep_total_counts_and_project_only_page_turns() {
        let dir = tempdir().unwrap();
        copy_kimi_audit_fixture(dir.path());
        let sessions_root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
        let session_dir = sessions_root
            .join("2030c6ce97e98c160351b18f097eb584")
            .join("11111111-1111-4111-8111-111111111111");
        let source_path = session_dir.to_str().unwrap();

        let capabilities = KimiProvider.capabilities();
        assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);
        assert_eq!(capabilities.storage_shape, StorageShape::Directory);
        assert_eq!(capabilities.turn_quality, TurnQuality::Inferred);

        let full = KimiProvider
            .import_session_page(source_path, 0, None)
            .unwrap();
        assert_eq!(full.imported.session.events.len(), full.event_count);
        assert_eq!(
            full.message_count,
            full.imported
                .session
                .events
                .iter()
                .filter(|event| canonical_event_is_visible_message(event))
                .count()
        );
        assert_eq!(full.turns.len(), 2);
        assert_eq!(full.turn_count, Some(full.turns.len()));

        let assistant_index = full
            .imported
            .session
            .events
            .iter()
            .position(|event| {
                event.role == EventRole::Assistant && event.links.provider_turn_id.is_some()
            })
            .unwrap();
        let expected_turn_id = full.imported.session.events[assistant_index]
            .links
            .provider_turn_id
            .clone()
            .unwrap();
        let page = KimiProvider
            .import_session_page(source_path, assistant_index, Some(1))
            .unwrap();

        assert_eq!(page.imported.session.events.len(), 1);
        assert_eq!(page.event_count, full.event_count);
        assert_eq!(page.message_count, full.message_count);
        assert_eq!(page.turn_count, full.turn_count);
        assert_eq!(page.turns.len(), 1);
        assert_eq!(
            page.turns[0].provider_turn_id.as_deref(),
            Some(expected_turn_id.as_str())
        );
        assert_eq!(
            page.turns[0].confidence,
            crate::session_projection::TurnConfidence::Exact
        );

        let empty = KimiProvider
            .import_session_page(source_path, full.event_count, Some(0))
            .unwrap();
        assert!(empty.imported.session.events.is_empty());
        assert!(empty.turns.is_empty());
        assert_eq!(empty.event_count, full.event_count);
        assert_eq!(empty.message_count, full.message_count);
        assert_eq!(empty.turn_count, full.turn_count);

        let context_only_dir = sessions_root
            .join("0017cc2b0eee031e9194d1384b4bcdd8")
            .join("22222222-2222-4222-8222-222222222222");
        let context_only = KimiProvider
            .import_session_page(context_only_dir.to_str().unwrap(), 0, None)
            .unwrap();
        assert_eq!(context_only.turns.len(), 1);
        assert_eq!(context_only.turns[0].provider_turn_id, None);
        assert_eq!(
            context_only.turns[0].confidence,
            crate::session_projection::TurnConfidence::Inferred
        );
    }

    #[test]
    fn session_index_and_detail_dispatch_are_idempotent_source_backed_and_bodyless() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let _home_guard = TestConfigHomeGuard::new(&home);
        let sessions_root = dir.path().join("sessions");
        let _kimi_guard = use_test_kimi_sessions_dir(sessions_root.clone());
        let session_id = "33333333-3333-4333-8333-333333333333";
        let session_dir = write_native_kimi_fixture(&sessions_root, "project-index", session_id);
        let summary = KimiProvider
            .scan_sessions()
            .unwrap()
            .into_iter()
            .find(|summary| summary.session_id == session_id)
            .unwrap();
        assert_eq!(
            summary.source_path.as_deref(),
            Some(session_dir.to_string_lossy().as_ref())
        );
        let fingerprint = KimiProvider
            .session_source_fingerprint(summary.source_path.as_deref().unwrap())
            .unwrap()
            .unwrap();
        let full = KimiProvider
            .import_session_page(summary.source_path.as_deref().unwrap(), 0, None)
            .unwrap();
        let expected_turn_count = full.turn_count.unwrap();

        let mut conn = local_store::open_database().unwrap();
        let first = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                PROVIDER_ID,
                &summary,
                KimiProvider.capabilities(),
                &fingerprint,
            )
            .unwrap();
        let counts_after_first: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'kimi'),
                    (SELECT COUNT(*) FROM sessions WHERE provider_id = 'kimi'),
                    (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'kimi'),
                    (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'kimi')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        let second = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                PROVIDER_ID,
                &summary,
                KimiProvider.capabilities(),
                &fingerprint,
            )
            .unwrap();
        let counts_after_second: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'kimi'),
                    (SELECT COUNT(*) FROM sessions WHERE provider_id = 'kimi'),
                    (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'kimi'),
                    (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'kimi')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
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
            )
            .unwrap();
        assert_eq!(source_path, session_dir.to_string_lossy());
        assert_eq!(storage_shape, "directory");
        assert_eq!(source_cursor, fingerprint.value);
        let snapshot_json: String = conn
            .query_row(
                "SELECT snapshot_json FROM session_snapshots WHERE session_id = ?1",
                [&first.canonical_session_id],
                |row| row.get(0),
            )
            .unwrap();
        let snapshot_json: Value = serde_json::from_str(&snapshot_json).unwrap();
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
        let body_table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table'
                   AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(body_table_count, 0);
        drop(conn);

        let detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0)).unwrap();
        assert!(detail.events.is_empty());
        assert!(detail.turns.is_empty());
        assert_eq!(detail.event_count, full.event_count);
        assert_eq!(detail.message_count, full.message_count);
        assert_eq!(
            detail.source_path.as_deref(),
            Some(session_dir.to_string_lossy().as_ref())
        );
        assert_eq!(
            detail.projection_report.as_ref().unwrap().id,
            format!("source-read:{PROVIDER_ID}:{session_id}")
        );

        let conn = local_store::open_database().unwrap();
        let cached_counts: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT event_count, message_count, turn_count, counts_complete
                 FROM session_snapshots WHERE session_id = ?1",
                [&first.canonical_session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(cached_counts.0, full.event_count as i64);
        assert_eq!(cached_counts.1, full.message_count as i64);
        assert_eq!(cached_counts.2, expected_turn_count as i64);
        assert_eq!(cached_counts.3, 1);
        drop(conn);

        std::fs::remove_dir_all(&session_dir).unwrap();
        let groups = crate::core::list_sessions(&crate::core::SessionListParams {
            all: true,
            providers: vec![PROVIDER_ID.to_string()],
            cwd: None,
            include_message_counts: true,
            limit: None,
            offset: None,
            sort: crate::core::SessionListSort::Recent,
            hook_filter: crate::core::SessionHookFilter::All,
        })
        .unwrap();
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
    }

    #[test]
    fn import_canonical_session_reconciles_context_with_wire_lifecycle() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let session_dir = temp.path().join("project-hash").join("kimi-session-1");
        std::fs::create_dir_all(&session_dir)?;
        let wire_path = session_dir.join("wire.jsonl");
        let context_path = session_dir.join("context.jsonl");
        let state_path = session_dir.join("state.json");
        std::fs::write(
            &context_path,
            concat!(
                "{\"role\":\"_system_prompt\",\"content\":\"system\"}\n",
                "{\"role\":\"_checkpoint\",\"id\":0}\n",
                "{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello\"},{\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,abc\"}}]}\n",
                "{\"role\":\"_usage\",\"token_count\":42}\n",
                "{\"role\":\"assistant\",\"content\":[{\"type\":\"think\",\"think\":\"reasoning\",\"encrypted\":null},{\"type\":\"text\",\"text\":\"answer\"},{\"type\":\"custom\",\"payload\":{\"kept\":true}}]}\n"
            ),
        )?;
        std::fs::write(
            &state_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "custom_title": "Kimi Title",
                "archived": false,
                "todos": [{"content": "keep raw state"}]
            }))?,
        )?;
        let mut wire_file = File::create(&wire_path)?;
        for value in [
            serde_json::json!({"type": "metadata", "protocol_version": "1.9"}),
            serde_json::json!({
                "timestamp": 1710000001.0,
                "message": {
                    "type": "TurnBegin",
                    "payload": {"user_input": [
                        {"type": "text", "text": "hello"},
                        {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
                    ]}
                }
            }),
            serde_json::json!({
                "timestamp": 1710000002.0,
                "message": {"type": "StepBegin", "payload": {"n": 1}}
            }),
            serde_json::json!({
                "timestamp": 1710000003.0,
                "message": {"type": "ContentPart", "payload": {"type": "think", "think": "reasoning"}}
            }),
            serde_json::json!({
                "timestamp": 1710000004.0,
                "message": {"type": "ContentPart", "payload": {"type": "text", "text": "answer"}}
            }),
            serde_json::json!({
                "timestamp": 1710000004.5,
                "message": {"type": "ContentPart", "payload": {"type": "custom", "payload": {"kept": true}}}
            }),
            serde_json::json!({
                "timestamp": 1710000005.0,
                "message": {"type": "StatusUpdate", "payload": {"context_tokens": 42}}
            }),
            serde_json::json!({
                "timestamp": 1710000006.0,
                "message": {"type": "TurnEnd", "payload": {}}
            }),
            serde_json::json!({
                "timestamp": 1710000007.0,
                "message": {"type": "FutureRecord", "payload": {"kept": true}}
            }),
        ] {
            writeln!(wire_file, "{value}")?;
        }

        let imported = import_canonical_session_from_dir(&session_dir)?;

        assert_eq!(imported.session.identity.canonical_id, "kimi-session-1");
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Kimi Title")
        );
        assert!(imported.session.extensions.contains_key("kimi_state"));
        assert_eq!(
            imported.session.extensions["kimi_wire_metadata"][0]["protocol_version"],
            "1.9"
        );
        assert!(!imported.session.events.iter().any(|event| {
            event.blocks.iter().any(
                |block| matches!(block, EventBlock::ProviderPayload { kind, .. } if kind == "metadata"),
            )
        }));

        let visible = imported
            .session
            .events
            .iter()
            .filter_map(|event| {
                canonical_event_visible_message_text(event).map(|text| (event.role, text))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            visible,
            vec![
                (
                    EventRole::User,
                    "hello\n[Image: image/png]\ndata:image/png;base64,abc".to_string()
                ),
                (EventRole::Assistant, "reasoning\nanswer".to_string()),
            ]
        );
        let user = imported
            .session
            .events
            .iter()
            .find(|event| event.role == EventRole::User)
            .unwrap();
        assert!(user.blocks.iter().any(
            |block| matches!(block, EventBlock::Image { data: Some(data), .. } if data == "data:image/png;base64,abc")
        ));
        let assistant = imported
            .session
            .events
            .iter()
            .find(|event| {
                event.role == EventRole::Assistant && event.kind == SessionEventKind::Message
            })
            .unwrap();
        assert!(assistant.blocks.iter().any(
            |block| matches!(block, EventBlock::Thinking { text, .. } if text == "reasoning")
        ));
        assert!(assistant.blocks.iter().any(
            |block| matches!(block, EventBlock::ProviderPayload { kind, .. } if kind == "custom")
        ));

        let turn_begin = imported
            .session
            .events
            .iter()
            .find(|event| provider_payload_kind(event) == Some("TurnBegin"))
            .unwrap();
        let turn_end = imported
            .session
            .events
            .iter()
            .find(|event| provider_payload_kind(event) == Some("TurnEnd"))
            .unwrap();
        assert_eq!(turn_begin.links.turn_boundary, Some(TurnBoundary::Started));
        assert_eq!(turn_end.links.turn_boundary, Some(TurnBoundary::Completed));
        assert_eq!(
            turn_begin.links.provider_turn_id,
            turn_end.links.provider_turn_id
        );
        assert_eq!(
            user.links.provider_turn_id,
            turn_begin.links.provider_turn_id
        );
        assert_eq!(
            assistant.links.provider_turn_id,
            turn_begin.links.provider_turn_id
        );
        for kind in ["StepBegin", "StatusUpdate", "custom", "FutureRecord"] {
            assert!(imported
                .session
                .events
                .iter()
                .any(|event| provider_payload_kind(event) == Some(kind)));
        }
        assert_eq!(imported.report.overall, MappingDisposition::Preserved);
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "provider_block_preserved"));
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "provider_wire_message_preserved"));
        Ok(())
    }

    #[test]
    fn sanitized_kimi_fixture_imports_context_authoritatively_with_native_turns() {
        let dir = tempdir().unwrap();
        copy_kimi_audit_fixture(dir.path());
        let sessions_root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
        let session_dir = sessions_root
            .join("2030c6ce97e98c160351b18f097eb584")
            .join("11111111-1111-4111-8111-111111111111");

        let imported = KimiProvider
            .import_session(session_dir.to_str().unwrap())
            .unwrap();
        let visible = imported
            .session
            .events
            .iter()
            .filter_map(|event| {
                canonical_event_visible_message_text(event).map(|text| (event.role, text))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            visible,
            vec![
                (EventRole::User, "[sanitized user request]".to_string()),
                (
                    EventRole::Assistant,
                    "[sanitized reasoning]\n[sanitized assistant response]".to_string()
                ),
                (EventRole::User, "[sanitized follow-up]".to_string()),
                (
                    EventRole::Assistant,
                    "[sanitized follow-up response]".to_string()
                ),
            ]
        );
        assert_eq!(
            imported.session.extensions["kimi_wire_metadata"][0]["protocol_version"],
            "1.3"
        );
        assert!(imported
            .session
            .events
            .iter()
            .any(|event| provider_payload_kind(event) == Some("StepBegin")));
        assert!(imported
            .session
            .events
            .iter()
            .any(|event| provider_payload_kind(event) == Some("StatusUpdate")));
        let turn_ids = imported
            .session
            .events
            .iter()
            .filter(|event| matches!(event.role, EventRole::User | EventRole::Assistant))
            .filter_map(|event| event.links.provider_turn_id.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(turn_ids.len(), 4);
        assert_eq!(turn_ids[0], turn_ids[1]);
        assert_eq!(turn_ids[2], turn_ids[3]);
        assert_ne!(turn_ids[0], turn_ids[2]);
        assert_eq!(imported.report.overall, MappingDisposition::Preserved);
        assert!(imported.report.issues.is_empty());
    }

    #[test]
    fn malformed_wire_line_is_reported_without_losing_context_messages() {
        let dir = tempdir().unwrap();
        copy_kimi_audit_fixture(dir.path());
        let sessions_root = dir.path().join("sessions");
        let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
        let session_dir = sessions_root
            .join("2030c6ce97e98c160351b18f097eb584")
            .join("11111111-1111-4111-8111-111111111111");
        std::fs::copy(
            dir.path().join("variants/wire.malformed.jsonl"),
            session_dir.join("wire.jsonl"),
        )
        .unwrap();

        let imported = KimiProvider
            .import_session(session_dir.to_str().unwrap())
            .unwrap();
        assert_eq!(
            imported
                .session
                .events
                .iter()
                .filter_map(canonical_event_visible_message_text)
                .collect::<Vec<_>>(),
            vec![
                "[sanitized user request]".to_string(),
                "[sanitized reasoning]\n[sanitized assistant response]".to_string(),
                "[sanitized follow-up]".to_string(),
                "[sanitized follow-up response]".to_string(),
            ]
        );
        assert!(imported
            .report
            .issues
            .iter()
            .any(|issue| issue.code == "invalid_wire_jsonl_line"
                && issue.disposition == MappingDisposition::Dropped));
        assert!(imported
            .session
            .events
            .iter()
            .any(|event| provider_payload_kind(event) == Some("TurnBegin")));
        assert!(imported
            .session
            .events
            .iter()
            .any(|event| provider_payload_kind(event) == Some("TurnEnd")));
        assert_eq!(imported.report.overall, MappingDisposition::Dropped);
    }

    #[test]
    fn compressed_segment_exports_as_portable_kimi_text_part() {
        let block = EventBlock::Compressed {
            source_provider_id: "opencode".to_string(),
            summary: "compressed summary".to_string(),
            source_event_ids: vec![
                "old-event-1".to_string(),
                "old-event-2".to_string(),
                "old-event-3".to_string(),
            ],
            source_event_count: None,
            archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
        };

        let part = canonical_block_to_kimi_content_part(&block).expect("kimi text part");
        let text = part
            .get("text")
            .and_then(Value::as_str)
            .expect("portable compressed text");

        assert_eq!(part.get("type").and_then(Value::as_str), Some("text"));
        assert!(text.contains("[Compressed session segment from opencode]"));
        assert!(text.contains("compressed summary"));
        assert!(text.contains("Source event count: 3"));
        assert!(text.contains("Archive: memorph-archive://s1/archive.json.gz"));
        assert!(text.contains("memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"));
        assert!(!text.contains("old-event-1"));
        assert!(!text.contains("old-event-2"));
        assert!(!text.contains("old-event-3"));
    }

    #[test]
    fn provider_payload_block_is_skipped_in_kimi_text_part_export() {
        let block = EventBlock::ProviderPayload {
            kind: "custom".to_string(),
            payload: serde_json::json!({"kept": true}),
        };

        assert!(canonical_block_to_kimi_content_part(&block).is_none());
    }
}
