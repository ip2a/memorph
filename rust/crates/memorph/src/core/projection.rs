use super::*;

static PROJECTION_OPERATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static PROJECTION_INITIALIZED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

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
    // ponytail: 首次 list 同步跑一次 bootstrap,保证全新环境也能看到会话;
    // 首次会同步等一次 scan。要秒回则改为 server 启动时后台预热 + 此处兜底。
    PROJECTION_INITIALIZED.get_or_init(|| {
        let _ = bootstrap_session_projections(None, ActivityActor::System);
    });
    list_projected_session_snapshots(params)
}

pub fn refresh_projected_session_staleness(
    actor: ActivityActor,
) -> Result<SnapshotStaleScanReport> {
    let _operation = PROJECTION_OPERATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Projection operation lock is poisoned"))?;
    let started_at = std::time::Instant::now();
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
    crate::logging::info(
        "snapshot_staleness",
        format!("completed in {} ms", started_at.elapsed().as_millis()),
    );
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
    let _operation = PROJECTION_OPERATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Projection operation lock is poisoned"))?;
    let started_at = std::time::Instant::now();
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
    crate::logging::info(
        "projection_bootstrap",
        format!("completed in {} ms", started_at.elapsed().as_millis()),
    );
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
            let needs_creation = session.created_at.is_some()
                && session_creation_time_needs_backfill(conn, provider_id, &session.session_id)
                    .unwrap_or(false);
            if !needs_creation {
                report.unchanged_sessions += 1;
                return;
            }
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
        Ok(stored) => stored,
        Err(error) => {
            report.failed_sessions += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("failed to project provider session: {error:#}"),
            ));
            return;
        }
    };
    report.projected_sessions += 1;
}

fn session_creation_time_needs_backfill(
    conn: &rusqlite::Connection,
    provider_id: &str,
    provider_session_id: &str,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM sessions
           WHERE provider_id = ?1
             AND provider_session_id = ?2
             AND created_at_ms IS NULL
             AND deleted_at_ms IS NULL
         )",
        rusqlite::params![provider_id, provider_session_id],
        |row| row.get(0),
    )
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
    let _operation = PROJECTION_OPERATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Projection operation lock is poisoned"))?;
    let started_at = std::time::Instant::now();
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
    crate::logging::info(
        "stale_reprojection",
        format!("completed in {} ms", started_at.elapsed().as_millis()),
    );
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
    Ok(projected_snapshot_groups(snapshots, params))
}

pub(super) fn projected_snapshot_groups(
    snapshots: Vec<ProjectedSessionSnapshotRow>,
    params: &SessionListParams,
) -> Vec<SessionGroup> {
    let provider_ids = resolve_providers(&params.providers);
    let requested_workspace = if params.all {
        None
    } else {
        params.cwd.as_deref()
    };
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(usize::MAX);
    let mut snapshots_by_provider: HashMap<String, Vec<ProjectedSessionSnapshotRow>> =
        HashMap::new();
    for snapshot in snapshots {
        snapshots_by_provider
            .entry(snapshot.provider_id.clone())
            .or_default()
            .push(snapshot);
    }

    provider_ids
        .into_iter()
        .filter_map(|provider_id| {
            let provider_name = providers::find_provider(&provider_id)
                .map(|provider| provider.name().to_string())
                .unwrap_or_else(|| provider_id.clone());
            let requested_workspace_key = requested_workspace.and_then(|workspace| {
                session_management::normalized_workspace_key(&provider_id, Some(workspace))
            });
            let mut sessions: Vec<SessionItem> = snapshots_by_provider
                .remove(&provider_id)
                .unwrap_or_default()
                .into_iter()
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
                .map(|snapshot| projected_snapshot_item(&snapshot))
                .collect();

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
    }
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
    }
}

pub(super) fn sort_session_items(items: &mut [SessionItem], sort: &SessionListSort) {
    items.sort_by(|left, right| compare_session_items(left, right, sort));
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
