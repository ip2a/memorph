use super::*;

pub fn resolve_providers(filter: &[String]) -> Vec<String> {
    if filter.is_empty() {
        providers::all_provider_ids()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        filter.to_vec()
    }
}

pub fn list_sessions(params: &SessionListParams) -> Result<Vec<SessionGroup>> {
    list_projected_session_snapshots(params)
}

pub fn refresh_projected_session_staleness(
    actor: ActivityActor,
) -> Result<SnapshotStaleScanReport> {
    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Scan,
        actor,
        summary: "Scanning projected session source fingerprints".to_string(),
        details: serde_json::json!({"scan_kind": "snapshot_staleness"}),
    })?;
    let result = (|| {
        let conn = local_store::open_database()?;
        crate::storage::snapshot_store::SnapshotStore::new(&conn)
            .refresh_session_snapshot_staleness(|provider_id, source_path| {
                let provider = providers::find_provider(provider_id)
                    .with_context(|| format!("Unknown provider: {provider_id}"))?;
                Ok(provider
                    .session_source_fingerprint(source_path)?
                    .map(|fingerprint| fingerprint.value))
            })
    })();
    match result {
        Ok(report) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Scanned projected session source fingerprints",
                    serde_json::json!({
                        "checked_sources": report.checked_sources,
                        "fresh_snapshots": report.fresh_snapshots,
                        "stale_snapshots": report.stale_snapshots,
                        "missing_sources": report.missing_sources,
                        "unknown_sources": report.unknown_sources,
                    }),
                ),
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to scan projected session source fingerprints",
                    serde_json::json!({"scan_kind": "snapshot_staleness"}),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionReprojectionReport {
    pub candidate_snapshots: usize,
    pub reprojected_snapshots: usize,
    pub missing_sources: usize,
    pub unsupported_providers: usize,
    pub failed_snapshots: usize,
    pub failures: Vec<SessionReprojectionFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionReprojectionFailure {
    pub provider_id: String,
    pub session_id: String,
    pub source_path: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionProjectionBootstrapReport {
    pub scanned_providers: usize,
    pub failed_providers: usize,
    pub discovered_sessions: usize,
    pub projected_sessions: usize,
    pub unchanged_sessions: usize,
    pub missing_sources: usize,
    pub unsupported_providers: usize,
    pub failed_sessions: usize,
    pub failures: Vec<SessionProjectionBootstrapFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionProjectionBootstrapFailure {
    pub provider_id: String,
    pub session_id: Option<String>,
    pub source_path: Option<String>,
    pub reason: String,
}

pub fn bootstrap_session_projections(
    provider_filter: Option<&str>,
    actor: ActivityActor,
) -> Result<SessionProjectionBootstrapReport> {
    let provider_filter = provider_filter.map(providers::canonical_provider_id);
    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: provider_filter.clone(),
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Scan,
        actor,
        summary: "Discovering and projecting provider sessions".to_string(),
        details: serde_json::json!({
            "scan_kind": "projection_bootstrap",
            "provider_filter": provider_filter,
        }),
    })?;
    let result = (|| {
        let mut conn = local_store::open_database()?;
        bootstrap_session_projections_in_connection(&mut conn, provider_filter.as_deref())
    })();
    match result {
        Ok(report) => {
            let has_failures = report.failed_providers > 0
                || report.failed_sessions > 0
                || report.missing_sources > 0
                || report.unsupported_providers > 0;
            let status = if !has_failures {
                ActivityStatus::Success
            } else if report.projected_sessions == 0 && report.unchanged_sessions == 0 {
                ActivityStatus::Failed
            } else {
                ActivityStatus::Partial
            };
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status,
                    provider_id: provider_filter.clone(),
                    provider_session_id: None,
                    workspace_dir: None,
                    summary: "Discovered and projected provider sessions".to_string(),
                    details: serde_json::to_value(&report)?,
                    error: (!report.failures.is_empty()).then(|| {
                        report
                            .failures
                            .iter()
                            .map(|failure| failure.reason.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    }),
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to discover and project provider sessions",
                    serde_json::json!({
                        "scan_kind": "projection_bootstrap",
                        "provider_filter": provider_filter,
                    }),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

pub(super) fn bootstrap_session_projections_in_connection(
    conn: &mut rusqlite::Connection,
    provider_filter: Option<&str>,
) -> Result<SessionProjectionBootstrapReport> {
    let provider_ids = provider_filter
        .map(|provider_id| vec![providers::canonical_provider_id(provider_id)])
        .unwrap_or_else(|| {
            PROJECTED_SESSION_PROVIDER_IDS
                .iter()
                .map(|provider_id| (*provider_id).to_string())
                .collect()
        });
    let mut report = SessionProjectionBootstrapReport::default();

    for provider_id in provider_ids {
        if !provider_supports_session_projection(&provider_id) {
            report.unsupported_providers += 1;
            report.failures.push(SessionProjectionBootstrapFailure {
                provider_id: provider_id.clone(),
                session_id: None,
                source_path: None,
                reason: format!(
                    "provider does not support projected store bootstrap yet: {provider_id}"
                ),
            });
            continue;
        }
        let Some(provider) = providers::find_provider(&provider_id) else {
            report.unsupported_providers += 1;
            report.failures.push(SessionProjectionBootstrapFailure {
                provider_id: provider_id.clone(),
                session_id: None,
                source_path: None,
                reason: format!("provider is not registered: {provider_id}"),
            });
            continue;
        };
        let sessions = match provider.scan_sessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                report.failed_providers += 1;
                report.failures.push(SessionProjectionBootstrapFailure {
                    provider_id: provider_id.clone(),
                    session_id: None,
                    source_path: None,
                    reason: format!("failed to scan provider sessions: {error:#}"),
                });
                continue;
            }
        };
        report.scanned_providers += 1;
        report.discovered_sessions += sessions.len();

        for session in sessions {
            bootstrap_provider_session(conn, &provider_id, &session, &mut report);
        }
    }

    Ok(report)
}

pub(super) fn bootstrap_provider_session(
    conn: &mut rusqlite::Connection,
    provider_id: &str,
    session: &ProviderSessionSummary,
    report: &mut SessionProjectionBootstrapReport,
) {
    let Some(source_path) = session
        .source_path
        .as_deref()
        .filter(|source_path| !source_path.is_empty())
    else {
        report.missing_sources += 1;
        report.failures.push(bootstrap_failure(
            provider_id,
            session,
            "provider session has no source path".to_string(),
        ));
        return;
    };
    let Some(provider) = providers::find_provider(provider_id) else {
        report.failed_sessions += 1;
        report.failures.push(bootstrap_failure(
            provider_id,
            session,
            format!("provider is not registered: {provider_id}"),
        ));
        return;
    };
    let fingerprint = match provider.session_source_fingerprint(source_path) {
        Ok(Some(fingerprint)) => fingerprint,
        Ok(None) => {
            report.missing_sources += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("session source not found: {source_path}"),
            ));
            return;
        }
        Err(error) => {
            report.failed_sessions += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("failed to fingerprint provider session source: {error:#}"),
            ));
            return;
        }
    };

    let freshness = crate::storage::snapshot_store::SnapshotStore::new(conn)
        .session_source_is_fresh(
            provider_id,
            &session.session_id,
            source_path,
            &fingerprint.value,
        );
    match freshness {
        Ok(true) => {
            report.unchanged_sessions += 1;
            return;
        }
        Ok(false) => {}
        Err(error) => {
            report.failed_sessions += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("failed to inspect projected source freshness: {error:#}"),
            ));
            return;
        }
    }

    match crate::storage::session_index_store::SessionIndexStore::new(conn).write_session_summary(
        provider_id,
        session,
        provider.capabilities(),
        &fingerprint,
    ) {
        Ok(_) => report.projected_sessions += 1,
        Err(error) => {
            report.failed_sessions += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("failed to project provider session: {error:#}"),
            ));
        }
    }
}

pub(super) fn bootstrap_failure(
    provider_id: &str,
    session: &ProviderSessionSummary,
    reason: String,
) -> SessionProjectionBootstrapFailure {
    SessionProjectionBootstrapFailure {
        provider_id: provider_id.to_string(),
        session_id: Some(session.session_id.clone()),
        source_path: session.source_path.clone(),
        reason,
    }
}

pub fn reproject_stale_sessions(
    provider_filter: Option<&str>,
    actor: ActivityActor,
) -> Result<SessionReprojectionReport> {
    let provider_filter = provider_filter.map(providers::canonical_provider_id);
    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: provider_filter.clone(),
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Scan,
        actor,
        summary: "Reprojecting stale session snapshots".to_string(),
        details: serde_json::json!({
            "scan_kind": "stale_reprojection",
            "provider_filter": provider_filter,
        }),
    })?;
    let result = (|| {
        let mut conn = local_store::open_database()?;
        let sources = crate::storage::snapshot_store::SnapshotStore::new(&conn)
            .list_stale_snapshot_sources(provider_filter.as_deref())?;
        reproject_stale_snapshot_sources(&mut conn, sources)
    })();
    match result {
        Ok(report) => {
            let status = if report.failed_snapshots == 0
                && report.missing_sources == 0
                && report.unsupported_providers == 0
            {
                ActivityStatus::Success
            } else if report.reprojected_snapshots == 0 {
                ActivityStatus::Failed
            } else {
                ActivityStatus::Partial
            };
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status,
                    provider_id: provider_filter.clone(),
                    provider_session_id: None,
                    workspace_dir: None,
                    summary: "Reprojected stale session snapshots".to_string(),
                    details: serde_json::to_value(&report)?,
                    error: (!report.failures.is_empty()).then(|| {
                        report
                            .failures
                            .iter()
                            .map(|failure| failure.reason.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    }),
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to reproject stale session snapshots",
                    serde_json::json!({
                        "scan_kind": "stale_reprojection",
                        "provider_filter": provider_filter,
                    }),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

pub(super) fn reproject_stale_snapshot_sources(
    conn: &mut rusqlite::Connection,
    sources: Vec<StaleSnapshotSourceRow>,
) -> Result<SessionReprojectionReport> {
    let mut report = SessionReprojectionReport {
        candidate_snapshots: sources.len(),
        ..SessionReprojectionReport::default()
    };
    for source in sources {
        if !provider_supports_session_projection(&source.provider_id) {
            report.unsupported_providers += 1;
            report.failures.push(reprojection_failure(
                &source,
                format!(
                    "provider does not support projected store reprojection yet: {}",
                    source.provider_id
                ),
            ));
            continue;
        }
        let Some(provider_session_id) = source
            .provider_session_id
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            report.failed_snapshots += 1;
            report.failures.push(reprojection_failure(
                &source,
                "indexed session has no provider session id".to_string(),
            ));
            continue;
        };
        let Some(provider) = providers::find_provider(&source.provider_id) else {
            report.unsupported_providers += 1;
            report.failures.push(reprojection_failure(
                &source,
                format!("provider is not registered: {}", source.provider_id),
            ));
            continue;
        };
        let summary = match provider.get_session_meta(provider_session_id) {
            Ok(Some(summary)) => summary,
            Ok(None) => {
                report.missing_sources += 1;
                report.failures.push(reprojection_failure(
                    &source,
                    "provider no longer reports the indexed session".to_string(),
                ));
                continue;
            }
            Err(error) => {
                report.failed_snapshots += 1;
                report.failures.push(reprojection_failure(
                    &source,
                    format!("failed to refresh provider session summary: {error:#}"),
                ));
                continue;
            }
        };
        let Some(summary_source_path) = summary
            .source_path
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            report.missing_sources += 1;
            report.failures.push(reprojection_failure(
                &source,
                "provider session summary has no source locator".to_string(),
            ));
            continue;
        };
        let fingerprint = match provider.session_source_fingerprint(summary_source_path) {
            Ok(Some(fingerprint)) => fingerprint,
            Ok(None) => {
                report.missing_sources += 1;
                report.failures.push(reprojection_failure(
                    &source,
                    format!("session source not found: {summary_source_path}"),
                ));
                continue;
            }
            Err(error) => {
                report.failed_snapshots += 1;
                report.failures.push(reprojection_failure(
                    &source,
                    format!("failed to fingerprint provider session source: {error:#}"),
                ));
                continue;
            }
        };
        match crate::storage::session_index_store::SessionIndexStore::new(conn)
            .write_session_summary(
                &source.provider_id,
                &summary,
                provider.capabilities(),
                &fingerprint,
            ) {
            Ok(_) => report.reprojected_snapshots += 1,
            Err(error) => {
                report.failed_snapshots += 1;
                report
                    .failures
                    .push(reprojection_failure(&source, format!("{error:#}")));
            }
        }
    }
    Ok(report)
}

pub(super) fn provider_supports_session_projection(provider_id: &str) -> bool {
    PROJECTED_SESSION_PROVIDER_IDS.contains(&provider_id)
}

pub(super) fn reprojection_failure(
    source: &StaleSnapshotSourceRow,
    reason: String,
) -> SessionReprojectionFailure {
    SessionReprojectionFailure {
        provider_id: source.provider_id.clone(),
        session_id: source
            .provider_session_id
            .clone()
            .unwrap_or_else(|| source.canonical_session_id.clone()),
        source_path: source.source_path.clone(),
        reason,
    }
}

pub(super) fn list_projected_session_snapshots(
    params: &SessionListParams,
) -> Result<Vec<SessionGroup>> {
    let provider_ids = resolve_providers(&params.providers);
    let workspace_scopes = if params.all {
        None
    } else {
        let scopes: Vec<(String, String)> = provider_ids
            .iter()
            .filter_map(|provider_id| {
                session_management::normalized_workspace_key(provider_id, params.cwd.as_deref())
                    .map(|workspace| (provider_id.clone(), workspace))
            })
            .collect();
        Some(scopes)
    };

    let conn = crate::storage::local_store::open_database()?;
    let snapshots = crate::storage::snapshot_store::SnapshotStore::new(&conn)
        .list_session_snapshots_filtered(
            Some(&provider_ids),
            workspace_scopes.as_deref(),
            params.include_message_counts,
        )?;
    let hook_runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
    let providers_with_snapshots: Vec<String> = provider_ids
        .iter()
        .filter(|provider_id| {
            snapshots
                .iter()
                .any(|snapshot| snapshot.provider_id == **provider_id)
        })
        .cloned()
        .collect();
    let last_event_at_cache =
        crate::hooks::store::last_event_observed_at_ms_for_providers(&providers_with_snapshots)
            .unwrap_or_default();
    let hook_statuses = providers_with_snapshots
        .iter()
        .map(|provider_id| {
            let hook_status = crate::hooks::operations::status_with_cached_last_event_at(
                provider_id,
                &last_event_at_cache,
            )
            .unwrap_or(crate::hooks::model::HookInstallStatus {
                provider: provider_id.clone(),
                status: crate::hooks::model::HookHealthStatus::InstalledBrokenConfig,
                config_path: None,
                installed_version: None,
                current_version: None,
                message: Some(
                    "Failed to inspect hook status while building session list.".to_string(),
                ),
                last_event_at: None,
            });
            (provider_id.clone(), hook_status)
        })
        .collect();
    Ok(projected_snapshot_groups(
        snapshots,
        params,
        &hook_runtime_snapshot,
        &hook_statuses,
    ))
}

pub(super) fn projected_snapshot_groups(
    snapshots: Vec<ProjectedSessionSnapshotRow>,
    params: &SessionListParams,
    hook_runtime_snapshot: &[crate::hooks::model::RuntimeSession],
    hook_statuses: &HashMap<String, crate::hooks::model::HookInstallStatus>,
) -> Vec<SessionGroup> {
    let provider_ids = resolve_providers(&params.providers);
    let requested_workspace = if params.all {
        None
    } else {
        params.cwd.as_deref()
    };
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(usize::MAX);

    provider_ids
        .into_iter()
        .filter_map(|provider_id| {
            let provider_name = providers::find_provider(&provider_id)
                .map(|provider| provider.name().to_string())
                .unwrap_or_else(|| provider_id.clone());
            let requested_workspace_key = requested_workspace.and_then(|workspace| {
                session_management::normalized_workspace_key(&provider_id, Some(workspace))
            });
            let hook_status = hook_statuses.get(&provider_id);
            let mut sessions: Vec<SessionItem> = snapshots
                .iter()
                .filter(|snapshot| snapshot.provider_id == provider_id)
                .filter(|snapshot| {
                    let Some(requested_workspace_key) = requested_workspace_key.as_deref() else {
                        return true;
                    };
                    let session_workspace_key = session_management::normalized_workspace_key(
                        &provider_id,
                        snapshot.workspace_dir.as_deref(),
                    );
                    session_workspace_key.as_deref() == Some(requested_workspace_key)
                })
                .map(|snapshot| {
                    let mut item = projected_snapshot_item(snapshot);
                    if let Some(hook_status) = hook_status {
                        let hook_augmentation =
                            crate::hooks::augmentation::augment_session_from_snapshot_with_status(
                                hook_runtime_snapshot,
                                hook_status.clone(),
                                &provider_id,
                                &item.session_id,
                                snapshot.workspace_dir.as_deref(),
                            );
                        item.hook_runtime_summary = hook_augmentation.runtime_summary;
                        item.hook_diagnosis = hook_augmentation.diagnosis;
                    }
                    item
                })
                .collect();

            sessions.retain(|item| session_matches_hook_filter(item, &params.hook_filter));
            sort_session_items(&mut sessions, &params.sort);
            let sessions: Vec<SessionItem> =
                sessions.into_iter().skip(offset).take(limit).collect();
            if sessions.is_empty() {
                None
            } else {
                Some(SessionGroup {
                    provider_id,
                    provider_name,
                    sessions,
                })
            }
        })
        .collect()
}

pub(super) fn projected_snapshot_item(snapshot: &ProjectedSessionSnapshotRow) -> SessionItem {
    let provider_session_id = snapshot
        .provider_session_id
        .as_deref()
        .unwrap_or(&snapshot.canonical_session_id);
    SessionItem {
        session_id: provider_session_id.to_string(),
        title: snapshot.title.clone(),
        native_title: snapshot.title.clone(),
        display_title: snapshot.display_title.clone(),
        hidden: snapshot.hidden,
        pinned: snapshot.pinned,
        stale: snapshot.stale,
        preferred_targets: snapshot.preferred_targets.clone(),
        project_dir: snapshot
            .workspace_dir
            .as_deref()
            .map(utils::user_visible_path),
        last_active_at: snapshot.last_active_at_ms,
        source_path: snapshot
            .source_path
            .as_deref()
            .map(utils::user_visible_path),
        provider_id: snapshot.provider_id.clone(),
        message_count: snapshot.message_count,
        size_bytes: snapshot.size_bytes,
        hook_runtime_summary: None,
        hook_diagnosis: None,
    }
}

pub(super) fn session_matches_hook_filter(
    item: &SessionItem,
    hook_filter: &SessionHookFilter,
) -> bool {
    use crate::hooks::augmentation::SessionHookDiagnosisKind;

    match hook_filter {
        SessionHookFilter::All => true,
        SessionHookFilter::Attention => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| {
                matches!(
                    diagnosis.kind,
                    SessionHookDiagnosisKind::HookNotInstalled
                        | SessionHookDiagnosisKind::HookNeedsAttention
                        | SessionHookDiagnosisKind::NoEventsYet
                        | SessionHookDiagnosisKind::NoActiveRuntime
                        | SessionHookDiagnosisKind::NoSessionMatch
                )
            })
            .unwrap_or(false),
        SessionHookFilter::Weak => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| diagnosis.kind == SessionHookDiagnosisKind::WeaklyLinked)
            .unwrap_or(false),
        SessionHookFilter::Runtime => item.hook_runtime_summary.is_some(),
        SessionHookFilter::NoHook => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| {
                matches!(
                    diagnosis.kind,
                    SessionHookDiagnosisKind::HookNotInstalled
                        | SessionHookDiagnosisKind::HookUnsupported
                )
            })
            .unwrap_or(false),
        SessionHookFilter::NoMatch => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| diagnosis.kind == SessionHookDiagnosisKind::NoSessionMatch)
            .unwrap_or(false),
        SessionHookFilter::Linked => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| diagnosis.kind == SessionHookDiagnosisKind::Linked)
            .unwrap_or(false),
    }
}

pub(super) fn sort_session_items(items: &mut [SessionItem], sort: &SessionListSort) {
    items.sort_by(|left, right| compare_session_items(left, right, sort));
}

pub(super) fn compare_session_items(
    left: &SessionItem,
    right: &SessionItem,
    sort: &SessionListSort,
) -> std::cmp::Ordering {
    let pin_order = right.pinned.cmp(&left.pinned);
    if pin_order != std::cmp::Ordering::Equal {
        return pin_order;
    }

    match sort {
        SessionListSort::Recent => compare_recent_then_title(left, right),
        SessionListSort::Title => compare_title_then_recent(left, right),
        SessionListSort::HookAttention => {
            let severity_order = hook_attention_priority(left).cmp(&hook_attention_priority(right));
            if severity_order != std::cmp::Ordering::Equal {
                return severity_order;
            }
            compare_recent_then_title(left, right)
        }
    }
}

pub(super) fn compare_recent_then_title(
    left: &SessionItem,
    right: &SessionItem,
) -> std::cmp::Ordering {
    right
        .last_active_at
        .cmp(&left.last_active_at)
        .then_with(|| session_display_key(left).cmp(&session_display_key(right)))
}

pub(super) fn compare_title_then_recent(
    left: &SessionItem,
    right: &SessionItem,
) -> std::cmp::Ordering {
    session_display_key(left)
        .cmp(&session_display_key(right))
        .then_with(|| right.last_active_at.cmp(&left.last_active_at))
}

pub(super) fn session_display_key(item: &SessionItem) -> String {
    item.display_title
        .as_deref()
        .or(item.title.as_deref())
        .unwrap_or(&item.session_id)
        .to_ascii_lowercase()
}

pub(super) fn hook_attention_priority(item: &SessionItem) -> usize {
    use crate::hooks::augmentation::SessionHookDiagnosisKind;

    match item
        .hook_diagnosis
        .as_ref()
        .map(|diagnosis| &diagnosis.kind)
    {
        Some(SessionHookDiagnosisKind::HookNeedsAttention) => 0,
        Some(SessionHookDiagnosisKind::NoSessionMatch) => 1,
        Some(SessionHookDiagnosisKind::HookNotInstalled) => 2,
        Some(SessionHookDiagnosisKind::NoActiveRuntime) => 3,
        Some(SessionHookDiagnosisKind::NoEventsYet) => 4,
        Some(SessionHookDiagnosisKind::WeaklyLinked) => 5,
        Some(SessionHookDiagnosisKind::Linked) => 6,
        Some(SessionHookDiagnosisKind::HookUnsupported) => 7,
        None => 8,
    }
}

#[cfg(test)]
mod session_list_hook_tests {
    use super::*;
    use crate::hooks::augmentation::{SessionHookDiagnosis, SessionHookDiagnosisKind};
    use crate::hooks::model::HookHealthStatus;

    fn session_item(
        session_id: &str,
        last_active_at: Option<i64>,
        diagnosis: Option<SessionHookDiagnosisKind>,
        has_runtime: bool,
    ) -> SessionItem {
        SessionItem {
            session_id: session_id.to_string(),
            title: Some(session_id.to_string()),
            native_title: Some(session_id.to_string()),
            display_title: None,
            hidden: false,
            pinned: false,
            stale: false,
            preferred_targets: Vec::new(),
            project_dir: Some("/tmp/project".to_string()),
            last_active_at,
            source_path: None,
            provider_id: "claude".to_string(),
            message_count: None,
            size_bytes: None,
            hook_runtime_summary: has_runtime.then(|| {
                crate::hooks::augmentation::HookRuntimeSummary {
                    linked_sessions: 1,
                    waiting_sessions: 0,
                    status: crate::hooks::model::RuntimeSessionStatus::Running,
                    current_tool_name: None,
                    has_pending_permission: false,
                    has_pending_question: false,
                    last_event_at: None,
                    matched_by: None,
                    confidence: None,
                }
            }),
            hook_diagnosis: diagnosis.map(|kind| SessionHookDiagnosis {
                kind,
                provider_status: HookHealthStatus::InstalledOk,
                linked_runtime_sessions: usize::from(has_runtime),
                provider_runtime_sessions: usize::from(has_runtime),
                matched_by: None,
                confidence: None,
                last_event_at: None,
                message: String::new(),
                actions: Vec::new(),
            }),
        }
    }

    #[test]
    fn session_matches_attention_hook_filter() {
        let attention = session_item(
            "session-attention",
            Some(20),
            Some(SessionHookDiagnosisKind::HookNeedsAttention),
            false,
        );
        let linked = session_item(
            "session-linked",
            Some(10),
            Some(SessionHookDiagnosisKind::Linked),
            true,
        );

        assert!(session_matches_hook_filter(
            &attention,
            &SessionHookFilter::Attention
        ));
        assert!(!session_matches_hook_filter(
            &linked,
            &SessionHookFilter::Attention
        ));
    }

    #[test]
    fn session_matches_runtime_hook_filter() {
        let runtime = session_item("session-runtime", Some(20), None, true);
        let offline = session_item("session-offline", Some(10), None, false);

        assert!(session_matches_hook_filter(
            &runtime,
            &SessionHookFilter::Runtime
        ));
        assert!(!session_matches_hook_filter(
            &offline,
            &SessionHookFilter::Runtime
        ));
    }

    #[test]
    fn hook_attention_sort_prioritizes_diagnosis_before_recency() {
        let mut items = vec![
            session_item(
                "linked-newer",
                Some(300),
                Some(SessionHookDiagnosisKind::Linked),
                true,
            ),
            session_item(
                "attention-older",
                Some(100),
                Some(SessionHookDiagnosisKind::HookNeedsAttention),
                false,
            ),
            session_item(
                "weak-middle",
                Some(200),
                Some(SessionHookDiagnosisKind::WeaklyLinked),
                true,
            ),
        ];

        sort_session_items(&mut items, &SessionListSort::HookAttention);

        assert_eq!(items[0].session_id, "attention-older");
        assert_eq!(items[1].session_id, "weak-middle");
        assert_eq!(items[2].session_id, "linked-newer");
    }

    #[test]
    fn projected_snapshot_groups_filter_workspace_and_apply_limit() {
        let params = SessionListParams {
            all: false,
            providers: vec!["claude".to_string()],
            cwd: Some("/tmp/project".to_string()),
            include_message_counts: true,
            limit: Some(1),
            offset: None,
            sort: SessionListSort::Recent,
            hook_filter: SessionHookFilter::All,
        };
        let groups = projected_snapshot_groups(
            vec![
                projected_row("canonical-new", "native-new", "/tmp/project", 30),
                projected_row("canonical-old", "native-old", "/tmp/project", 20),
                projected_row("canonical-other", "native-other", "/tmp/other", 40),
            ],
            &params,
            &[],
            &HashMap::new(),
        );

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].provider_id, "claude");
        assert_eq!(groups[0].sessions.len(), 1);
        assert_eq!(groups[0].sessions[0].session_id, "native-new");
        assert_eq!(groups[0].sessions[0].message_count, Some(3));
        assert_eq!(
            groups[0].sessions[0].project_dir.as_deref(),
            Some("/tmp/project")
        );
    }

    #[test]
    fn projected_snapshot_item_exposes_stale_snapshot_state() {
        let mut row = projected_row("canonical-1", "native-1", "/tmp/project", 30);
        row.stale = true;

        let item = projected_snapshot_item(&row);

        assert!(item.stale);
    }

    fn projected_row(
        canonical_session_id: &str,
        provider_session_id: &str,
        workspace_dir: &str,
        last_active_at_ms: i64,
    ) -> ProjectedSessionSnapshotRow {
        ProjectedSessionSnapshotRow {
            canonical_session_id: canonical_session_id.to_string(),
            provider_id: "claude".to_string(),
            provider_session_id: Some(provider_session_id.to_string()),
            title: Some(provider_session_id.to_string()),
            display_title: None,
            workspace_dir: Some(workspace_dir.to_string()),
            last_active_at_ms: Some(last_active_at_ms),
            source_path: Some(format!("/tmp/{provider_session_id}.jsonl")),
            message_count: Some(3),
            event_count: 5,
            turn_count: 2,
            size_bytes: Some(128),
            hidden: false,
            pinned: false,
            preferred_targets: Vec::new(),
            stale: false,
        }
    }
}
