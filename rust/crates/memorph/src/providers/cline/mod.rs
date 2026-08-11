pub mod adapter;
pub mod hook;

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
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct ClineProvider;
const PROVIDER_ID: &str = "cline";
const HISTORY_FILE: &str = "api_conversation_history.json";

impl Provider for ClineProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "Cline"
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
                provider_payload: Some(Fidelity::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            resume_quality: ResumeQuality::None,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: false,
                sidecar_files: true,
                index_repair: true,
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
        for root in cline_task_roots() {
            if !root.exists() {
                continue;
            }
            for entry in WalkDir::new(root)
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
            {
                let path = entry.path();
                if path.file_name().and_then(|value| value.to_str()) != Some(HISTORY_FILE) {
                    continue;
                }
                let Some(task_id) = task_id_from_history_path(path) else {
                    continue;
                };
                let Ok(value) = read_history(path) else {
                    continue;
                };
                let title = first_text(&value);
                sessions.push(ProviderSessionSummary {
                    session_id: task_id,
                    title,
                    project_dir: task_workspace(path),
                    created_at: None,
                    last_active_at: file_modified_ms(path),
                    source_path: Some(path.to_string_lossy().into_owned()),
                });
            }
        }
        sessions.sort_by_key(|session| std::cmp::Reverse(session.last_active_at.unwrap_or(0)));
        sessions.dedup_by(|left, right| left.session_id == right.session_id);
        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let path = canonical_path(Path::new(source_path))?;
        let task_id = task_id_from_history_path(&path)
            .context("Cline source must be tasks/<taskId>/api_conversation_history.json")?;
        let value = read_history(&path)?;
        let timestamp = file_modified_datetime(&path).unwrap_or_else(Utc::now);
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let events = history_events(&value, timestamp, &mut report);
        let event_meta = events
            .iter()
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect::<Vec<_>>();
        let title = first_text(&value);
        let mut extensions = BTreeMap::new();
        extensions.insert("cline_api_conversation_history".into(), value);
        Ok(ImportedSession {
            session: Session {
                lineage: Vec::new(),
                schema: Schema::default(),
                identity: Identity {
                    id: task_id.clone(),
                    title,
                },
                context: Context {
                    workspace: task_workspace(&path),
                    created_at: Some(timestamp),
                    last_active_at: Some(timestamp),
                    tags: Vec::new(),
                },
                events,
                extensions,
            },
            provenance: Provenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".into()),
                primary_source: ProviderRef {
                    provider_id: PROVIDER_ID.into(),
                    session_id: task_id,
                    source_path: Some(path.to_string_lossy().into_owned()),
                },
                aliases: Vec::new(),
            },
            event_meta,
            report,
        })
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        let path = canonical_path(Path::new(source_path))?;
        if !path.exists() {
            return Ok(None);
        }
        let metadata = std::fs::metadata(&path)?;
        let digest = Sha256::digest(std::fs::read(&path)?);
        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms: file_modified_ms(&path).unwrap_or(0),
            size_bytes: metadata.len().min(i64::MAX as u64) as i64,
            value: format!(
                "cline-task-history-v1:{}:{digest:x}",
                task_id_from_history_path(&path).unwrap_or_default()
            ),
        }))
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        cline_task_roots()
    }
}

fn cline_task_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    const EXTENSION_IDS: &[&str] = &[
        "saoudrizwan.claude-dev",
        "rooveterinaryinc.roo-cline",
        "kilocode.kilo-code",
    ];
    #[cfg(target_os = "macos")]
    for app in ["Code", "Code - Insiders", "VSCodium", "Cursor"] {
        if let Some(home) = dirs::home_dir() {
            for ext in EXTENSION_IDS {
                roots.push(
                    home.join("Library/Application Support")
                        .join(app)
                        .join("User/globalStorage")
                        .join(ext)
                        .join("tasks"),
                );
            }
        }
    }
    #[cfg(target_os = "linux")]
    for app in ["Code", "Code - Insiders", "VSCodium", "Cursor"] {
        if let Some(home) = dirs::home_dir() {
            for ext in EXTENSION_IDS {
                roots.push(
                    home.join(".config")
                        .join(app)
                        .join("User/globalStorage")
                        .join(ext)
                        .join("tasks"),
                );
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Documents/Cline/tasks"));
    }
    roots
}

fn canonical_path(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path)
        .with_context(|| format!("Cline source does not exist: {}", path.display()))
}
fn read_history(path: &Path) -> Result<Value> {
    serde_json::from_str(&std::fs::read_to_string(path)?)
        .with_context(|| format!("invalid Cline history: {}", path.display()))
}
fn task_id_from_history_path(path: &Path) -> Option<String> {
    path.parent()?
        .file_name()?
        .to_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
fn task_workspace(path: &Path) -> Option<String> {
    let metadata = path.parent()?.join("task_metadata.json");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(metadata).ok()?).ok()?;
    ["cwd", "workspace", "workspaceDir", "rootPath"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(str::to_string))
}
fn file_modified_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
}
fn file_modified_datetime(path: &Path) -> Option<DateTime<Utc>> {
    file_modified_ms(path).and_then(DateTime::<Utc>::from_timestamp_millis)
}
fn first_text(value: &Value) -> Option<String> {
    value.as_array()?.iter().find_map(|item| {
        message_blocks(item)
            .into_iter()
            .find_map(|block| match block {
                Block::Text { text } if !text.trim().is_empty() => Some(text),
                _ => None,
            })
    })
}

fn history_events(
    value: &Value,
    timestamp: DateTime<Utc>,
    report: &mut MappingReport,
) -> Vec<Event> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, item)| {
            let role_raw = item
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let role = match role_raw {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                "system" => Role::System,
                _ => Role::Other,
            };
            let blocks = message_blocks(item);
            if blocks.is_empty() {
                return None;
            }
            let kind = if blocks
                .iter()
                .any(|block| matches!(block, Block::ToolCall { .. }))
            {
                EventKind::Action
            } else if blocks
                .iter()
                .any(|block| matches!(block, Block::ToolResult { .. }))
            {
                EventKind::Observation
            } else {
                EventKind::Message
            };
            report.push_issue(crate::session::MappingIssue {
                level: crate::session::MappingIssueLevel::Info,
                disposition: Fidelity::Preserved,
                code: "cline-native-block".into(),
                message: "Mapped current Cline API conversation history block".into(),
                path: Some(format!("messages[{index}]")),
                raw: None,
            });
            Some(Event {
                id: item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("cline:message:{index}")),
                kind,
                role,
                timestamp,
                links: Links::default(),
                blocks,
                tags: Vec::new(),
                extensions: Default::default(),
                metadata: Metadata {
                    model: None,
                    usage: None,
                },
            })
        })
        .collect()
}

fn message_blocks(item: &Value) -> Vec<Block> {
    let content = item.get("content").unwrap_or(item);
    if let Some(text) = content.as_str() {
        return vec![Block::Text { text: text.into() }];
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(
            |block| match block.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" => block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| Block::Text { text: text.into() }),
                "thinking" => {
                    block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .map(|text| Block::Thinking {
                            text: text.into(),
                            signature: None,
                        })
                }
                "tool_use" => Some(Block::ToolCall {
                    tool_call_id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .into(),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .into(),
                    input: block.get("input").cloned(),
                }),
                "tool_result" => Some(Block::ToolResult {
                    tool_call_id: block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .into(),
                    content: block.get("content").map(value_text).unwrap_or_default(),
                    outcome: crate::session::execution_outcome(
                        block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    ),
                }),
                _ => None,
            },
        )
        .collect()
}
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capabilities_match_native_read_only_contract() {
        let capabilities = ClineProvider.capabilities();
        assert!(capabilities.scan);
        assert!(capabilities.import);
        assert!(!capabilities.export);
        assert!(!capabilities.delete);
        assert!(!capabilities.rename);
        assert!(!capabilities.resume);
        assert_eq!(capabilities.storage_shape, StorageShape::Directory);
        assert_eq!(capabilities.scan_strategy, ScanStrategy::FullScan);
        assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);
        assert_eq!(capabilities.resume_quality, ResumeQuality::None);
        assert!(!capabilities.backup_support.before_write);
        assert!(!capabilities.backup_support.restore);
    }
    #[test]
    fn task_identity_comes_from_parent_directory() {
        let path = Path::new("/tmp/tasks/task-123/api_conversation_history.json");
        assert_eq!(task_id_from_history_path(path).as_deref(), Some("task-123"));
    }
    #[test]
    fn maps_current_cline_content_blocks() {
        let value = serde_json::json!({"role":"assistant","content":[{"type":"thinking","thinking":"reason"},{"type":"tool_use","id":"call-1","name":"bash","input":{"cmd":"pwd"}}]});
        let blocks = message_blocks(&value);
        assert!(matches!(blocks[0], Block::Thinking { .. }));
        assert!(matches!(blocks[1], Block::ToolCall { ref name, .. } if name == "bash"));
    }
}
