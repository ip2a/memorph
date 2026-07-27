use anyhow::{Context as _, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::canonical::{Fidelity, Session};
#[cfg(test)]
use crate::core::compression;
use crate::provider::ProviderWriteRisk;
use crate::providers;
use crate::storage::{
    activity_store::{
        ActivityActor, ActivityCompletion, ActivityOperationKind, ActivityStatus, ActivityStore,
        NewActivity,
    },
    local_store, sync_store,
};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncGroup {
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
pub struct SyncCreateParams {
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
    pub target_assessments: Vec<SyncTargetAssessment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncTargetAssessment {
    pub provider: String,
    pub fidelity: Fidelity,
    pub write_risk: ProviderWriteRisk,
}

pub fn list_groups() -> Result<Vec<SyncGroup>> {
    let conn = local_store::open_database()?;
    sync_store::list_groups(&conn)
}

pub fn load_group(id: &str) -> Result<SyncGroup> {
    let conn = local_store::open_database()?;
    sync_store::load_group(&conn, id)
}

fn save_group(group: &SyncGroup) -> Result<()> {
    let mut conn = local_store::open_database()?;
    sync_store::save_group(&mut conn, group)
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

pub fn create_group(params: &SyncCreateParams) -> Result<SyncGroup> {
    let targets = normalized_distinct_targets(&params.provider, &params.targets);
    if targets.is_empty() {
        anyhow::bail!("At least one target provider is required");
    }

    let source_session =
        crate::core::sessions::get_canonical_session(&params.provider, &params.session_id)
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

    let group = SyncGroup {
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
        if let Err(error) = crate::core::session_mutation::delete_session(
            provider_id,
            session_id,
            ActivityActor::Sync,
        ) {
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

fn group_has_holding(group: &SyncGroup, provider: &str, session_id: &str) -> bool {
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
                let _ = crate::core::session_mutation::delete_session(
                    &holding.provider,
                    &holding.session_id,
                    ActivityActor::Sync,
                );
            }
        }
    }

    let conn = local_store::open_database()?;
    sync_store::delete_group(&conn, group_id)
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

pub fn push_sync(
    group_id: &str,
    source_holding_id: &str,
    actor: ActivityActor,
) -> Result<SyncReport> {
    let activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "group_id": group_id,
        "source_holding_id": source_holding_id,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Sync,
        actor,
        summary: "Synchronizing session group".to_string(),
        details: input_details.clone(),
    })?;
    let mut source_identity: Option<(String, String, Option<String>)> = None;
    let result = (|| {
        let mut group = load_group(group_id)?;
        let source = group
            .holdings
            .iter()
            .find(|h| h.id == source_holding_id)
            .with_context(|| format!("Source holding not found: {}", source_holding_id))?
            .clone();
        source_identity = Some((
            source.provider.clone(),
            source.session_id.clone(),
            source.target_dir.clone(),
        ));

        let session =
            crate::core::sessions::get_canonical_session(&source.provider, &source.session_id)
                .with_context(|| {
                    format!("Failed to load source session from {}", source.provider)
                })?;
        if let Some((_, _, workspace_dir)) = source_identity.as_mut() {
            if workspace_dir.is_none() {
                *workspace_dir = session.session.context.workspace_dir.clone();
            }
        }

        let mut report = SyncReport {
            source_provider: source.provider.clone(),
            source_holding_id: source_holding_id.to_string(),
            success: Vec::new(),
            errors: Vec::new(),
            target_assessments: Vec::new(),
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
                Some(provider) => provider,
                None => {
                    let message = format!("Unknown provider: {}", holding.provider);
                    holding.last_error = Some(message.clone());
                    report.errors.push(message);
                    continue;
                }
            };
            let target_dir = holding
                .target_dir
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let old_session_id = holding.session_id.clone();
            let capabilities = provider.capabilities();
            let target_session =
                prepare_session_for_export(&session.session, &source.provider, &holding.provider)?;
            match provider.export_session(&target_session, &target_dir) {
                Ok(exported) => {
                    let fidelity = exported.report.overall;
                    holding.session_id = exported.session_id;
                    holding.last_sync_at = Some(now);
                    holding.last_sync_from = Some(source.provider.clone());
                    holding.last_error = None;
                    report.success.push(holding.provider.clone());
                    report.target_assessments.push(SyncTargetAssessment {
                        provider: holding.provider.clone(),
                        fidelity,
                        write_risk: capabilities.write_risk,
                    });

                    if capabilities.delete && old_session_id != holding.session_id {
                        if let Err(error) = crate::core::session_mutation::delete_session(
                            &holding.provider,
                            &old_session_id,
                            ActivityActor::Sync,
                        ) {
                            let message = format!(
                                "Synchronized to {} but failed to delete old session {}: {error:#}",
                                holding.provider, old_session_id
                            );
                            holding.last_error = Some(message.clone());
                            report.errors.push(message);
                        }
                    }
                }
                Err(error) => {
                    let message = format!("Failed to sync to {}: {error:#}", holding.provider);
                    holding.last_error = Some(message.clone());
                    report.errors.push(message);
                }
            }
        }

        group.updated_at = now;
        save_group(&group)?;
        let conn = local_store::open_database()?;
        sync_store::record_sync_run(
            &conn,
            group_id,
            source_holding_id,
            now,
            Utc::now().timestamp_millis(),
            &report,
        )?;
        Ok(report)
    })();

    let (provider_id, provider_session_id, workspace_dir) = source_identity
        .clone()
        .map(|(provider_id, provider_session_id, workspace_dir)| {
            (Some(provider_id), Some(provider_session_id), workspace_dir)
        })
        .unwrap_or((None, None, None));
    match result {
        Ok(report) => {
            let status = if report.errors.is_empty() {
                ActivityStatus::Success
            } else if report.success.is_empty() {
                ActivityStatus::Failed
            } else {
                ActivityStatus::Partial
            };
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status,
                    provider_id,
                    provider_session_id,
                    workspace_dir,
                    summary: "Synchronized session group".to_string(),
                    details: serde_json::json!({
                        "group_id": group_id,
                        "source_holding_id": source_holding_id,
                        "source_provider": report.source_provider,
                        "success": report.success,
                        "errors": report.errors,
                        "target_assessments": report.target_assessments,
                    }),
                    error: (!report.errors.is_empty()).then(|| report.errors.join("\n")),
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status: ActivityStatus::Failed,
                    provider_id,
                    provider_session_id,
                    workspace_dir,
                    summary: "Failed to synchronize session group".to_string(),
                    details: input_details,
                    error: Some(message),
                },
            )?;
            Err(error)
        }
    }
}

pub fn sync_to_latest(group_id: &str, actor: ActivityActor) -> Result<SyncReport> {
    let source_result = (|| {
        let mut group = load_group(group_id)?;
        refresh_active_times(&mut group)?;
        group
            .holdings
            .iter()
            .filter(|holding| holding.last_active_at.is_some())
            .max_by_key(|holding| holding.last_active_at.unwrap_or(0))
            .map(|holding| holding.id.clone())
            .with_context(|| "No holding with active time found")
    })();
    match source_result {
        Ok(source_id) => push_sync(group_id, &source_id, actor),
        Err(error) => {
            let conn = local_store::open_database()?;
            let activity_id = ActivityStore::new(&conn).start(NewActivity {
                provider_id: None,
                provider_session_id: None,
                workspace_dir: None,
                operation_kind: ActivityOperationKind::Sync,
                actor,
                summary: "Selecting latest session sync source".to_string(),
                details: serde_json::json!({"group_id": group_id}),
            })?;
            let message = format!("{error:#}");
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to select latest session sync source",
                    serde_json::json!({"group_id": group_id}),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

// ---------------------------------------------------------------------------
// Status helpers
// ---------------------------------------------------------------------------

pub fn refresh_active_times(group: &mut SyncGroup) -> Result<()> {
    let conn = local_store::open_database()?;
    let snapshots =
        crate::storage::snapshot_store::SnapshotStore::new(&conn).list_session_snapshots()?;
    apply_projected_active_times(group, &snapshots);
    Ok(())
}

fn apply_projected_active_times(
    group: &mut SyncGroup,
    snapshots: &[crate::storage::snapshot_store::ProjectedSessionSnapshotRow],
) {
    for holding in &mut group.holdings {
        if let Some(snapshot) = snapshots.iter().find(|snapshot| {
            snapshot.provider_id == holding.provider
                && (snapshot.canonical_session_id == holding.session_id
                    || snapshot.provider_session_id.as_deref() == Some(holding.session_id.as_str()))
        }) {
            holding.last_active_at = snapshot.last_active_at_ms;
        }
    }
}

fn build_canonical_session(group: &SyncGroup) -> Result<(Session, String)> {
    // For now, build from the first holding that we can load.
    // In practice, add_holding is usually called with a specific session_id
    // or when creating a new projection from the group.
    if let Some(first) = group.holdings.first() {
        crate::core::sessions::get_canonical_session(&first.provider, &first.session_id)
            .map(|imported| (imported.session, first.provider.clone()))
    } else {
        anyhow::bail!("Group has no holdings to build canonical session from")
    }
}

fn prepare_session_for_export(
    session: &Session,
    source_provider: &str,
    target_provider: &str,
) -> Result<Session> {
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
        Block, Context, Event, EventKind, Fidelity, Identity, Links, Metadata, Provenance,
        ProviderRef, Role, Schema, Source,
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
        let group = SyncGroup {
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
    fn refresh_active_times_uses_projected_sessions_and_preserves_missing_values() {
        let mut group = SyncGroup {
            id: "group-1".to_string(),
            title: "group".to_string(),
            source_provider: Some("codex".to_string()),
            created_at: 1,
            updated_at: 1,
            holdings: vec![
                Holding {
                    id: "holding-native".to_string(),
                    provider: "codex".to_string(),
                    session_id: "native-1".to_string(),
                    target_dir: None,
                    created_at: 1,
                    last_active_at: None,
                    last_sync_at: None,
                    last_sync_from: None,
                    last_error: None,
                },
                Holding {
                    id: "holding-canonical".to_string(),
                    provider: "claude".to_string(),
                    session_id: "claude:canonical-2".to_string(),
                    target_dir: None,
                    created_at: 1,
                    last_active_at: None,
                    last_sync_at: None,
                    last_sync_from: None,
                    last_error: None,
                },
                Holding {
                    id: "holding-missing".to_string(),
                    provider: "opencode".to_string(),
                    session_id: "missing".to_string(),
                    target_dir: None,
                    created_at: 1,
                    last_active_at: Some(7),
                    last_sync_at: None,
                    last_sync_from: None,
                    last_error: None,
                },
            ],
        };
        let snapshots = vec![
            projected_snapshot("codex:canonical-1", "codex", "native-1", 30),
            projected_snapshot("claude:canonical-2", "claude", "native-2", 40),
        ];

        apply_projected_active_times(&mut group, &snapshots);

        assert_eq!(group.holdings[0].last_active_at, Some(30));
        assert_eq!(group.holdings[1].last_active_at, Some(40));
        assert_eq!(group.holdings[2].last_active_at, Some(7));
    }

    #[test]
    fn sync_export_preparation_preserves_source_compression() {
        let session = Session {
            schema: Schema::default(),
            identity: Identity {
                canonical_id: "s1".to_string(),
                source_title: None,
            },
            provenance: Provenance {
                imported_at: Utc::now(),
                imported_by: Some("test".to_string()),
                primary_source: ProviderRef {
                    provider_id: "opencode".to_string(),
                    session_id: "s1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: Context::default(),
            events: vec![
                text_event("old", Role::User, "old expanded context", false),
                compaction_event("marker"),
                text_event("summary", Role::Assistant, "compressed summary", true),
                text_event("tail", Role::User, "latest request", false),
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
            Some(Block::Compressed { summary, .. }) if summary == "compressed summary"
        ));
        assert_eq!(prepared.events[1].id, "tail");
    }

    fn text_event(id: &str, role: Role, text: &str, summary: bool) -> Event {
        let mut provider_ext = BTreeMap::new();
        provider_ext.insert(
            "opencode_message".to_string(),
            serde_json::json!({ "summary": summary }),
        );

        Event {
            id: id.to_string(),
            kind: EventKind::Message,
            role,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::Text {
                text: text.to_string(),
            }],
            metadata: Metadata {
                source: Source {
                    provider_id: "opencode".to_string(),
                    original_id: Some(id.to_string()),
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: Fidelity::Preserved,
                provider_ext,
            },
        }
    }

    fn compaction_event(id: &str) -> Event {
        Event {
            id: id.to_string(),
            kind: EventKind::Unknown,
            role: Role::User,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::ProviderPayload {
                kind: "compaction".to_string(),
                payload: serde_json::json!({ "type": "compaction" }),
            }],
            metadata: Metadata {
                source: Source {
                    provider_id: "opencode".to_string(),
                    original_id: Some(id.to_string()),
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: Fidelity::Preserved,
                provider_ext: BTreeMap::new(),
            },
        }
    }

    fn projected_snapshot(
        canonical_session_id: &str,
        provider_id: &str,
        provider_session_id: &str,
        last_active_at_ms: i64,
    ) -> crate::storage::snapshot_store::ProjectedSessionSnapshotRow {
        crate::storage::snapshot_store::ProjectedSessionSnapshotRow {
            canonical_session_id: canonical_session_id.to_string(),
            provider_id: provider_id.to_string(),
            provider_session_id: Some(provider_session_id.to_string()),
            title: None,
            display_title: None,
            workspace_dir: None,
            last_active_at_ms: Some(last_active_at_ms),
            source_path: Some("/missing/provider/source".to_string()),
            message_count: Some(0),
            event_count: 0,
            turn_count: 0,
            size_bytes: None,
            hidden: false,
            pinned: false,
            preferred_targets: Vec::new(),
            stale: false,
        }
    }
}
