pub mod adapter;
pub mod hook;

use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance, TurnBoundary, UsageStats,
};
use crate::provider::{
    canonical_block_text, canonical_event_is_visible_message, canonical_event_visible_message_role,
    canonical_event_visible_text, canonical_export_result, canonical_session_title, PageStrategy,
    Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionBackup, ProviderSessionImportPage,
    ProviderSessionSummary, ProviderSourceMutation, ProviderWriteRisk, ResumeQuality, ScanStrategy,
    StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::session_projection::project_session_turns;
use crate::storage::event_index;
use crate::utils::{
    encode_project_dir, extract_text, parse_timestamp_to_ms, path_basename, truncate_summary,
};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct ClaudeProvider;

const PROVIDER_ID: &str = "claude";
const TITLE_MAX_CHARS: usize = 80;
const CLAUDE_BACKUP_FORMAT: &str = "claude-session-backup-v1";
const CLAUDE_BACKUP_MIME: &str = "application/vnd.memorph.claude-session-backup";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClaudeSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    session_path: PathBuf,
    sidecar_path: PathBuf,
    capture_sidecar: bool,
    sidecar_present: bool,
}

impl Provider for ClaudeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "claude"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: true,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::IndexedPage,
            storage_shape: StorageShape::Jsonl,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Preserved),
                tool_call: Some(MappingDisposition::Preserved),
                tool_result: Some(MappingDisposition::Preserved),
                patch: Some(MappingDisposition::Unsupported),
                image: Some(MappingDisposition::Downgraded),
                file: Some(MappingDisposition::Downgraded),
                compressed: Some(MappingDisposition::Unsupported),
                provider_payload: Some(MappingDisposition::Preserved),
            },
            export_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Preserved),
                tool_call: Some(MappingDisposition::Preserved),
                tool_result: Some(MappingDisposition::Preserved),
                patch: Some(MappingDisposition::Downgraded),
                image: Some(MappingDisposition::Downgraded),
                file: Some(MappingDisposition::Downgraded),
                compressed: Some(MappingDisposition::Downgraded),
                provider_payload: Some(MappingDisposition::Dropped),
            },
            resume_quality: ResumeQuality::Native,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::Medium,
                multiple_files: false,
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
                runtime_endpoint: true,
                session_activity: true,
            },
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let root = get_claude_config_dir().join("projects");
        if !root.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in WalkDir::new(&root)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // Skip agent sub-sessions
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("agent-"))
                .unwrap_or(false)
            {
                continue;
            }
            if let Some(meta) = parse_session(path) {
                sessions.push(meta);
            }
        }

        Ok(sessions)
    }

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        let root = get_claude_config_dir().join("projects");
        if !root.exists() {
            return Ok(None);
        }

        for entry in WalkDir::new(&root)
            .max_depth(3)
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
                .map(|n| n.starts_with("agent-"))
                .unwrap_or(false)
            {
                continue;
            }
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                return Ok(parse_session(path));
            }
        }

        Ok(None)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        import_canonical_session(Path::new(source_path))
    }

    fn import_session_page(
        &self,
        source_path: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<ProviderSessionImportPage> {
        import_claude_session_page(Path::new(source_path), event_offset, event_limit)
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
        let projects_dir = get_claude_config_dir().join("projects");
        if !projects_dir.exists() {
            anyhow::bail!("Claude projects directory not found");
        }

        // Find the session file
        let mut found = false;
        for entry in WalkDir::new(&projects_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                // Remove the JSONL file
                std::fs::remove_file(path).with_context(|| {
                    format!("Failed to remove session file: {}", path.display())
                })?;
                // Remove sidecar directory if exists
                if let Some(parent) = path.parent() {
                    let sidecar = parent.join(session_id);
                    if sidecar.exists() {
                        std::fs::remove_dir_all(&sidecar).with_context(|| {
                            format!("Failed to remove sidecar directory: {}", sidecar.display())
                        })?;
                    }
                }
                found = true;
                break;
            }
        }

        if !found {
            anyhow::bail!("Claude session not found: {}", session_id);
        }

        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let projects_dir = get_claude_config_dir().join("projects");
        if !projects_dir.exists() {
            anyhow::bail!("Claude projects directory not found");
        }

        // Find the session file
        let mut found_path: Option<PathBuf> = None;
        for entry in WalkDir::new(&projects_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                found_path = Some(path.to_path_buf());
                break;
            }
        }

        let path =
            found_path.with_context(|| format!("Claude session not found: {}", session_id))?;

        // Rewrite the JSONL with updated custom-title line
        let content = std::fs::read_to_string(&path)?;
        let mut new_lines = Vec::new();
        let mut title_updated = false;

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut value: Value = serde_json::from_str(line)?;
            if value.get("type").and_then(|v| v.as_str()) == Some("custom-title") {
                if let Value::Object(ref mut map) = value {
                    map.insert(
                        "customTitle".to_string(),
                        Value::String(new_title.to_string()),
                    );
                    title_updated = true;
                }
                new_lines.push(serde_json::to_string(&value)?);
            } else {
                new_lines.push(line.to_string());
            }
        }

        // If no custom-title line found, prepend one after permission-mode
        if !title_updated {
            let title_line = serde_json::json!({
                "type": "custom-title",
                "customTitle": new_title,
                "sessionId": session_id,
            });
            // Insert after permission-mode line if present, else at beginning
            let mut inserted = false;
            for (i, line) in new_lines.iter().enumerate() {
                let v: Value = serde_json::from_str(line)?;
                if v.get("type").and_then(|v| v.as_str()) == Some("permission-mode") {
                    new_lines.insert(i + 1, serde_json::to_string(&title_line)?);
                    inserted = true;
                    break;
                }
            }
            if !inserted {
                new_lines.insert(0, serde_json::to_string(&title_line)?);
            }
        }

        std::fs::write(&path, new_lines.join("\n") + "\n")?;
        Ok(())
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        create_claude_session_backup(
            &get_claude_config_dir().join("projects"),
            mutation,
            operation_id,
            session_id,
            backup_root,
        )
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        restore_claude_session_backup(backup)
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("claude --resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let path = Path::new(session_id);
        if path.exists() {
            Ok(std::fs::metadata(path)?.len())
        } else {
            Ok(0)
        }
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        vec![get_claude_config_dir()]
    }
}

fn get_claude_config_dir() -> PathBuf {
    crate::config::effective_home_dir()
        .map(|h| h.join(".claude"))
        .unwrap_or_else(|_| PathBuf::from(".claude"))
}

fn create_claude_session_backup(
    projects_dir: &Path,
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    let session_path = find_claude_session_file(projects_dir, session_id)?
        .with_context(|| format!("Claude session not found: {session_id}"))?
        .canonicalize()
        .with_context(|| format!("Failed to resolve Claude session source: {session_id}"))?;
    let sidecar_path = session_path
        .parent()
        .context("Claude session source has no parent directory")?
        .join(session_id);
    let capture_sidecar = mutation == ProviderSourceMutation::Delete;
    let sidecar_present = capture_sidecar && sidecar_path.exists();

    let provider_backup_root = backup_root.join(PROVIDER_ID);
    std::fs::create_dir_all(&provider_backup_root).with_context(|| {
        format!(
            "Failed to create Claude backup root: {}",
            provider_backup_root.display()
        )
    })?;
    let backup_path = provider_backup_root.join(operation_id);
    std::fs::create_dir(&backup_path).with_context(|| {
        format!(
            "Failed to create Claude session backup: {}",
            backup_path.display()
        )
    })?;
    std::fs::copy(&session_path, backup_path.join("session.jsonl")).with_context(|| {
        format!(
            "Failed to back up Claude session file: {}",
            session_path.display()
        )
    })?;
    if sidecar_present {
        copy_claude_sidecar(&sidecar_path, &backup_path.join("sidecar"))?;
    }

    let metadata = ClaudeSessionBackupMetadata {
        version: 1,
        provider_id: PROVIDER_ID.to_string(),
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        session_path: session_path.clone(),
        sidecar_path: sidecar_path.clone(),
        capture_sidecar,
        sidecar_present,
    };
    std::fs::write(
        backup_path.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)?,
    )
    .with_context(|| {
        format!(
            "Failed to write Claude backup metadata: {}",
            backup_path.display()
        )
    })?;

    Ok(ProviderSessionBackup {
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        source_path: session_path,
        backup_path,
        restore_hint:
            "Restore this backup with memorph's Claude native session restore flow before reopening Claude."
                .to_string(),
        mime_type: CLAUDE_BACKUP_MIME.to_string(),
        format: CLAUDE_BACKUP_FORMAT.to_string(),
        artifact_metadata: serde_json::json!({
            "role": "claude_prewrite_session_backup",
            "mutation": mutation,
            "sidecar_captured": capture_sidecar,
            "sidecar_present": sidecar_present,
        }),
        restore_metadata: serde_json::json!({
            "restore_mode": "claude_session_restore",
            "metadata_file": "metadata.json",
            "mutation": mutation,
        }),
    })
}

fn restore_claude_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != CLAUDE_BACKUP_FORMAT {
        anyhow::bail!(
            "Unsupported Claude session backup format: {}",
            backup.format
        );
    }
    let metadata_path = backup.backup_path.join("metadata.json");
    let metadata: ClaudeSessionBackupMetadata =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).with_context(|| {
            format!(
                "Failed to read Claude backup metadata: {}",
                metadata_path.display()
            )
        })?)?;
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.mutation != backup.mutation
        || metadata.session_path != backup.source_path
    {
        anyhow::bail!(
            "Claude backup metadata does not match the registered restore context: {}",
            backup.backup_path.display()
        );
    }

    if let Some(parent) = metadata.session_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(
        backup.backup_path.join("session.jsonl"),
        &metadata.session_path,
    )
    .with_context(|| {
        format!(
            "Failed to restore Claude session file: {}",
            metadata.session_path.display()
        )
    })?;

    if metadata.capture_sidecar {
        if metadata.sidecar_path.exists() {
            std::fs::remove_dir_all(&metadata.sidecar_path).with_context(|| {
                format!(
                    "Failed to replace Claude sidecar during restore: {}",
                    metadata.sidecar_path.display()
                )
            })?;
        }
        if metadata.sidecar_present {
            copy_claude_sidecar(&backup.backup_path.join("sidecar"), &metadata.sidecar_path)?;
        }
    }
    Ok(())
}

fn find_claude_session_file(projects_dir: &Path, session_id: &str) -> Result<Option<PathBuf>> {
    if !projects_dir.exists() {
        return Ok(None);
    }
    for entry in WalkDir::new(projects_dir)
        .max_depth(3)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl")
            && path.file_stem().and_then(|stem| stem.to_str()) == Some(session_id)
        {
            return Ok(Some(path.to_path_buf()));
        }
    }
    Ok(None)
}

fn copy_claude_sidecar(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir(destination).with_context(|| {
        format!(
            "Failed to create Claude sidecar copy: {}",
            destination.display()
        )
    })?;
    for entry in WalkDir::new(source).follow_links(false).into_iter().skip(1) {
        let entry = entry
            .with_context(|| format!("Failed to walk Claude sidecar: {}", source.display()))?;
        let relative = entry.path().strip_prefix(source)?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir(&target)?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &target).with_context(|| {
                format!(
                    "Failed to copy Claude sidecar file: {}",
                    entry.path().display()
                )
            })?;
        } else if entry.file_type().is_symlink() {
            copy_claude_sidecar_symlink(entry.path(), &target)?;
        } else {
            anyhow::bail!(
                "Claude sidecar contains unsupported entry: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_claude_sidecar_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = std::fs::read_link(source)?;
    std::os::unix::fs::symlink(target, destination)?;
    Ok(())
}

#[cfg(windows)]
fn copy_claude_sidecar_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = std::fs::read_link(source)?;
    let resolved_target = source
        .parent()
        .map(|parent| parent.join(&target))
        .unwrap_or_else(|| target.clone());
    if resolved_target.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination)?;
    } else {
        std::os::windows::fs::symlink_file(target, destination)?;
    }
    Ok(())
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

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let encoded_dir = encode_project_dir(&target_dir.to_string_lossy());
    let claude_projects_dir = get_claude_config_dir().join("projects").join(&encoded_dir);
    std::fs::create_dir_all(&claude_projects_dir)?;

    let file_path = claude_projects_dir.join(format!("{}.jsonl", session_id));
    let sidecar_dir = claude_projects_dir.join(&session_id);
    std::fs::create_dir_all(&sidecar_dir)?;
    std::fs::create_dir_all(sidecar_dir.join("tool-results"))?;
    std::fs::create_dir_all(sidecar_dir.join("subagents"))?;

    let mut file = File::create(&file_path)?;
    let title = canonical_session_title(session);
    let project_dir_str = target_dir.to_string_lossy().to_string();
    let version = "2.1.116";
    let git_branch = get_git_branch(target_dir).unwrap_or_else(|| "main".to_string());

    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "type": "permission-mode",
            "permissionMode": "bypassPermissions",
            "sessionId": session_id,
        }))?
    )?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "type": "custom-title",
            "customTitle": title,
            "sessionId": session_id,
        }))?
    )?;

    let mut prev_uuid: Option<String> = None;
    for event in &session.events {
        let Some(message_role) = claude_message_role(event) else {
            continue;
        };
        let Some(content) = canonical_event_to_claude_message_content(event) else {
            continue;
        };
        let msg_uuid = Uuid::new_v4().to_string();
        let timestamp = event.timestamp.to_rfc3339();
        let line_type = message_role;

        let mut message = serde_json::json!({
            "role": message_role,
            "content": content,
        });
        if event.role == EventRole::Assistant {
            message["id"] = Value::String(format!(
                "msg_{}",
                Uuid::new_v4()
                    .to_string()
                    .replace("-", "")
                    .chars()
                    .take(20)
                    .collect::<String>()
            ));
            message["type"] = Value::String("message".to_string());
            if let Some(model) = &event.metadata.model {
                message["model"] = Value::String(model.clone());
            }
            message["stop_reason"] = Value::String(
                if event
                    .blocks
                    .iter()
                    .any(|block| matches!(block, EventBlock::ToolCall { .. }))
                {
                    "tool_use"
                } else {
                    "end_turn"
                }
                .to_string(),
            );
            if let Some(usage) = &event.metadata.usage {
                message["usage"] = serde_json::json!({
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                });
            }
        }

        let line = serde_json::json!({
            "parentUuid": prev_uuid,
            "isSidechain": false,
            "message": message,
            "type": line_type,
            "uuid": msg_uuid,
            "timestamp": timestamp,
            "userType": "external",
            "entrypoint": "cli",
            "cwd": project_dir_str,
            "sessionId": session_id,
            "version": version,
            "gitBranch": git_branch,
        });
        writeln!(file, "{}", serde_json::to_string(&line)?)?;
        prev_uuid = Some(msg_uuid);
    }

    Ok(session_id)
}

fn claude_message_role(event: &SessionEvent) -> Option<&'static str> {
    let role = canonical_event_visible_message_role(event)?;
    Some(if role == EventRole::Assistant {
        "assistant"
    } else {
        "user"
    })
}

fn canonical_event_to_claude_message_content(event: &SessionEvent) -> Option<Value> {
    if event.role == EventRole::Assistant {
        let content = event
            .blocks
            .iter()
            .filter_map(canonical_block_to_claude_content)
            .collect::<Vec<_>>();
        return (!content.is_empty()).then_some(Value::Array(content));
    }

    if event
        .blocks
        .iter()
        .all(|block| matches!(block, EventBlock::ToolResult { .. }))
    {
        let content = event
            .blocks
            .iter()
            .filter_map(canonical_block_to_claude_content)
            .collect::<Vec<_>>();
        return (!content.is_empty()).then_some(Value::Array(content));
    }

    let text = canonical_event_visible_text(event);
    (!text.trim().is_empty()).then_some(Value::String(text))
}

fn canonical_block_to_claude_content(block: &EventBlock) -> Option<Value> {
    match block {
        EventBlock::Text { text } => Some(serde_json::json!({
            "type": "text",
            "text": text,
        })),
        EventBlock::Thinking { text, signature } => {
            let mut value = serde_json::json!({
                "type": "thinking",
                "thinking": text,
            });
            if let Some(signature) = signature {
                value["signature"] = Value::String(signature.clone());
            }
            Some(value)
        }
        EventBlock::ToolCall {
            tool_call_id,
            name,
            input,
        } => {
            let mut value = serde_json::json!({
                "type": "tool_use",
                "id": tool_call_id,
                "name": name,
            });
            if let Some(input) = input {
                value["input"] = input.clone();
            }
            Some(value)
        }
        EventBlock::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => Some(serde_json::json!({
            "type": "tool_result",
            "tool_use_id": tool_call_id,
            "content": content,
            "is_error": is_error,
        })),
        EventBlock::ProviderPayload { .. } => None,
        _ => {
            let text = canonical_block_text(block);
            (!text.trim().is_empty()).then(|| {
                serde_json::json!({
                    "type": "text",
                    "text": text,
                })
            })
        }
    }
}

fn import_canonical_session(path: &Path) -> Result<ImportedSession> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Claude session: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut source_title: Option<String> = None;
    let mut created_at: Option<chrono::DateTime<Utc>> = None;
    let mut last_active_at: Option<chrono::DateTime<Utc>> = None;
    let mut extensions = BTreeMap::new();

    for (line_idx, line) in reader.lines().enumerate() {
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
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Claude session line: {}", error),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };

        let timestamp = claude_line_timestamp(&value, line_idx + 1);
        created_at = created_at.or(Some(timestamp));
        last_active_at = Some(timestamp);

        session_id = value
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(session_id);
        project_dir = value
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(project_dir);

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if line_type == "custom-title" {
            source_title = value
                .get("customTitle")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string)
                .or(source_title);
            extensions.insert("claude_custom_title".to_string(), value.clone());
        }

        if let Some(event) = canonical_event_from_claude_line(
            line_idx + 1,
            line_type,
            timestamp,
            &value,
            &mut report,
        ) {
            events.push(event);
        }
    }

    let fallback_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_session_id = session_id.unwrap_or(fallback_id);

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: source_session_id.clone(),
                source_title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: source_session_id,
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

/// Build (or load) a byte-offset event index for a Claude session and return the
/// requested page of canonical events plus full event/message counts.
///
/// Claude JSONL sessions append one JSON object per line; every non-empty valid
/// line produces exactly one canonical event, so the index records one location
/// per line. The index is persisted in the shared `session_event_index` table
/// and reused as long as the source file fingerprint is unchanged, mirroring the
/// Codex provider. Counts and page slicing are therefore stable across detail
/// views without re-parsing the full session on every open.
fn import_claude_session_page(
    path: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let (state, locations) =
        load_or_build_claude_event_index_page(path, event_offset, event_limit)?;

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::with_capacity(locations.len());
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open Claude session: {}", path.display()))?;
    let mut extensions = BTreeMap::new();

    for location in locations {
        file.seek(SeekFrom::Start(location.byte_offset))
            .with_context(|| format!("Failed to seek Claude session: {}", path.display()))?;
        let mut line_bytes = vec![0u8; location.byte_length as usize];
        file.read_exact(&mut line_bytes)
            .with_context(|| format!("Failed to read Claude session: {}", path.display()))?;
        let line = String::from_utf8_lossy(&line_bytes);
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Claude session line: {}", error),
                    path: Some(format!("line:{}", location.line_no)),
                    raw: Some(Value::String(line.into_owned())),
                });
                continue;
            }
        };

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if line_type == "custom-title" {
            extensions.insert("claude_custom_title".to_string(), value.clone());
        }

        let timestamp = claude_line_timestamp(&value, location.line_no);
        if let Some(event) = canonical_event_from_claude_line(
            location.line_no,
            line_type,
            timestamp,
            &value,
            &mut report,
        ) {
            events.push(event);
        }
    }

    let imported = ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: state.session_id.clone(),
                source_title: state.source_title.clone(),
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: state.session_id.clone(),
                    source_path: Some(path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: state.workspace_dir.clone(),
                created_at: state
                    .created_at_ms
                    .and_then(chrono::DateTime::from_timestamp_millis),
                last_active_at: state
                    .last_active_at_ms
                    .and_then(chrono::DateTime::from_timestamp_millis),
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    };

    let turns = project_session_turns(
        &imported.session.identity.canonical_id,
        &imported.session.events,
        TurnQuality::Inferred,
    );
    Ok(ProviderSessionImportPage {
        imported,
        event_count: state.event_count,
        message_count: state.message_count,
        turns,
    })
}

fn load_or_build_claude_event_index_page(
    path: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<(
    event_index::IndexedSessionState,
    Vec<event_index::IndexedEventLocation>,
)> {
    let source_path = path.to_string_lossy().to_string();
    let fingerprint = event_index::source_file_fingerprint(path)?;
    let mut conn = event_index::open_database()?;
    let mut state =
        match event_index::load_fresh_session_state(&conn, PROVIDER_ID, &source_path, fingerprint)?
        {
            Some(state) => state,
            None => {
                let (state, locations) = build_claude_event_index(path, fingerprint)?;
                event_index::replace_session_index(&mut conn, &state, &locations)?;
                state
            }
        };

    let mut locations = event_index::load_event_locations(
        &conn,
        PROVIDER_ID,
        &source_path,
        fingerprint,
        event_offset,
        event_limit,
    )?;

    // If the requested range is empty but counts disagree with the file (e.g.
    // the index predates an append since the last build because the fingerprint
    // check above matched a stale entry), rebuild once. The fingerprint guard
    // above normally makes this a no-op, but rebuilding keeps counts correct
    // when the index was populated by an older build that did not record the
    // same count semantics.
    let needs_rebuild = locations.is_empty() && event_offset < state.event_count;
    if needs_rebuild {
        let (rebuilt_state, rebuilt_locations) = build_claude_event_index(path, fingerprint)?;
        event_index::replace_session_index(&mut conn, &rebuilt_state, &rebuilt_locations)?;
        state = rebuilt_state;
        locations = event_index::load_event_locations(
            &conn,
            PROVIDER_ID,
            &source_path,
            fingerprint,
            event_offset,
            event_limit,
        )?;
    }

    Ok((state, locations))
}

/// Single-pass index builder for a Claude JSONL session.
///
/// Records one `IndexedEventLocation` per non-empty valid line (every such line
/// yields exactly one canonical event). `event_count`/`message_count` are
/// computed by reusing the same canonical mapping used at import time, so the
/// counts stay identical to a full import. Claude has no native turn ids, so
/// turn fields are left empty and turns are derived per page at read time.
fn build_claude_event_index(
    path: &Path,
    fingerprint: event_index::SourceFileFingerprint,
) -> Result<(
    event_index::IndexedSessionState,
    Vec<event_index::IndexedEventLocation>,
)> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Claude session: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut byte_offset = 0u64;
    let mut line_no = 0usize;
    let mut event_count = 0usize;
    let mut message_count = 0usize;
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at_ms: Option<i64> = None;
    let mut last_active_at_ms: Option<i64> = None;
    let mut source_title: Option<String> = None;
    let mut locations = Vec::new();
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);

    loop {
        line.clear();
        let byte_length = reader
            .read_line(&mut line)
            .with_context(|| format!("Failed to read Claude session: {}", path.display()))?;
        if byte_length == 0 {
            break;
        }
        line_no += 1;
        let line_offset = byte_offset;
        byte_offset += byte_length as u64;

        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let timestamp = claude_line_timestamp(&value, line_no);
        let timestamp_ms = timestamp.timestamp_millis();
        created_at_ms = created_at_ms.or(Some(timestamp_ms));
        last_active_at_ms = Some(timestamp_ms);

        session_id = value
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(session_id);
        project_dir = value
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or(project_dir);

        if line_type == "custom-title" {
            source_title = value
                .get("customTitle")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string)
                .or(source_title);
        }

        // Reuse the canonical mapper to decide visible-message membership so
        // counts stay identical to a full import.
        if let Some(event) =
            canonical_event_from_claude_line(line_no, line_type, timestamp, &value, &mut report)
        {
            if canonical_event_is_visible_message(&event) {
                message_count += 1;
            }
        }

        locations.push(event_index::IndexedEventLocation {
            event_index: event_count,
            byte_offset: line_offset,
            byte_length: byte_length as u64,
            line_no,
            provider_turn_id: None,
            turn_index: None,
            turn_boundary: None,
        });
        event_count += 1;
    }

    let fallback_id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_session_id = session_id.unwrap_or(fallback_id);

    Ok((
        event_index::IndexedSessionState {
            provider_id: PROVIDER_ID.to_string(),
            session_id: source_session_id,
            source_path: path.to_string_lossy().to_string(),
            file_fingerprint: fingerprint,
            workspace_dir: project_dir,
            created_at_ms,
            last_active_at_ms,
            source_title,
            event_count,
            message_count,
        },
        locations,
    ))
}

fn claude_line_timestamp(value: &Value, line_number: usize) -> chrono::DateTime<Utc> {
    value
        .get("timestamp")
        .and_then(parse_timestamp_to_ms)
        .and_then(chrono::DateTime::from_timestamp_millis)
        .unwrap_or_else(|| {
            chrono::DateTime::from_timestamp_millis(line_number as i64)
                .expect("Claude source line number is a valid timestamp")
        })
}

fn canonical_event_from_claude_line(
    line_number: usize,
    line_type: &str,
    timestamp: chrono::DateTime<Utc>,
    value: &Value,
    report: &mut MappingReport,
) -> Option<SessionEvent> {
    let event_id = value
        .get("uuid")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("claude:line:{}", line_number));
    let parent_id = value
        .get("parentUuid")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let Some(message) = value.get("message") else {
        return Some(provider_payload_event(
            event_id,
            SessionEventKind::Lifecycle,
            EventRole::System,
            timestamp,
            line_type,
            value.clone(),
            value.clone(),
            parent_id,
        ));
    };

    let role = claude_event_role(line_type, message, value);
    let blocks = claude_event_blocks(message.get("content"), value, line_number, report);
    if blocks.is_empty() {
        return Some(provider_payload_event(
            event_id,
            SessionEventKind::Unknown,
            role,
            timestamp,
            line_type,
            value.clone(),
            value.clone(),
            parent_id,
        ));
    }

    let kind = claude_event_kind(&blocks);
    let model = message
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let usage = message.get("usage").map(|usage| UsageStats {
        input_tokens: usage.get("input_tokens").and_then(|v| v.as_u64()),
        output_tokens: usage.get("output_tokens").and_then(|v| v.as_u64()),
        total_tokens: usage.get("total_tokens").and_then(|v| v.as_u64()),
    });

    Some(SessionEvent {
        id: event_id,
        kind,
        role,
        timestamp,
        links: EventLinks {
            parent_event_id: parent_id.clone(),
            provider_parent_id: parent_id,
            provider_turn_id: None,
            turn_index: None,
            turn_boundary: claude_turn_boundary(message),
            related_event_ids: Vec::new(),
        },
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: value
                    .get("uuid")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                original_role: message
                    .get("role")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| Some(line_type.to_string())),
                phase: Some(line_type.to_string()),
            },
            model,
            usage,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("claude_raw_line".to_string(), value.clone());
                ext
            },
        },
    })
}

fn claude_turn_boundary(message: &Value) -> Option<TurnBoundary> {
    match message.get("stop_reason").and_then(Value::as_str) {
        Some("end_turn" | "stop_sequence") => Some(TurnBoundary::Completed),
        Some("max_tokens") => Some(TurnBoundary::Interrupted),
        _ => None,
    }
}

fn claude_event_role(line_type: &str, message: &Value, raw: &Value) -> EventRole {
    match line_type {
        "assistant" => EventRole::Assistant,
        "user" => {
            if let Some(Value::Array(items)) = message.get("content") {
                let all_tool_results = !items.is_empty()
                    && items.iter().all(|item| {
                        item.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                    });
                if all_tool_results {
                    return EventRole::Tool;
                }
            }
            EventRole::User
        }
        _ => match message.get("role").and_then(|v| v.as_str()) {
            Some("assistant") => EventRole::Assistant,
            Some("user") => {
                if raw
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        !items.is_empty()
                            && items.iter().all(|item| {
                                item.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                            })
                    })
                    .unwrap_or(false)
                {
                    EventRole::Tool
                } else {
                    EventRole::User
                }
            }
            Some("system") => EventRole::System,
            _ => EventRole::Unknown,
        },
    }
}

fn claude_event_blocks(
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
            .map(|(idx, item)| claude_content_block(item, line_number, idx, report))
            .collect(),
        Some(other) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Normalized,
                code: "claude_content_shape_preserved".to_string(),
                message: "Claude message content was preserved as an unknown block because it was neither a string nor an array".to_string(),
                path: Some(format!("line:{}:content", line_number)),
                raw: Some(other.clone()),
            });
            vec![EventBlock::Unknown { raw: other.clone() }]
        }
        None => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Normalized,
                code: "claude_content_missing".to_string(),
                message: "Claude message had no content; the raw message event was preserved"
                    .to_string(),
                path: Some(format!("line:{}:content", line_number)),
                raw: Some(raw_line.clone()),
            });
            Vec::new()
        }
    }
}

fn claude_content_block(
    value: &Value,
    line_number: usize,
    block_index: usize,
    report: &mut MappingReport,
) -> EventBlock {
    match value.get("type").and_then(|v| v.as_str()) {
        Some("text") if value.get("text").and_then(Value::as_str).is_some() => EventBlock::Text {
            text: value
                .get("text")
                .and_then(Value::as_str)
                .unwrap()
                .to_string(),
        },
        Some("thinking") if value.get("thinking").and_then(Value::as_str).is_some() => {
            EventBlock::Thinking {
                text: value
                    .get("thinking")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
                signature: value
                    .get("signature")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            }
        }
        Some("tool_use")
            if value.get("id").and_then(Value::as_str).is_some()
                && value.get("name").and_then(Value::as_str).is_some()
                && value.get("input").is_some() =>
        {
            EventBlock::ToolCall {
                tool_call_id: value.get("id").and_then(Value::as_str).unwrap().to_string(),
                name: value
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
                input: value.get("input").cloned(),
            }
        }
        Some("tool_result")
            if value.get("tool_use_id").and_then(Value::as_str).is_some()
                && value.get("content").is_some() =>
        {
            EventBlock::ToolResult {
                tool_call_id: value
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
                content: value
                    .get("content")
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap(),
                is_error: value
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }
        }
        Some("text" | "thinking" | "tool_use" | "tool_result") => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Normalized,
                code: "claude_malformed_content_block".to_string(),
                message: "Malformed Claude content block was preserved as provider payload"
                    .to_string(),
                path: Some(format!("line:{}:block:{}", line_number, block_index)),
                raw: Some(value.clone()),
            });
            EventBlock::ProviderPayload {
                kind: value
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap()
                    .to_string(),
                payload: value.clone(),
            }
        }
        Some(kind) => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Preserved,
                code: "provider_block_preserved".to_string(),
                message: format!("Preserved unsupported Claude content block '{}'", kind),
                path: Some(format!("line:{}:block:{}", line_number, block_index)),
                raw: Some(value.clone()),
            });
            EventBlock::ProviderPayload {
                kind: kind.to_string(),
                payload: value.clone(),
            }
        }
        None => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: MappingDisposition::Normalized,
                code: "claude_block_missing_type".to_string(),
                message: "Claude content block without a type was preserved as unknown".to_string(),
                path: Some(format!("line:{}:block:{}", line_number, block_index)),
                raw: Some(value.clone()),
            });
            EventBlock::Unknown { raw: value.clone() }
        }
    }
}

fn claude_event_kind(blocks: &[EventBlock]) -> SessionEventKind {
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

fn provider_payload_event(
    id: String,
    kind: SessionEventKind,
    role: EventRole,
    timestamp: chrono::DateTime<Utc>,
    payload_kind: &str,
    payload: Value,
    raw_line: Value,
    parent_id: Option<String>,
) -> SessionEvent {
    SessionEvent {
        id,
        kind,
        role,
        timestamp,
        links: EventLinks {
            parent_event_id: parent_id.clone(),
            provider_parent_id: parent_id,
            provider_turn_id: None,
            turn_index: None,
            turn_boundary: None,
            related_event_ids: Vec::new(),
        },
        blocks: vec![EventBlock::ProviderPayload {
            kind: payload_kind.to_string(),
            payload,
        }],
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: Some(payload_kind.to_string()),
                phase: Some(payload_kind.to_string()),
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("claude_raw_line".to_string(), raw_line);
                ext
            },
        },
    }
}

fn parse_session(path: &Path) -> Option<ProviderSessionSummary> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().filter_map(Result::ok).collect();

    let head = lines.iter().take(20).collect::<Vec<_>>();
    let tail = lines.iter().rev().take(30).collect::<Vec<_>>();

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut first_user_message: Option<String> = None;
    let mut custom_title: Option<String> = None;

    for line in &head {
        let value: Value = serde_json::from_str(line).ok()?;
        if custom_title.is_none()
            && value.get("type").and_then(|v| v.as_str()) == Some("custom-title")
        {
            custom_title = value
                .get("customTitle")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if session_id.is_none() {
            session_id = value
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        if project_dir.is_none() {
            project_dir = value
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if first_user_message.is_none() {
            let is_user = value.get("type").and_then(|v| v.as_str()) == Some("user")
                || value
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|v| v.as_str())
                    == Some("user");
            if is_user {
                if let Some(message) = value.get("message") {
                    let text = extract_text(message.get("content")?);
                    let trimmed = text.trim();
                    if !trimmed.is_empty()
                        && !trimmed.contains("<local-command-caveat>")
                        && !trimmed.starts_with("<command-name>")
                    {
                        first_user_message = Some(trimmed.to_string());
                    }
                }
            }
        }
        if session_id.is_some()
            && project_dir.is_some()
            && created_at.is_some()
            && first_user_message.is_some()
        {
            break;
        }
    }

    let mut last_active_at: Option<i64> = None;
    let mut summary: Option<String> = None;

    for line in &tail {
        let value: Value = serde_json::from_str(line).ok()?;
        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if custom_title.is_none()
            && value.get("type").and_then(|v| v.as_str()) == Some("custom-title")
        {
            custom_title = value
                .get("customTitle")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if summary.is_none() {
            if value.get("isMeta").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            if let Some(message) = value.get("message") {
                let text = extract_text(message.get("content")?);
                if !text.trim().is_empty() {
                    summary = Some(text);
                }
            }
        }
        if last_active_at.is_some() && summary.is_some() && custom_title.is_some() {
            break;
        }
    }

    let session_id = session_id.or_else(|| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .map(|stem| stem.to_string())
    })?;

    let title = custom_title
        .map(|t| truncate_summary(&t, TITLE_MAX_CHARS))
        .or_else(|| first_user_message.map(|t| truncate_summary(&t, TITLE_MAX_CHARS)))
        .or_else(|| {
            project_dir
                .as_deref()
                .and_then(path_basename)
                .map(|v| v.to_string())
        });

    let _summary = summary.map(|text| truncate_summary(&text, 160));

    Some(ProviderSessionSummary {
        session_id: session_id.clone(),
        title,
        project_dir,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::BTreeMap;
    use tempfile::NamedTempFile;

    fn write_native_claude_session(
        projects_dir: &Path,
        session_id: &str,
        content: &[u8],
    ) -> PathBuf {
        let project_dir = projects_dir.join("-tmp-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join(format!("{session_id}.jsonl"));
        std::fs::write(&session_path, content).unwrap();
        session_path
    }

    fn build_structured_claude_session() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "permission-mode",
                "permissionMode": "bypassPermissions",
                "sessionId": "session-page",
                "timestamp": "2026-01-01T00:00:00Z"
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "custom-title",
                "customTitle": "Claude Page Title",
                "sessionId": "session-page",
                "timestamp": "2026-01-01T00:00:01Z"
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "user",
                "uuid": "user-1",
                "sessionId": "session-page",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:02Z",
                "message": { "role": "user", "content": "Build this" }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-1",
                "parentUuid": "user-1",
                "sessionId": "session-page",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:03Z",
                "message": {
                    "role": "assistant",
                    "model": "claude-sonnet",
                    "content": [{ "type": "text", "text": "Hello" }]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "user",
                "uuid": "tool-result-1",
                "parentUuid": "assistant-1",
                "sessionId": "session-page",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:04Z",
                "message": {
                    "role": "user",
                    "content": [
                        { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok", "is_error": false }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "file-history-snapshot",
                "sessionId": "session-page",
                "timestamp": "2026-01-01T00:00:05Z",
                "files": [{ "path": "Cargo.toml" }]
            })
        )
        .unwrap();
        file
    }

    #[test]
    fn import_session_page_reports_full_counts_and_paginates_events() {
        assert_eq!(
            ClaudeProvider.capabilities().page_strategy,
            PageStrategy::IndexedPage
        );
        let file = build_structured_claude_session();

        // Full page: counts must match a full import, every line yields one event.
        let full = import_claude_session_page(file.path(), 0, None).unwrap();
        let full_import = import_canonical_session(file.path()).unwrap();
        assert_eq!(full.event_count, full_import.session.events.len());
        assert_eq!(full.imported.session.events.len(), full.event_count);
        // Three visible messages: user "Build this", assistant text, tool result.
        let expected_messages = full_import
            .session
            .events
            .iter()
            .filter(|event| canonical_event_is_visible_message(event))
            .count();
        assert_eq!(full.message_count, expected_messages);
        assert!(full.message_count >= 1);

        // Page with a limit returns a strict subset but keeps total counts.
        let page1 = import_claude_session_page(file.path(), 0, Some(2)).unwrap();
        assert_eq!(page1.imported.session.events.len(), 2);
        assert_eq!(page1.event_count, full.event_count);
        assert_eq!(page1.message_count, full.message_count);
        assert_eq!(
            page1.imported.session.events[0].id,
            full.imported.session.events[0].id
        );

        // Second page starts at offset 2.
        let page2 = import_claude_session_page(file.path(), 2, Some(2)).unwrap();
        assert_eq!(page2.imported.session.events.len(), 2);
        assert_eq!(page2.event_count, full.event_count);
        assert_eq!(
            page2.imported.session.events[0].id,
            full.imported.session.events[2].id
        );

        // Identity carries the session id and title for every page.
        assert_eq!(page1.imported.session.identity.canonical_id, "session-page");
        assert_eq!(
            page1.imported.session.identity.source_title.as_deref(),
            Some("Claude Page Title")
        );
        assert_eq!(
            page1.imported.session.context.workspace_dir.as_deref(),
            Some("/tmp/project")
        );
    }

    #[test]
    fn delete_backup_restores_exact_claude_session_and_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let projects_dir = dir.path().join("projects");
        let backup_root = dir.path().join("backups");
        let session_id = "session-delete-1";
        let original_session = b"{\"type\":\"user\",\"message\":\"exact bytes\"}\n\n";
        let session_path = write_native_claude_session(&projects_dir, session_id, original_session);
        let sidecar_path = session_path.parent().unwrap().join(session_id);
        std::fs::create_dir_all(sidecar_path.join("nested")).unwrap();
        std::fs::write(sidecar_path.join("index.json"), b"{\"version\":1}\n").unwrap();
        std::fs::write(
            sidecar_path.join("nested").join("state.bin"),
            [0, 1, 2, 255],
        )
        .unwrap();

        let backup = create_claude_session_backup(
            &projects_dir,
            ProviderSourceMutation::Delete,
            "operation-delete-1",
            session_id,
            &backup_root,
        )
        .unwrap();

        std::fs::remove_file(&session_path).unwrap();
        std::fs::remove_dir_all(&sidecar_path).unwrap();
        std::fs::write(&session_path, b"partial provider rewrite").unwrap();
        std::fs::create_dir_all(&sidecar_path).unwrap();
        std::fs::write(sidecar_path.join("unexpected"), b"remove me").unwrap();

        restore_claude_session_backup(&backup).unwrap();

        assert_eq!(std::fs::read(&session_path).unwrap(), original_session);
        assert_eq!(
            std::fs::read(sidecar_path.join("index.json")).unwrap(),
            b"{\"version\":1}\n"
        );
        assert_eq!(
            std::fs::read(sidecar_path.join("nested").join("state.bin")).unwrap(),
            [0, 1, 2, 255]
        );
        assert!(!sidecar_path.join("unexpected").exists());
    }

    #[test]
    fn rename_backup_restores_jsonl_without_replacing_claude_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let projects_dir = dir.path().join("projects");
        let backup_root = dir.path().join("backups");
        let session_id = "session-rename-1";
        let original_session = b"{\"type\":\"custom-title\",\"customTitle\":\"Before\"}\n";
        let session_path = write_native_claude_session(&projects_dir, session_id, original_session);
        let sidecar_path = session_path.parent().unwrap().join(session_id);
        std::fs::create_dir_all(&sidecar_path).unwrap();
        std::fs::write(sidecar_path.join("live-state"), b"before").unwrap();

        let backup = create_claude_session_backup(
            &projects_dir,
            ProviderSourceMutation::Rename,
            "operation-rename-1",
            session_id,
            &backup_root,
        )
        .unwrap();

        std::fs::write(&session_path, b"renamed bytes").unwrap();
        std::fs::write(sidecar_path.join("live-state"), b"changed after backup").unwrap();
        std::fs::write(sidecar_path.join("new-state"), b"keep me").unwrap();

        restore_claude_session_backup(&backup).unwrap();

        assert_eq!(std::fs::read(&session_path).unwrap(), original_session);
        assert_eq!(
            std::fs::read(sidecar_path.join("live-state")).unwrap(),
            b"changed after backup"
        );
        assert_eq!(
            std::fs::read(sidecar_path.join("new-state")).unwrap(),
            b"keep me"
        );
        assert!(!backup.backup_path.join("sidecar").exists());
    }

    #[test]
    fn claude_backup_contract_and_capabilities_are_truthful() {
        let dir = tempfile::tempdir().unwrap();
        let projects_dir = dir.path().join("projects");
        let backup_root = dir.path().join("backups");
        let session_id = "session-contract-1";
        let session_path =
            write_native_claude_session(&projects_dir, session_id, b"{\"type\":\"user\"}\n");

        let backup = create_claude_session_backup(
            &projects_dir,
            ProviderSourceMutation::Delete,
            "operation-contract-1",
            session_id,
            &backup_root,
        )
        .unwrap();

        let capabilities = ClaudeProvider.capabilities();
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert_eq!(backup.mutation, ProviderSourceMutation::Delete);
        assert_eq!(backup.operation_id, "operation-contract-1");
        assert_eq!(backup.provider_session_id, session_id);
        assert_eq!(backup.source_path, session_path.canonicalize().unwrap());
        assert_eq!(backup.format, CLAUDE_BACKUP_FORMAT);
        assert_eq!(backup.mime_type, CLAUDE_BACKUP_MIME);
        assert_eq!(
            backup
                .restore_metadata
                .get("restore_mode")
                .and_then(Value::as_str),
            Some("claude_session_restore")
        );

        let metadata: ClaudeSessionBackupMetadata = serde_json::from_str(
            &std::fs::read_to_string(backup.backup_path.join("metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata.version, 1);
        assert_eq!(metadata.provider_id, PROVIDER_ID);
        assert_eq!(metadata.mutation, ProviderSourceMutation::Delete);
        assert_eq!(metadata.operation_id, "operation-contract-1");
        assert_eq!(metadata.provider_session_id, session_id);
        assert_eq!(metadata.session_path, backup.source_path);
        assert!(metadata.capture_sidecar);
        assert!(!metadata.sidecar_present);
    }

    fn test_event(
        kind: SessionEventKind,
        role: EventRole,
        blocks: Vec<EventBlock>,
    ) -> SessionEvent {
        SessionEvent {
            id: "test-event".to_string(),
            kind,
            role,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks,
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
        }
    }

    #[test]
    fn maps_claude_end_turn_to_completed_boundary() {
        let raw = serde_json::json!({
            "type": "assistant",
            "uuid": "assistant-1",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "Done"}],
                "stop_reason": "end_turn"
            }
        });
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);

        let event = canonical_event_from_claude_line(1, "assistant", Utc::now(), &raw, &mut report)
            .unwrap();

        assert_eq!(event.links.provider_turn_id, None);
        assert_eq!(event.links.turn_boundary, Some(TurnBoundary::Completed));
    }

    #[test]
    fn import_canonical_session_preserves_claude_structured_and_meta_lines() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "permission-mode",
                "permissionMode": "bypassPermissions",
                "sessionId": "session-1",
                "timestamp": "2026-01-01T00:00:00Z"
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "custom-title",
                "customTitle": "Claude Title",
                "sessionId": "session-1",
                "timestamp": "2026-01-01T00:00:01Z"
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "user",
                "uuid": "user-1",
                "sessionId": "session-1",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:02Z",
                "message": {
                    "role": "user",
                    "content": "Build this"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-1",
                "parentUuid": "user-1",
                "sessionId": "session-1",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:03Z",
                "message": {
                    "role": "assistant",
                    "model": "claude-sonnet",
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 20
                    },
                    "content": [
                        {
                            "type": "thinking",
                            "thinking": "Thinking",
                            "signature": "sig"
                        },
                        {
                            "type": "tool_use",
                            "id": "toolu_1",
                            "name": "Read",
                            "input": { "file_path": "Cargo.toml" }
                        }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "user",
                "uuid": "tool-result-1",
                "parentUuid": "assistant-1",
                "sessionId": "session-1",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:04Z",
                "message": {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "toolu_1",
                            "content": "contents",
                            "is_error": false
                        }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "file-history-snapshot",
                "sessionId": "session-1",
                "timestamp": "2026-01-01T00:00:05Z",
                "files": [{ "path": "Cargo.toml" }]
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();

        assert_eq!(imported.session.identity.canonical_id, "session-1");
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Claude Title")
        );
        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some("/tmp/project")
        );
        assert!(imported.session.events.iter().any(|event| {
            event.kind == SessionEventKind::Lifecycle
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ProviderPayload { kind, .. })
                        if kind == "file-history-snapshot"
                )
        }));
        let assistant = imported
            .session
            .events
            .iter()
            .find(|event| event.id == "assistant-1")
            .unwrap();
        assert_eq!(assistant.kind, SessionEventKind::ToolCall);
        assert!(matches!(
            assistant.blocks.first(),
            Some(EventBlock::Thinking {
                text,
                signature: Some(signature)
            }) if text == "Thinking" && signature == "sig"
        ));
        assert!(assistant.blocks.iter().any(|block| matches!(
            block,
            EventBlock::ToolCall {
                tool_call_id,
                name,
                ..
            } if tool_call_id == "toolu_1" && name == "Read"
        )));
        let tool_result = imported
            .session
            .events
            .iter()
            .find(|event| event.id == "tool-result-1")
            .unwrap();
        assert_eq!(tool_result.role, EventRole::Tool);
        assert!(matches!(
            tool_result.blocks.first(),
            Some(EventBlock::ToolResult {
                tool_call_id,
                content,
                is_error
            }) if tool_call_id == "toolu_1" && content == "contents" && !is_error
        ));
    }

    #[test]
    fn import_canonical_session_uses_stable_source_order_for_missing_timestamps() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "user",
                "uuid": "user-1",
                "sessionId": "session-stable",
                "message": { "role": "user", "content": "First" }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-1",
                "sessionId": "session-stable",
                "timestamp": "not-a-timestamp",
                "message": { "role": "assistant", "content": "Second" }
            })
        )
        .unwrap();
        file.flush().unwrap();

        let first = import_canonical_session(file.path()).unwrap();
        let second = import_canonical_session(file.path()).unwrap();

        assert_eq!(
            serde_json::to_value(&first.session.events).unwrap(),
            serde_json::to_value(&second.session.events).unwrap()
        );
        assert_eq!(first.session.events[0].timestamp.timestamp_millis(), 1);
        assert_eq!(first.session.events[1].timestamp.timestamp_millis(), 2);
        assert_eq!(
            first.session.context.created_at.unwrap().timestamp_millis(),
            1
        );
        assert_eq!(
            first
                .session
                .context
                .last_active_at
                .unwrap()
                .timestamp_millis(),
            2
        );
    }

    #[test]
    fn compressed_segment_exports_as_portable_claude_text_block() {
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

        let content = canonical_block_to_claude_content(&block).expect("claude text block");
        let text = content
            .get("text")
            .and_then(Value::as_str)
            .expect("portable compressed text");

        assert_eq!(content.get("type").and_then(Value::as_str), Some("text"));
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
    fn provider_payload_block_is_skipped_in_export() {
        let block = EventBlock::ProviderPayload {
            kind: "function_call".to_string(),
            payload: serde_json::json!({ "name": "shell" }),
        };
        assert!(canonical_block_to_claude_content(&block).is_none());
    }

    #[test]
    fn lifecycle_events_are_skipped_in_claude_export() {
        let event = test_event(
            SessionEventKind::Lifecycle,
            EventRole::Assistant,
            vec![
                EventBlock::Text {
                    text: "Done.".to_string(),
                },
                EventBlock::ProviderPayload {
                    kind: "task_complete".to_string(),
                    payload: serde_json::json!({
                        "type": "task_complete",
                        "last_agent_message": "Done."
                    }),
                },
            ],
        );

        assert!(claude_message_role(&event).is_none());
        assert!(canonical_event_to_claude_message_content(&event).is_some());
    }

    #[test]
    fn developer_events_are_skipped_in_claude_export() {
        let event = test_event(
            SessionEventKind::Message,
            EventRole::Developer,
            vec![EventBlock::Text {
                text: "<model_switch>internal</model_switch>".to_string(),
            }],
        );

        assert!(claude_message_role(&event).is_none());
    }

    #[test]
    fn assistant_tool_calls_export_as_structured_tool_use() {
        let event = test_event(
            SessionEventKind::ToolCall,
            EventRole::Assistant,
            vec![EventBlock::ToolCall {
                tool_call_id: "call_1".to_string(),
                name: "exec_command".to_string(),
                input: Some(serde_json::json!({
                    "cmd": "git status --short"
                })),
            }],
        );

        let content = canonical_event_to_claude_message_content(&event).unwrap();
        let items = content.as_array().expect("structured assistant content");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("type").and_then(Value::as_str),
            Some("tool_use")
        );
        assert_eq!(items[0].get("id").and_then(Value::as_str), Some("call_1"));
        assert_eq!(
            items[0].get("name").and_then(Value::as_str),
            Some("exec_command")
        );
    }

    #[test]
    fn non_assistant_text_fallback_omits_provider_payload_blocks() {
        let event = test_event(
            SessionEventKind::Message,
            EventRole::User,
            vec![
                EventBlock::Text {
                    text: "Build this".to_string(),
                },
                EventBlock::ProviderPayload {
                    kind: "token_count".to_string(),
                    payload: serde_json::json!({
                        "input_tokens": 10,
                        "output_tokens": 20
                    }),
                },
            ],
        );

        let content = canonical_event_to_claude_message_content(&event).unwrap();
        assert_eq!(content.as_str(), Some("Build this"));
    }

    #[test]
    fn malformed_claude_content_blocks_are_preserved_and_reported() {
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let block = claude_content_block(
            &serde_json::json!({"type": "tool_use", "name": "Read"}),
            7,
            2,
            &mut report,
        );

        assert!(matches!(
            block,
            EventBlock::ProviderPayload { ref kind, ref payload }
                if kind == "tool_use" && payload == &serde_json::json!({"type": "tool_use", "name": "Read"})
        ));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "claude_malformed_content_block"));
    }

    #[test]
    fn claude_content_shape_is_preserved_and_reported() {
        let raw_line = serde_json::json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": {"unexpected": true}}
        });
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let blocks = claude_event_blocks(
            raw_line
                .get("message")
                .and_then(|message| message.get("content")),
            &raw_line,
            8,
            &mut report,
        );

        assert!(matches!(
            blocks.as_slice(),
            [EventBlock::Unknown { raw }] if raw == &serde_json::json!({"unexpected": true})
        ));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "claude_content_shape_preserved"));
    }
}
