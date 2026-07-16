pub mod adapter;
pub mod hook;

use crate::canonical::ImportedSession;
use crate::provider::{
    Provider, ProviderCapabilities, ProviderSessionSummary, ProviderSourceFingerprint,
    ScanStrategy, StorageShape,
};
use anyhow::{Context, Result};
use chrono::DateTime;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
            import: false,
            scan_strategy: ScanStrategy::FullScan,
            storage_shape: StorageShape::Directory,
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

    fn import_session(&self, _source_path: &str) -> Result<ImportedSession> {
        anyhow::bail!("Canonical import is not implemented for the current Kiro session format")
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
    use serde_json::{json, Value};
    use std::fs;
    use std::sync::{MutexGuard, OnceLock};

    static TEST_KIRO_TEST_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

    struct TestKiroSessionsDirGuard {
        _lock: MutexGuard<'static, ()>,
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
        assert!(!capabilities.import);
        assert!(!capabilities.export);
        assert!(!capabilities.delete);
        assert!(!capabilities.rename);
        assert!(!capabilities.resume);
        assert_eq!(capabilities.scan_strategy, ScanStrategy::FullScan);
        assert_eq!(capabilities.page_strategy, PageStrategy::Unknown);
        assert_eq!(capabilities.storage_shape, StorageShape::Directory);
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
        assert!(KiroProvider
            .import_session(source_path.to_str().unwrap())
            .unwrap_err()
            .to_string()
            .contains("current Kiro session format"));
        assert_eq!(KiroProvider.data_source_paths(), vec![temp.path()]);
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
