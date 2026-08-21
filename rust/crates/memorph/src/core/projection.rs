use super::*;
use std::path::PathBuf;

static PROJECTION_OPERATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// Index a single session by id on demand.
///
/// Used by read paths that hit an unindexed identity: instead of triggering a
/// full provider-wide bootstrap, this asks the provider for one session via
/// find_session_by_id (which defaults to None for providers that cannot resolve
/// a session without scanning) and writes just that one row into SQLite.
/// Returns true when the session was found and indexed.
pub fn index_single_session(provider_id: &str, session_id: &str) -> Result<bool> {
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    let Some(session) = provider.find_session_by_id(session_id)? else {
        return Ok(false);
    };
    let mut conn = local_store::open_database()?;
    let mut report = SessionProjectionBootstrapReport::default();
    bootstrap_provider_session(&mut conn, provider_id, &session, &mut report);
    Ok(report.projected_sessions > 0 || report.unchanged_sessions > 0)
}

/// Index all sessions within a workspace directory for one provider.
///
/// Uses scan_workspace when the provider implements it (returns non-empty);
/// otherwise falls back to scan_sessions and filters by workspace key.
/// Returns the projection report for the scoped set.
pub fn index_workspace_sessions(
    provider_id: &str,
    workspace_dir: &std::path::Path,
    actor: ActivityActor,
) -> Result<SessionProjectionBootstrapReport> {
    let _operation = PROJECTION_OPERATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Projection operation lock is poisoned"))?;
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;

    // Prefer the provider's workspace-scoped scan; fall back to full scan + filter.
    let mut sessions = provider.scan_workspace(workspace_dir)?;
    let used_scope = !sessions.is_empty();
    if !used_scope {
        let all = provider.scan_sessions()?;
        sessions = all
            .into_iter()
            .filter(|summary| {
                provider.workspace_matches(
                    summary.project_dir.as_deref(),
                    Some(workspace_dir.to_string_lossy().as_ref()),
                )
            })
            .collect();
    }

    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(provider_id.to_string()),
        provider_session_id: None,
        workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
        operation_kind: ActivityOperationKind::Scan,
        actor,
        summary: format!(
            "Indexing sessions for provider {provider_id} in workspace {workspace_dir_display}",
            workspace_dir_display = workspace_dir.display()
        ),
        details: serde_json::json!({
            "scan_kind": "workspace_index",
            "provider": provider_id,
            "workspace_dir": workspace_dir.to_string_lossy(),
            "scoped_scan": used_scope,
        }),
    })?;

    let result = (|| {
        let mut conn = local_store::open_database()?;
        let mut report = SessionProjectionBootstrapReport::default();
        report.scanned_providers = 1;
        report.discovered_sessions = sessions.len();
        for session in &sessions {
            bootstrap_provider_session(&mut conn, provider_id, session, &mut report);
        }
        // Bump the workspace feed revision when real projections landed, so the
        // home view can refetch silently. A scan that only confirmed unchanged
        // sessions (projected_sessions == 0) does not bump.
        if report.projected_sessions > 0 {
            if let Some(workspace_key) = super::session_management::normalized_workspace_key(
                provider_id,
                Some(&workspace_dir.to_string_lossy()),
            ) {
                let _ = crate::storage::workspace_feed_revision::bump(
                    &conn,
                    &workspace_key,
                    crate::utils::now_ms(),
                );
            }
        }
        Ok(report)
    })();

    match result {
        Ok(report) => {
            let status = if report.failed_sessions == 0 && report.missing_sources == 0 {
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
                    provider_id: Some(provider_id.to_string()),
                    provider_session_id: None,
                    workspace_dir: Some(workspace_dir.to_string_lossy().to_string()),
                    summary: format!(
                        "Indexed {} sessions for provider {provider_id}",
                        report.discovered_sessions
                    ),
                    details: serde_json::to_value(&report)?,
                    error: (!report.failures.is_empty()).then(|| {
                        report
                            .failures
                            .iter()
                            .map(|f| f.reason.as_str())
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
                    "Failed to index workspace sessions",
                    serde_json::json!({
                        "scan_kind": "workspace_index",
                        "provider": provider_id,
                    }),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

/// Readiness-only workspace projection. Unlike the explicit workspace index
/// endpoint, this never falls back to a provider-wide full scan.
pub fn reconcile_workspace_session_projections(
    workspace_dir: &std::path::Path,
    actor: ActivityActor,
) -> Result<SessionProjectionBootstrapReport> {
    let _operation = PROJECTION_OPERATION_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Projection operation lock is poisoned"))?;
    let workspace_dir = workspace_dir
        .canonicalize()
        .with_context(|| format!("Failed to resolve workspace: {}", workspace_dir.display()))?;
    reconcile_workspace_session_projections_for(
        &workspace_dir,
        actor,
        &readiness_workspace_provider_ids(),
    )
}

pub(super) fn reconcile_workspace_session_projections_for(
    workspace_dir: &std::path::Path,
    actor: ActivityActor,
    provider_ids: &[&str],
) -> Result<SessionProjectionBootstrapReport> {
    let workspace = workspace_dir.to_string_lossy().into_owned();
    let mut report = SessionProjectionBootstrapReport::default();

    for provider_id in provider_ids.iter().copied() {
        let activity_conn = local_store::open_database()?;
        let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
            provider_id: Some(provider_id.to_string()),
            provider_session_id: None,
            workspace_dir: Some(workspace.clone()),
            operation_kind: ActivityOperationKind::Scan,
            actor,
            summary: format!("Projecting {provider_id} workspace session metadata"),
            details: serde_json::json!({
                "scan_kind": "readiness_workspace_projection",
                "workspace_dir": workspace,
            }),
        })?;
        let Some(provider) = providers::find_provider(provider_id) else {
            report.unsupported_providers += 1;
            let reason = format!("provider is not registered: {provider_id}");
            report.failures.push(SessionProjectionBootstrapFailure {
                provider_id: provider_id.to_string(),
                session_id: None,
                source_path: None,
                reason: reason.clone(),
            });
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Workspace metadata projection is unsupported",
                    serde_json::json!({"scan_kind": "readiness_workspace_projection"}),
                    reason,
                ),
            )?;
            continue;
        };
        let Some(scan) = scoped_safe_workspace_scan(provider.as_ref(), &workspace_dir) else {
            report.unsupported_providers += 1;
            let reason = format!(
                "provider has no safe workspace or lightweight readiness scan: {provider_id}"
            );
            report.failures.push(SessionProjectionBootstrapFailure {
                provider_id: provider_id.to_string(),
                session_id: None,
                source_path: None,
                reason: reason.clone(),
            });
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Workspace metadata projection is unsupported",
                    serde_json::json!({"scan_kind": "readiness_workspace_projection"}),
                    reason,
                ),
            )?;
            continue;
        };

        let sessions = match scan {
            Ok(sessions) => sessions,
            Err(error) => {
                report.failed_providers += 1;
                let reason = format!("failed to scan workspace session metadata: {error:#}");
                report.failures.push(SessionProjectionBootstrapFailure {
                    provider_id: provider_id.to_string(),
                    session_id: None,
                    source_path: None,
                    reason: reason.clone(),
                });
                ActivityStore::new(&activity_conn).finish(
                    &activity_id,
                    ActivityCompletion::failed(
                        "Workspace metadata projection failed",
                        serde_json::json!({"scan_kind": "readiness_workspace_projection"}),
                        reason,
                    ),
                )?;
                continue;
            }
        };
        report.scanned_providers += 1;
        report.discovered_sessions += sessions.len();
        let before_failures = report.failed_sessions + report.missing_sources;
        let mut conn = local_store::open_database()?;
        for session in &sessions {
            bootstrap_provider_session(&mut conn, provider_id, session, &mut report);
        }
        let provider_failed = report.failed_sessions + report.missing_sources > before_failures;
        let status = if provider_failed {
            ActivityStatus::Partial
        } else {
            ActivityStatus::Success
        };
        ActivityStore::new(&activity_conn).finish(
            &activity_id,
            ActivityCompletion {
                status,
                provider_id: Some(provider_id.to_string()),
                provider_session_id: None,
                workspace_dir: Some(workspace.clone()),
                summary: format!("Projected {} workspace sessions", sessions.len()),
                details: serde_json::json!({
                    "scan_kind": "readiness_workspace_projection",
                    "discovered_sessions": sessions.len(),
                    "scoped_scan": provider.supports_workspace_scan(),
                    "lightweight_scan": !provider.supports_workspace_scan(),
                }),
                error: provider_failed
                    .then(|| "One or more session summaries could not be projected".to_string()),
            },
        )?;
    }
    Ok(report)
}

pub(super) fn readiness_workspace_provider_ids() -> Vec<&'static str> {
    readiness_workspace_provider_ids_with(|provider_id| {
        let environment = crate::agent_environment::detect_provider_environment_fast(provider_id);
        provider_is_relevant(
            environment.installed,
            crate::agent_environment::provider_config_path(provider_id).exists(),
        )
    })
}

fn readiness_workspace_provider_ids_with(
    mut relevant: impl FnMut(&str) -> bool,
) -> Vec<&'static str> {
    PROJECTED_SESSION_PROVIDER_IDS
        .iter()
        .copied()
        .filter(|provider_id| relevant(provider_id))
        .collect()
}

fn provider_is_relevant(environment_installed: bool, config_path_exists: bool) -> bool {
    environment_installed || config_path_exists
}

fn scoped_safe_workspace_scan(
    provider: &dyn Provider,
    workspace_dir: &std::path::Path,
) -> Option<Result<Vec<ProviderSessionSummary>>> {
    if provider.supports_workspace_scan() {
        return Some(provider.scan_workspace(workspace_dir));
    }
    if !provider.capabilities().lightweight_scan {
        return None;
    }
    Some(provider.scan_sessions_lightweight().map(|sessions| {
        sessions
            .into_iter()
            .filter(|session| {
                provider.workspace_matches(
                    session.project_dir.as_deref(),
                    Some(workspace_dir.to_string_lossy().as_ref()),
                )
            })
            .collect()
    }))
}

#[cfg(test)]
mod readiness_projection_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct LightweightProvider<'a> {
        lightweight_called: &'a AtomicBool,
    }

    impl Provider for LightweightProvider<'_> {
        fn id(&self) -> &'static str {
            "test-lightweight"
        }

        fn name(&self) -> &'static str {
            "test-lightweight"
        }

        fn capabilities(&self) -> crate::provider::ProviderCapabilities {
            crate::provider::ProviderCapabilities {
                lightweight_scan: true,
                ..Default::default()
            }
        }

        fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
            panic!("readiness must not invoke the full scan fallback")
        }

        fn scan_sessions_lightweight(&self) -> Result<Vec<ProviderSessionSummary>> {
            self.lightweight_called.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }

        fn import_session(&self, _source_path: &str) -> Result<ImportedSession> {
            anyhow::bail!("not used")
        }
    }

    #[test]
    fn readiness_projection_uses_lightweight_scan_without_full_fallback() {
        let called = AtomicBool::new(false);
        let provider = LightweightProvider {
            lightweight_called: &called,
        };
        let workspace = tempfile::tempdir().unwrap();

        let sessions = scoped_safe_workspace_scan(&provider, workspace.path())
            .expect("supported lightweight scan")
            .unwrap();

        assert!(sessions.is_empty());
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn irrelevant_unsupported_provider_is_not_expected() {
        let expected = readiness_workspace_provider_ids_with(|provider_id| provider_id == "cursor");

        assert_eq!(expected, vec!["cursor"]);
        assert!(!expected.contains(&"claude"));
        assert!(!provider_is_relevant(false, false));
    }

    #[test]
    fn provider_config_or_installation_makes_provider_relevant() {
        assert!(provider_is_relevant(true, false));
        assert!(provider_is_relevant(false, true));
    }
}

/// Spawn a one-shot background thread that runs `index_workspace_sessions`
/// for every projected provider. Used by read paths that hit an empty SQLite
/// for a workspace: the API returns immediately with a `degraded` flag while
/// this thread warms the index. Per-session fingerprint dedup keeps repeated
/// triggers cheap.
// ponytail: one thread per trigger; if trigger rate becomes a concern, gate
// behind a debounce or a per-workspace in-flight map.
pub fn spawn_workspace_index_background(workspace_dir: PathBuf, actor: ActivityActor) {
    let workspace_dir = workspace_dir.canonicalize().unwrap_or(workspace_dir);
    std::thread::Builder::new()
        .name(format!(
            "memorph-workspace-index-{}",
            workspace_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("workspace")
        ))
        .spawn(move || {
            for provider_id in crate::core::PROJECTED_SESSION_PROVIDER_IDS.iter() {
                if let Err(error) =
                    index_workspace_sessions(provider_id, &workspace_dir, actor.clone())
                {
                    crate::logging::error(
                        "workspace_index_background",
                        format!(
                            "provider {provider_id} workspace {} index failed: {error:#}",
                            workspace_dir.display()
                        ),
                    );
                }
            }
        })
        .ok();
}

/// Spawn background workspace indexers for every workspace in the user's
/// history (config::known_workspaces). Non-blocking: fires off one thread per
/// workspace and returns immediately. Used by the "Scan Workspaces" button in
/// the workspace switcher so an empty picker can repopulate without a full
/// provider-wide bootstrap.
// ponytail: one thread per workspace; if history grows large, debounce or
// switch to a bounded worker pool.
pub fn scan_known_workspaces_background(actor: ActivityActor) -> usize {
    let workspaces = match crate::config::known_workspaces() {
        Ok(entries) => entries.into_iter().map(|e| e.path).collect::<Vec<_>>(),
        Err(error) => {
            crate::logging::error(
                "scan_known_workspaces",
                format!("Failed to read known workspaces: {error:#}"),
            );
            return 0;
        }
    };
    let count = workspaces.len();
    for path in workspaces {
        let workspace_dir = std::path::PathBuf::from(path);
        spawn_workspace_index_background(workspace_dir, actor.clone());
    }
    count
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
            Ok(_) => {
                report.reprojected_snapshots += 1;
                bump_feed_revision_for_session(conn, &source.provider_id, provider_session_id);
            }
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

/// Bump the feed revision for the workspace a projected session belongs to.
/// Used by the reproject and mutation paths that change display data outside
/// the workspace-scoped scan path. Sessions with no workspace, or no row at
/// all, are skipped — they are not part of any workspace feed. The lookup
/// ignores `deleted_at_ms` so it still resolves the workspace when a caller
/// captures the key before a hard delete.
pub(super) fn bump_feed_revision_for_session(
    conn: &rusqlite::Connection,
    provider_id: &str,
    provider_session_id: &str,
) {
    let workspace_dir: Option<String> = conn
        .query_row(
            "SELECT workspace_dir FROM sessions
             WHERE provider_id = ?1 AND provider_session_id = ?2",
            rusqlite::params![provider_id, provider_session_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());
    let Some(workspace_dir) = workspace_dir else {
        return;
    };
    let Some(workspace_key) =
        super::session_management::normalized_workspace_key(provider_id, Some(&workspace_dir))
    else {
        return;
    };
    let _ =
        crate::storage::workspace_feed_revision::bump(conn, &workspace_key, crate::utils::now_ms());
}

/// Resolve the workspace feed key for a session row without bumping. Used to
/// capture the key before a hard delete so the revision can be bumped after.
pub(super) fn workspace_key_for_session(
    conn: &rusqlite::Connection,
    provider_id: &str,
    provider_session_id: &str,
) -> Option<String> {
    let workspace_dir: Option<String> = conn
        .query_row(
            "SELECT workspace_dir FROM sessions
             WHERE provider_id = ?1 AND provider_session_id = ?2",
            rusqlite::params![provider_id, provider_session_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());
    let Some(workspace_dir) = workspace_dir else {
        return None;
    };
    super::session_management::normalized_workspace_key(provider_id, Some(&workspace_dir))
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
    let store = crate::storage::snapshot_store::SnapshotStore::new(&conn);
    let snapshots = store.list_session_snapshots_filtered(
        Some(&provider_ids),
        workspace_scopes.as_deref(),
        params.fields.include_stats(),
    )?;
    let mut metadata_filter = params.filter.clone();
    metadata_filter.text = None;
    // ponytail: in-memory metadata filtering is enough for personal datasets; push into SQL if slow.
    let snapshots: Vec<_> = snapshots
        .into_iter()
        .filter(|snapshot| snapshot_passes_filter(snapshot, &metadata_filter, None))
        .collect();
    let text_matches = params
        .filter
        .text
        .as_deref()
        .map(|pattern| search_projected_session_text(&snapshots, pattern))
        .transpose()?;
    let snapshots = snapshots
        .into_iter()
        .filter(|snapshot| snapshot_passes_filter(snapshot, &params.filter, text_matches.as_ref()))
        .collect();
    Ok(projected_snapshot_groups(snapshots, params))
}

fn search_projected_session_text(
    snapshots: &[ProjectedSessionSnapshotRow],
    pattern: &str,
) -> Result<HashSet<String>> {
    let query = pattern.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Ok(snapshots
            .iter()
            .map(|snapshot| snapshot.canonical_session_id.clone())
            .collect());
    }

    let mut matches = HashSet::new();
    for snapshot in snapshots {
        let source_path = snapshot
            .source_path
            .as_deref()
            .filter(|path| !path.is_empty())
            .with_context(|| {
                format!(
                    "Session has no source locator: {}/{}",
                    snapshot.provider_id,
                    snapshot
                        .provider_session_id
                        .as_deref()
                        .unwrap_or(&snapshot.canonical_session_id)
                )
            })?;
        let provider = providers::find_provider(&snapshot.provider_id)
            .with_context(|| format!("Unknown provider: {}", snapshot.provider_id))?;
        if !provider.capabilities().import {
            anyhow::bail!(
                "Provider does not support session text search: {}",
                snapshot.provider_id
            );
        }
        let imported = provider.import_session(source_path).with_context(|| {
            format!(
                "Failed to search session text: {}/{}",
                snapshot.provider_id,
                snapshot
                    .provider_session_id
                    .as_deref()
                    .unwrap_or(&snapshot.canonical_session_id)
            )
        })?;
        if imported.session.events.iter().any(|event| {
            provider::event_text(event)
                .to_ascii_lowercase()
                .contains(&query)
        }) {
            matches.insert(snapshot.canonical_session_id.clone());
        }
    }
    Ok(matches)
}

pub(super) fn snapshot_passes_filter(
    snapshot: &ProjectedSessionSnapshotRow,
    filter: &SessionListFilter,
    text_matches: Option<&HashSet<String>>,
) -> bool {
    let dir_matches = filter.dir.as_ref().is_none_or(|pattern| {
        snapshot
            .workspace_dir
            .as_ref()
            .is_some_and(|value| value.contains(pattern))
            || snapshot
                .source_path
                .as_ref()
                .is_some_and(|value| value.contains(pattern))
    });
    let session_matches = filter.session.as_ref().is_none_or(|pattern| {
        snapshot
            .provider_session_id
            .as_ref()
            .is_some_and(|value| value.contains(pattern))
            || snapshot.canonical_session_id.contains(pattern)
            || snapshot
                .title
                .as_ref()
                .is_some_and(|value| value.contains(pattern))
            || snapshot
                .display_title
                .as_ref()
                .is_some_and(|value| value.contains(pattern))
    });
    let title_matches = filter.title.as_ref().is_none_or(|pattern| {
        snapshot
            .title
            .as_ref()
            .is_some_and(|value| value.contains(pattern))
            || snapshot
                .display_title
                .as_ref()
                .is_some_and(|value| value.contains(pattern))
    });
    let text_matches = filter.text.is_none()
        || text_matches.is_some_and(|matches| matches.contains(&snapshot.canonical_session_id));
    let since_matches = filter.since_ms.is_none_or(|since| {
        snapshot
            .last_active_at_ms
            .is_none_or(|last_active| last_active >= since)
    });
    let before_matches = filter.before_ms.is_none_or(|before| {
        snapshot
            .last_active_at_ms
            .is_none_or(|last_active| last_active <= before)
    });
    let min_size_matches = filter
        .min_bytes
        .is_none_or(|min| snapshot.size_bytes.is_none_or(|size| size >= min));
    let max_size_matches = filter
        .max_bytes
        .is_none_or(|max| snapshot.size_bytes.is_none_or(|size| size <= max));

    dir_matches
        && session_matches
        && title_matches
        && text_matches
        && since_matches
        && before_matches
        && min_size_matches
        && max_size_matches
}

#[cfg(test)]
mod session_filter_tests {
    use super::*;

    fn snapshot() -> ProjectedSessionSnapshotRow {
        ProjectedSessionSnapshotRow {
            canonical_session_id: "canonical-1".to_string(),
            provider_id: "claude".to_string(),
            provider_session_id: Some("native-1".to_string()),
            title: Some("Native title".to_string()),
            display_title: Some("Local title".to_string()),
            workspace_dir: Some("/tmp/project".to_string()),
            last_active_at_ms: Some(200),
            source_path: Some("/tmp/project/session.jsonl".to_string()),
            message_count: Some(2),
            event_count: 3,
            turn_count: 1,
            size_bytes: Some(20),
            hidden: false,
            archived: false,
            pinned: false,
            preferred_targets: Vec::new(),
            stale: false,
        }
    }

    #[test]
    fn snapshot_filter_matches_all_fields_and_boundaries() {
        let row = snapshot();
        let cases = [
            (
                SessionListFilter {
                    dir: Some("project".into()),
                    ..Default::default()
                },
                true,
            ),
            (
                SessionListFilter {
                    session: Some("native-1".into()),
                    ..Default::default()
                },
                true,
            ),
            (
                SessionListFilter {
                    title: Some("Local title".into()),
                    ..Default::default()
                },
                true,
            ),
            (
                SessionListFilter {
                    since_ms: Some(200),
                    ..Default::default()
                },
                true,
            ),
            (
                SessionListFilter {
                    before_ms: Some(200),
                    ..Default::default()
                },
                true,
            ),
            (
                SessionListFilter {
                    min_bytes: Some(20),
                    ..Default::default()
                },
                true,
            ),
            (
                SessionListFilter {
                    max_bytes: Some(20),
                    ..Default::default()
                },
                true,
            ),
            (
                SessionListFilter {
                    dir: Some("missing".into()),
                    ..Default::default()
                },
                false,
            ),
            (
                SessionListFilter {
                    session: Some("missing".into()),
                    ..Default::default()
                },
                false,
            ),
            (
                SessionListFilter {
                    title: Some("missing".into()),
                    ..Default::default()
                },
                false,
            ),
            (
                SessionListFilter {
                    since_ms: Some(201),
                    ..Default::default()
                },
                false,
            ),
            (
                SessionListFilter {
                    before_ms: Some(199),
                    ..Default::default()
                },
                false,
            ),
            (
                SessionListFilter {
                    min_bytes: Some(21),
                    ..Default::default()
                },
                false,
            ),
            (
                SessionListFilter {
                    max_bytes: Some(19),
                    ..Default::default()
                },
                false,
            ),
        ];

        for (filter, expected) in cases {
            assert_eq!(
                snapshot_passes_filter(&row, &filter, None),
                expected,
                "{filter:?}"
            );
        }
    }

    #[test]
    fn source_backed_text_search_reads_provider_events() {
        use std::io::Write as _;

        let mut source = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            source,
            "{}",
            serde_json::json!({
                "type": "user",
                "uuid": "user-1",
                "sessionId": "native-1",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:00Z",
                "message": {"role": "user", "content": "Unique Search Needle"}
            })
        )
        .unwrap();
        let mut row = snapshot();
        row.source_path = Some(source.path().to_string_lossy().into_owned());

        assert_eq!(
            search_projected_session_text(&[row.clone()], "search needle").unwrap(),
            HashSet::from([row.canonical_session_id.clone()])
        );
        assert!(search_projected_session_text(&[row], "missing")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn snapshot_filter_text_requires_canonical_id_match() {
        let row = snapshot();
        let matches = HashSet::from(["canonical-1".to_string()]);
        assert!(snapshot_passes_filter(
            &row,
            &SessionListFilter {
                text: Some("needle".into()),
                ..Default::default()
            },
            Some(&matches),
        ));

        let misses = HashSet::from(["canonical-2".to_string()]);
        assert!(!snapshot_passes_filter(
            &row,
            &SessionListFilter {
                text: Some("needle".into()),
                ..Default::default()
            },
            Some(&misses),
        ));
    }
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

pub(crate) fn projected_snapshot_item(snapshot: &ProjectedSessionSnapshotRow) -> SessionItem {
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
        archived: snapshot.archived,
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

pub(crate) fn compare_session_items(
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

pub(crate) fn sort_session_items(items: &mut [SessionItem], sort: &SessionListSort) {
    items.sort_by(|left, right| compare_session_items(left, right, sort));
}

pub(crate) fn compare_recent_then_title(
    left: &SessionItem,
    right: &SessionItem,
) -> std::cmp::Ordering {
    right
        .last_active_at
        .cmp(&left.last_active_at)
        .then_with(|| session_display_key(left).cmp(&session_display_key(right)))
}

pub(crate) fn compare_title_then_recent(
    left: &SessionItem,
    right: &SessionItem,
) -> std::cmp::Ordering {
    session_display_key(left)
        .cmp(&session_display_key(right))
        .then_with(|| right.last_active_at.cmp(&left.last_active_at))
}

pub(crate) fn session_display_key(item: &SessionItem) -> String {
    item.display_title
        .as_deref()
        .or(item.title.as_deref())
        .unwrap_or(&item.session_id)
        .to_ascii_lowercase()
}
