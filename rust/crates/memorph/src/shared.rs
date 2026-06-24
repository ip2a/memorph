use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::canonical::CanonicalSession;
#[cfg(test)]
use crate::core::compression;
use crate::providers;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedGroup {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub source_provider: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub holdings: Vec<Holding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Holding {
    pub id: String,
    pub provider: String,
    pub session_id: String,
    pub target_dir: Option<String>,
    pub created_at: i64,
    pub last_active_at: Option<i64>,
    pub last_sync_at: Option<i64>,
    pub last_sync_from: Option<String>,
    pub last_error: Option<String>,
}

// ---------------------------------------------------------------------------
// Params / Results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareCreateParams {
    pub provider: String,
    pub session_id: String,
    pub targets: Vec<String>,
    pub to_dir: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddHoldingParams {
    pub group_id: String,
    pub provider: String,
    pub session_id: Option<String>,
    pub to_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReport {
    pub source_provider: String,
    pub source_holding_id: String,
    pub success: Vec<String>,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn shared_dir() -> Result<PathBuf> {
    let config_dir = crate::config::config_path()?
        .parent()
        .context("Config file path has no parent directory")?
        .to_path_buf();
    let dir = config_dir.join("shared");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn group_path(id: &str) -> Result<PathBuf> {
    Ok(shared_dir()?.join(format!("{}.json", id)))
}

pub fn list_groups() -> Result<Vec<SharedGroup>> {
    let dir = shared_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut groups = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Warning: failed to read directory entry: {}", e);
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Warning: failed to read {}: {}", path.display(), e);
                continue;
            }
        };
        let group: SharedGroup = match serde_json::from_str(&content) {
            Ok(g) => g,
            Err(e) => {
                eprintln!(
                    "Warning: failed to parse sync group {}: {}",
                    path.display(),
                    e
                );
                continue;
            }
        };
        groups.push(group);
    }

    groups.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(groups)
}

pub fn load_group(id: &str) -> Result<SharedGroup> {
    let path = group_path(id)?;
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("Sync group not found: {}", id))?;
    let group: SharedGroup = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse sync group: {}", path.display()))?;
    Ok(group)
}

fn save_group(group: &SharedGroup) -> Result<()> {
    let path = group_path(&group.id)?;
    let content = serde_json::to_string_pretty(group)? + "\n";
    crate::storage::atomic_write::write_string_atomic(&path, &content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

pub fn create_group(params: &ShareCreateParams) -> Result<SharedGroup> {
    let targets = normalized_distinct_targets(&params.provider, &params.targets);
    if targets.is_empty() {
        anyhow::bail!("At least one target provider is required");
    }

    let source_session =
        crate::core::get_canonical_session(&params.provider, &params.session_id)
            .with_context(|| format!("Failed to load source session {}", params.session_id))?;
    let now = Utc::now().timestamp_millis();
    let group_id = uuid::Uuid::new_v4().to_string();

    let title = params
        .title
        .clone()
        .or_else(|| source_session.session.primary_title().map(str::to_string))
        .unwrap_or_else(|| "Session sync".to_string());

    let mut holdings = Vec::new();

    // Source holding
    let source_holding_id = uuid::Uuid::new_v4().to_string();
    holdings.push(Holding {
        id: source_holding_id,
        provider: params.provider.clone(),
        session_id: params.session_id.clone(),
        target_dir: source_session.session.context.workspace_dir.clone(),
        created_at: now,
        last_active_at: source_session
            .session
            .context
            .last_active_at
            .map(|dt| dt.timestamp_millis()),
        last_sync_at: Some(now),
        last_sync_from: Some(params.provider.clone()),
        last_error: None,
    });

    let mut created_targets = Vec::new();
    let export_result: Result<()> = (|| {
        // Target holdings
        for target in &targets {
            let provider = providers::find_provider(target)
                .with_context(|| format!("Unknown target provider: {}", target))?;
            if !provider.capabilities().export {
                anyhow::bail!("Provider does not support writing sessions: {}", target);
            }
            let target_dir = resolve_target_dir(target, params.to_dir.as_deref())?;
            let session =
                prepare_session_for_export(&source_session.session, &params.provider, target)?;
            let exported = provider.export_session(&session, &target_dir)?;
            created_targets.push((target.clone(), exported.session_id.clone()));
            holdings.push(Holding {
                id: uuid::Uuid::new_v4().to_string(),
                provider: target.clone(),
                session_id: exported.session_id,
                target_dir: Some(target_dir.to_string_lossy().to_string()),
                created_at: now,
                last_active_at: None,
                last_sync_at: Some(now),
                last_sync_from: Some(params.provider.clone()),
                last_error: None,
            });
        }
        Ok(())
    })();
    if let Err(error) = export_result {
        cleanup_created_targets(&created_targets);
        return Err(error);
    }

    let group = SharedGroup {
        id: group_id,
        title,
        source_provider: Some(params.provider.clone()),
        created_at: now,
        updated_at: now,
        holdings,
    };

    save_group(&group)?;
    Ok(group)
}

fn normalized_distinct_targets(source_provider: &str, targets: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for target in targets {
        let target = target.trim();
        if target.is_empty() || target == source_provider || !seen.insert(target.to_string()) {
            continue;
        }
        normalized.push(target.to_string());
    }
    normalized
}

fn cleanup_created_targets(created_targets: &[(String, String)]) {
    for (provider_id, session_id) in created_targets.iter().rev() {
        let Some(provider) = providers::find_provider(provider_id) else {
            continue;
        };
        if !provider.capabilities().delete {
            continue;
        }
        if let Err(error) = crate::core::delete_session(provider_id, session_id) {
            eprintln!(
                "Warning: failed to clean up created sync target {}:{}: {}",
                provider_id, session_id, error
            );
        }
    }
}

pub fn add_holding(params: &AddHoldingParams) -> Result<Holding> {
    let mut group = load_group(&params.group_id)?;
    let provider = providers::find_provider(&params.provider)
        .with_context(|| format!("Unknown provider: {}", params.provider))?;
    let target_dir = resolve_target_dir(&params.provider, params.to_dir.as_deref())?;
    let now = Utc::now().timestamp_millis();

    let (session_id, target_dir_str) = if let Some(session_id) = &params.session_id {
        if group_has_holding(&group, &params.provider, session_id) {
            anyhow::bail!(
                "Session is already bound to this sync group: {}:{}",
                params.provider,
                session_id
            );
        }
        (
            session_id.clone(),
            Some(target_dir.to_string_lossy().to_string()),
        )
    } else {
        if !provider.capabilities().export {
            anyhow::bail!(
                "Provider does not support writing sessions: {}",
                params.provider
            );
        }
        let (session, source_provider) = build_canonical_session(&group)?;
        let session = prepare_session_for_export(&session, &source_provider, &params.provider)?;
        let exported = provider.export_session(&session, &target_dir)?;
        (
            exported.session_id,
            Some(target_dir.to_string_lossy().to_string()),
        )
    };

    let holding = Holding {
        id: uuid::Uuid::new_v4().to_string(),
        provider: params.provider.clone(),
        session_id,
        target_dir: target_dir_str,
        created_at: now,
        last_active_at: None,
        last_sync_at: Some(now),
        last_sync_from: group.source_provider.clone(),
        last_error: None,
    };

    group.holdings.push(holding.clone());
    group.updated_at = now;
    save_group(&group)?;
    Ok(holding)
}

fn group_has_holding(group: &SharedGroup, provider: &str, session_id: &str) -> bool {
    group
        .holdings
        .iter()
        .any(|holding| holding.provider == provider && holding.session_id == session_id)
}

pub fn remove_holding(group_id: &str, holding_id: &str) -> Result<()> {
    let mut group = load_group(group_id)?;
    let original_len = group.holdings.len();
    group.holdings.retain(|h| h.id != holding_id);
    if group.holdings.len() == original_len {
        anyhow::bail!("Holding not found: {}", holding_id);
    }
    group.updated_at = Utc::now().timestamp_millis();
    save_group(&group)?;
    Ok(())
}

pub fn delete_group(group_id: &str, delete_provider_sessions: bool) -> Result<()> {
    if delete_provider_sessions {
        if let Ok(group) = load_group(group_id) {
            for holding in &group.holdings {
                let _ = crate::core::delete_session(&holding.provider, &holding.session_id);
            }
        }
    }

    let path = group_path(group_id)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn rename_group(group_id: &str, title: &str) -> Result<()> {
    let mut group = load_group(group_id)?;
    group.title = title.to_string();
    group.updated_at = Utc::now().timestamp_millis();
    save_group(&group)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

pub fn push_sync(group_id: &str, source_holding_id: &str) -> Result<SyncReport> {
    let mut group = load_group(group_id)?;
    let source = group
        .holdings
        .iter()
        .find(|h| h.id == source_holding_id)
        .with_context(|| format!("Source holding not found: {}", source_holding_id))?
        .clone();

    let session = crate::core::get_canonical_session(&source.provider, &source.session_id)
        .with_context(|| format!("Failed to load source session from {}", source.provider))?;

    let mut report = SyncReport {
        source_provider: source.provider.clone(),
        source_holding_id: source_holding_id.to_string(),
        success: Vec::new(),
        errors: Vec::new(),
    };

    let now = Utc::now().timestamp_millis();

    for holding in &mut group.holdings {
        if holding.id == source_holding_id {
            holding.last_sync_at = Some(now);
            holding.last_sync_from = Some(source.provider.clone());
            holding.last_error = None;
            continue;
        }

        let provider = match providers::find_provider(&holding.provider) {
            Some(p) => p,
            None => {
                let msg = format!("Unknown provider: {}", holding.provider);
                holding.last_error = Some(msg.clone());
                report.errors.push(msg);
                continue;
            }
        };

        let target_dir = holding
            .target_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let old_session_id = holding.session_id.clone();
        let target_session =
            prepare_session_for_export(&session.session, &source.provider, &holding.provider)?;
        match provider.export_session(&target_session, &target_dir) {
            Ok(exported) => {
                holding.session_id = exported.session_id;
                holding.last_sync_at = Some(now);
                holding.last_sync_from = Some(source.provider.clone());
                holding.last_error = None;
                report.success.push(holding.provider.clone());

                if provider.capabilities().delete && old_session_id != holding.session_id {
                    if let Err(e) = crate::core::delete_session(&holding.provider, &old_session_id)
                    {
                        eprintln!(
                            "Warning: failed to delete old session {} after sync: {}",
                            old_session_id, e
                        );
                    }
                }
            }
            Err(e) => {
                let msg = format!("Failed to sync to {}: {:#}", holding.provider, e);
                holding.last_error = Some(msg.clone());
                report.errors.push(msg);
            }
        }
    }

    group.updated_at = now;
    save_group(&group)?;
    Ok(report)
}

pub fn sync_to_latest(group_id: &str) -> Result<SyncReport> {
    let mut group = load_group(group_id)?;
    refresh_active_times(&mut group)?;

    let source_id = group
        .holdings
        .iter()
        .filter(|h| h.last_active_at.is_some())
        .max_by_key(|h| h.last_active_at.unwrap_or(0))
        .map(|h| h.id.clone())
        .with_context(|| "No holding with active time found")?;

    // Re-load because push_sync also loads the group
    push_sync(group_id, &source_id)
}

// ---------------------------------------------------------------------------
// Status helpers
// ---------------------------------------------------------------------------

pub fn refresh_active_times(group: &mut SharedGroup) -> Result<()> {
    for holding in &mut group.holdings {
        if let Some(provider) = providers::find_provider(&holding.provider) {
            if provider.capabilities().scan {
                let cache = crate::cache::global_cache();
                if let Ok(sessions) =
                    cache.get_or_refresh(&holding.provider, || provider.scan_sessions())
                {
                    if let Some(meta) = sessions
                        .into_iter()
                        .find(|s| s.session_id == holding.session_id)
                    {
                        holding.last_active_at = meta.last_active_at;
                    }
                }
            }
        }
    }
    Ok(())
}

fn build_canonical_session(group: &SharedGroup) -> Result<(CanonicalSession, String)> {
    // For now, build from the first holding that we can load.
    // In practice, add_holding is usually called with a specific session_id
    // or when creating a new projection from the group.
    if let Some(first) = group.holdings.first() {
        crate::core::get_canonical_session(&first.provider, &first.session_id)
            .map(|imported| (imported.session, first.provider.clone()))
    } else {
        anyhow::bail!("Group has no holdings to build canonical session from")
    }
}

fn prepare_session_for_export(
    session: &CanonicalSession,
    source_provider: &str,
    target_provider: &str,
) -> Result<CanonicalSession> {
    crate::core::session_management::prepare_session_for_export(
        session,
        source_provider,
        target_provider,
    )
    .map(|(session, _)| session)
}

fn resolve_target_dir(provider_id: &str, input: Option<&str>) -> Result<PathBuf> {
    crate::core::session_management::resolve_existing_target_dir(provider_id, input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{
        CanonicalSchema, EventBlock, EventLinks, EventMetadata, EventRole, EventSource,
        MappingDisposition, ProviderSessionRef, SessionContext, SessionEvent, SessionEventKind,
        SessionIdentity, SessionProvenance,
    };
    use chrono::Utc;
    use std::collections::BTreeMap;

    #[test]
    fn normalized_targets_drop_source_empty_and_duplicates() {
        let targets = vec![
            "codex".to_string(),
            "".to_string(),
            "claude".to_string(),
            "claude".to_string(),
            " opencode ".to_string(),
        ];

        assert_eq!(
            normalized_distinct_targets("codex", &targets),
            vec!["claude".to_string(), "opencode".to_string()]
        );
    }

    #[test]
    fn group_has_holding_matches_provider_and_session() {
        let group = SharedGroup {
            id: "group-1".to_string(),
            title: "group".to_string(),
            source_provider: Some("codex".to_string()),
            created_at: 1,
            updated_at: 1,
            holdings: vec![Holding {
                id: "holding-1".to_string(),
                provider: "codex".to_string(),
                session_id: "session-1".to_string(),
                target_dir: None,
                created_at: 1,
                last_active_at: None,
                last_sync_at: None,
                last_sync_from: None,
                last_error: None,
            }],
        };

        assert!(group_has_holding(&group, "codex", "session-1"));
        assert!(!group_has_holding(&group, "claude", "session-1"));
        assert!(!group_has_holding(&group, "codex", "session-2"));
    }

    #[test]
    fn shared_export_preparation_preserves_source_compression() {
        let session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "s1".to_string(),
                source_title: None,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("test".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: "opencode".to_string(),
                    session_id: "s1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext::default(),
            events: vec![
                text_event("old", EventRole::User, "old expanded context", false),
                compaction_event("marker"),
                text_event("summary", EventRole::Assistant, "compressed summary", true),
                text_event("tail", EventRole::User, "latest request", false),
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };

        let prepared = compression::prepare_for_export(
            &session,
            &compression::CompressionPolicy::preserve("opencode", "codex"),
        )
        .0;

        assert_eq!(prepared.events.len(), 2);
        assert!(matches!(
            prepared.events[0].blocks.first(),
            Some(EventBlock::Compressed { summary, .. }) if summary == "compressed summary"
        ));
        assert_eq!(prepared.events[1].id, "tail");
    }

    fn text_event(id: &str, role: EventRole, text: &str, summary: bool) -> SessionEvent {
        let mut provider_ext = BTreeMap::new();
        provider_ext.insert(
            "opencode_message".to_string(),
            serde_json::json!({ "summary": summary }),
        );

        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Message,
            role,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: text.to_string(),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "opencode".to_string(),
                    original_id: Some(id.to_string()),
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext,
            },
        }
    }

    fn compaction_event(id: &str) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Unknown,
            role: EventRole::User,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::ProviderPayload {
                kind: "compaction".to_string(),
                payload: serde_json::json!({ "type": "compaction" }),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "opencode".to_string(),
                    original_id: Some(id.to_string()),
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
}
