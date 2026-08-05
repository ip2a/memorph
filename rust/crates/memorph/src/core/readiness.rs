use anyhow::{Context as _, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Mutex;

use crate::core::projection;
use crate::skills::{
    discovery, invocation, repository,
    scanner::{self, ScanMode},
};
use crate::storage::activity_store::{
    ActivityOperationKind, ActivityQuery, ActivityStatus, ActivityStore,
};
use crate::storage::local_store;

pub const PHASES: [&str; 7] = [
    "foundation",
    "agents",
    "sessions",
    "session_stats",
    "skills",
    "usage",
    "derived",
];

static CATALOG_OPERATION_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhaseState {
    Ready,
    Partial,
    Degraded,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhaseReadiness {
    pub state: PhaseState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PhaseReadiness {
    fn ready(message: impl Into<String>) -> Self {
        Self {
            state: PhaseState::Ready,
            message: Some(message.into()),
        }
    }

    fn partial(message: impl Into<String>) -> Self {
        Self {
            state: PhaseState::Partial,
            message: Some(message.into()),
        }
    }

    fn degraded(message: impl Into<String>) -> Self {
        Self {
            state: PhaseState::Degraded,
            message: Some(message.into()),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            state: PhaseState::Error,
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Readiness {
    pub workspace: Option<String>,
    pub state: PhaseState,
    pub active_operation_id: Option<String>,
    pub recommended_focus: Option<String>,
    pub phases: BTreeMap<String, PhaseReadiness>,
}

pub fn validate_workspace(workspace: Option<&str>) -> Result<Option<String>> {
    let Some(value) = workspace else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("Workspace path cannot be empty");
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        anyhow::bail!("Workspace must be an absolute path");
    }
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "Workspace does not exist or is inaccessible: {}",
            path.display()
        )
    })?;
    if !canonical.is_dir() {
        anyhow::bail!("Workspace must be an existing directory");
    }
    Ok(Some(canonical.to_string_lossy().into_owned()))
}

pub fn assess(workspace: Option<&str>, active_operation_id: Option<String>) -> Readiness {
    let workspace = match validate_workspace(workspace) {
        Ok(workspace) => workspace,
        Err(error) => {
            return error_readiness(workspace.map(str::to_string), active_operation_id, error)
        }
    };
    let mut phases = BTreeMap::new();

    let foundation = assess_foundation();
    let foundation_ready = foundation.state == PhaseState::Ready;
    phases.insert("foundation".into(), foundation);
    if foundation_ready {
        phases.insert("agents".into(), assess_agents());
        phases.insert("sessions".into(), assess_sessions(workspace.as_deref()));
        phases.insert(
            "session_stats".into(),
            assess_session_stats(workspace.as_deref()),
        );
        phases.insert("skills".into(), assess_skills(workspace.as_deref()));
        phases.insert("usage".into(), assess_usage(workspace.as_deref()));
    } else {
        for phase in ["agents", "sessions", "session_stats", "skills", "usage"] {
            phases.insert(
                phase.into(),
                PhaseReadiness::partial("Waiting for foundation"),
            );
        }
    }
    let inputs_ready = ["agents", "sessions", "session_stats", "skills", "usage"]
        .iter()
        .all(|phase| phases[*phase].state == PhaseState::Ready);
    phases.insert(
        "derived".into(),
        if inputs_ready {
            PhaseReadiness::ready("Readiness summary is current")
        } else {
            PhaseReadiness::partial("Waiting for readiness inputs")
        },
    );
    finish_readiness(workspace, active_operation_id, phases)
}

fn assess_foundation() -> PhaseReadiness {
    let result = (|| -> Result<()> {
        crate::config::load_config().context("Configuration is unreadable")?;
        let conn = local_store::open_database().context("Database is unavailable")?;
        let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        anyhow::ensure!(check == "ok", "Database quick check returned {check}");
        Ok(())
    })();
    match result {
        Ok(()) => PhaseReadiness::ready("Database, schema, and configuration are readable"),
        Err(error) => PhaseReadiness::error(format!("{error:#}")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundationRepair {
    pub selected_workspace: Option<String>,
    pub repaired: bool,
}

/// Recreate/reset configuration, run database migrations, and prime the
/// selected workspace using the same semantics as the system repair endpoint.
pub fn repair_foundation() -> Result<FoundationRepair> {
    let mut repaired = false;
    match crate::config::load_config() {
        Ok(existing) => crate::config::save_config(&existing)?,
        Err(_) => {
            crate::config::save_config(&crate::config::MemorphConfig::default())?;
            repaired = true;
        }
    }
    local_store::open_database()?;
    if crate::config::selected_workspace().ok().flatten().is_none() {
        let primed = crate::config::prime_default_workspace_if_unset().unwrap_or(false);
        if !primed {
            if let Some(latest) = crate::config::known_workspaces()
                .ok()
                .and_then(|mut entries| entries.drain(..).next().map(|entry| entry.path))
            {
                let _ = crate::config::remember_workspace(Path::new(&latest));
            }
        }
        repaired = true;
    }
    Ok(FoundationRepair {
        selected_workspace: crate::config::selected_workspace().ok().flatten(),
        repaired,
    })
}

fn assess_agents() -> PhaseReadiness {
    match crate::agent_management::list_agent_management_summaries() {
        Ok(entries) => PhaseReadiness::ready(format!("Assessed {} agent providers", entries.len())),
        Err(error) => PhaseReadiness::error(format!("Agent overview failed: {error:#}")),
    }
}

fn assess_sessions(workspace: Option<&str>) -> PhaseReadiness {
    let Some(workspace) = workspace else {
        return PhaseReadiness::ready("Workspace session projection is not applicable");
    };
    assess_workspace_checkpoints(workspace)
}

fn assess_workspace_checkpoints(workspace: &str) -> PhaseReadiness {
    let expected = projection::readiness_workspace_provider_ids();
    let result = (|| -> Result<HashMap<String, ActivityStatus>> {
        let conn = local_store::open_database()?;
        let records = ActivityStore::new(&conn).query(&ActivityQuery {
            workspace_dir: Some(workspace.to_string()),
            operation_kind: Some(ActivityOperationKind::Scan),
            limit: Some(500),
            ..ActivityQuery::default()
        })?;
        let mut latest = HashMap::<String, ActivityStatus>::new();
        for record in records {
            if record
                .details
                .get("scan_kind")
                .and_then(|value| value.as_str())
                != Some("readiness_workspace_projection")
            {
                continue;
            }
            if let Some(provider_id) = record.provider_id {
                latest.entry(provider_id).or_insert(record.status);
            }
        }
        Ok(latest)
    })();
    match result {
        Ok(latest) => assess_expected_workspace_checkpoints(&expected, &latest),
        Err(error) => {
            PhaseReadiness::error(format!("Session scan checkpoint is unreadable: {error:#}"))
        }
    }
}

fn assess_session_stats(workspace: Option<&str>) -> PhaseReadiness {
    let result = (|| -> Result<(usize, usize)> {
        let conn = local_store::open_database()?;
        let store = crate::storage::snapshot_store::SnapshotStore::new(&conn);
        let rows = store.list_session_snapshots()?;
        let scoped: Vec<_> = if let Some(workspace) = workspace {
            rows.into_iter()
                .filter(|row| {
                    crate::core::session_management::normalized_workspace_key(
                        &row.provider_id,
                        Some(workspace),
                    )
                    .as_deref()
                        == row.workspace_dir.as_deref()
                })
                .collect()
        } else {
            rows
        };
        let total = scoped.len();
        let incomplete = scoped.iter().filter(|r| r.message_count.is_none()).count();
        Ok((total, incomplete))
    })();
    match result {
        Ok((_, 0)) => PhaseReadiness::ready("All session message counts are complete"),
        Ok((total, incomplete)) => PhaseReadiness::partial(format!(
            "Session stats incomplete: {incomplete}/{total} sessions need message counting"
        )),
        Err(error) => PhaseReadiness::error(format!("Session stats check failed: {error:#}")),
    }
}

fn assess_expected_workspace_checkpoints(
    expected: &[&str],
    latest: &HashMap<String, ActivityStatus>,
) -> PhaseReadiness {
    let completed = expected
        .iter()
        .filter(|provider_id| latest.contains_key(**provider_id))
        .count();
    let successful = expected
        .iter()
        .filter(|provider_id| latest.get(**provider_id) == Some(&ActivityStatus::Success))
        .count();
    let failed = expected.iter().any(|provider_id| {
        latest
            .get(*provider_id)
            .is_some_and(|status| *status != ActivityStatus::Success)
    });
    if completed == expected.len() && successful == completed {
        PhaseReadiness::ready(format!(
            "Workspace session checkpoints are complete ({completed} providers)"
        ))
    } else if failed {
        PhaseReadiness::degraded(format!(
            "Workspace session scan has failed providers ({successful}/{completed} successful)"
        ))
    } else {
        PhaseReadiness::partial(format!(
            "Workspace session scan is incomplete ({completed}/{} providers)",
            expected.len()
        ))
    }
}

fn assess_skills(workspace: Option<&str>) -> PhaseReadiness {
    let result = (|| -> Result<(usize, usize, bool)> {
        let home = crate::config::effective_home_dir()?;
        let agents = discovery::agents(&home, workspace.map(Path::new));
        let conn = local_store::open_database()?;
        let mut complete = 0;
        let mut has_error = false;
        for agent in &agents {
            let root = agent.skills_dir.to_string_lossy();
            let state_key = format!("skill-root:{}:{root}", agent.agent_id);
            let status = conn
                .query_row(
                    "SELECT completeness_status FROM skill_scan_state
                     WHERE state_key = ?1 AND state_kind = 'skill-root' AND source_path = ?2",
                    rusqlite::params![state_key, root.as_ref()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match status.as_deref() {
                Some("complete") => complete += 1,
                Some("error") => has_error = true,
                _ => {}
            }
        }
        Ok((complete, agents.len(), has_error))
    })();
    match result {
        Ok((_, _, true)) => {
            PhaseReadiness::error("One or more expected skill roots failed to scan")
        }
        Ok((complete, expected, false)) if complete == expected => PhaseReadiness::ready(format!(
            "Global and selected-workspace skill roots are indexed ({complete} roots)"
        )),
        Ok((complete, expected, false)) => PhaseReadiness::partial(format!(
            "Skill root scan is incomplete ({complete}/{expected} roots)"
        )),
        Err(error) => PhaseReadiness::error(format!("Skill scan state is unreadable: {error:#}")),
    }
}

fn assess_usage(_workspace: Option<&str>) -> PhaseReadiness {
    let result = (|| -> Result<(Option<String>, usize, bool)> {
        let conn = local_store::open_database()?;
        let aggregate = conn
            .query_row(
                "SELECT completeness_status FROM skill_scan_state WHERE state_key = 'skill-sessions:all'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let sources = repository::session_sources(&conn)?;
        let catalog_generation = invocation::catalog_generation(&conn)?;
        let mut stale = 0;
        let mut has_error = false;
        for source in &sources {
            let state_key = format!("session-source:{}", source.id);
            if repository::session_source_scan_is_current(
                &conn,
                &state_key,
                &source.fingerprint,
                source.source_cursor.as_deref(),
                &catalog_generation,
            )? {
                continue;
            }
            let status = conn
                .query_row(
                    "SELECT completeness_status FROM skill_scan_state
                     WHERE state_key = ?1 AND state_kind = 'session-source'",
                    [state_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match status.as_deref() {
                Some("error") => {
                    stale += 1;
                    has_error = true;
                }
                _ => stale += 1,
            }
        }
        Ok((aggregate, stale, has_error))
    })();
    match result {
        Ok((Some(status), _, _)) if status == "error" => {
            PhaseReadiness::error("Global usage analysis failed")
        }
        Ok((_, stale, true)) => PhaseReadiness::degraded(format!(
            "Global usage analysis has {stale} failed or stale session sources"
        )),
        Ok((Some(status), 0, false)) if status == "complete" => PhaseReadiness::ready(
            "Global usage analysis is current for all persisted session sources",
        ),
        Ok((_, stale, false)) => PhaseReadiness::partial(format!(
            "Global usage analysis needs an incremental pass ({stale} stale session sources)"
        )),
        Err(error) => PhaseReadiness::error(format!("Usage index is unreadable: {error:#}")),
    }
}

fn error_readiness(
    workspace: Option<String>,
    active_operation_id: Option<String>,
    error: anyhow::Error,
) -> Readiness {
    let mut phases = PHASES
        .into_iter()
        .map(|phase| {
            (
                phase.to_string(),
                PhaseReadiness::partial("Assessment unavailable"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    phases.insert(
        "foundation".into(),
        PhaseReadiness::error(error.to_string()),
    );
    finish_readiness(workspace, active_operation_id, phases)
}

fn finish_readiness(
    workspace: Option<String>,
    active_operation_id: Option<String>,
    phases: BTreeMap<String, PhaseReadiness>,
) -> Readiness {
    let state = if phases["foundation"].state == PhaseState::Error {
        PhaseState::Error
    } else if phases
        .values()
        .any(|phase| matches!(phase.state, PhaseState::Error | PhaseState::Degraded))
    {
        PhaseState::Degraded
    } else if phases
        .values()
        .all(|phase| phase.state == PhaseState::Ready)
    {
        PhaseState::Ready
    } else {
        PhaseState::Partial
    };
    let recommended_focus = PHASES
        .iter()
        .find(|phase| phases[**phase].state != PhaseState::Ready)
        .map(|phase| {
            if *phase == "foundation" {
                "overview"
            } else {
                *phase
            }
            .to_string()
        });
    Readiness {
        workspace,
        state,
        active_operation_id,
        recommended_focus,
        phases,
    }
}

pub fn reconcile_phase(phase: &str, workspace: Option<&str>) -> Result<()> {
    match phase {
        "foundation" => repair_foundation().map(|_| ()),
        "agents" => crate::agent_management::list_agent_management_summaries().map(|_| ()),
        "sessions" => match workspace {
            Some(workspace) => {
                let report = projection::reconcile_workspace_session_projections(
                    Path::new(workspace),
                    crate::storage::activity_store::ActivityActor::Api,
                )?;
                if report.failed_providers > 0 || report.unsupported_providers > 0 {
                    anyhow::bail!(
                        "Workspace projection degraded: {} failed, {} unsupported",
                        report.failed_providers,
                        report.unsupported_providers
                    );
                }
                Ok(())
            }
            None => Ok(()),
        },
        "session_stats" => {
            let workspace_filter = workspace.map(str::to_string);
            let conn = local_store::open_database()?;
            let store = crate::storage::snapshot_store::SnapshotStore::new(&conn);
            let rows = store.list_session_snapshots()?;
            let scoped: Vec<_> = if let Some(ref ws) = workspace_filter {
                rows.into_iter()
                    .filter(|row| {
                        crate::core::session_management::normalized_workspace_key(
                            &row.provider_id,
                            Some(ws),
                        )
                        .as_deref()
                            == row.workspace_dir.as_deref()
                    })
                    .collect()
            } else {
                rows
            };
            let mut conn = local_store::open_database()?;
            crate::stats_dashboard::complete_missing_counts(&mut conn, &scoped)
        }
        "skills" => {
            let _guard = CATALOG_OPERATION_LOCK
                .lock()
                .map_err(|_| anyhow::anyhow!("Readiness catalog lock is poisoned"))?;
            reconcile_skills(workspace)
        }
        "usage" => {
            let _guard = CATALOG_OPERATION_LOCK
                .lock()
                .map_err(|_| anyhow::anyhow!("Readiness catalog lock is poisoned"))?;
            let mut store = local_store::LocalSqliteStore::open_default()?;
            invocation::index_sources_incrementally(store.connection_mut()).map(|_| ())
        }
        "derived" => {
            anyhow::bail!("Derived readiness is virtual and cannot be reconciled directly")
        }
        _ => anyhow::bail!("Unknown readiness phase: {phase}"),
    }
}

fn reconcile_skills(workspace: Option<&str>) -> Result<()> {
    let home = crate::config::effective_home_dir()?;
    let agents = discovery::agents(&home, workspace.map(Path::new));
    let overview = discovery::discover_catalog(&agents);
    scanner::persist_default(&overview, ScanMode::Incremental)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::activity_store::{ActivityActor, ActivityCompletion, NewActivity};
    use std::sync::{Mutex, OnceLock};

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record_successful_workspace_scan(workspace: &str) {
        let conn = local_store::open_database().unwrap();
        let store = ActivityStore::new(&conn);
        for provider_id in projection::readiness_workspace_provider_ids() {
            let id = store
                .start(NewActivity {
                    provider_id: Some(provider_id.to_string()),
                    provider_session_id: None,
                    workspace_dir: Some(workspace.to_string()),
                    operation_kind: ActivityOperationKind::Scan,
                    actor: ActivityActor::Api,
                    summary: "Scanning workspace metadata".into(),
                    details: serde_json::json!({
                        "scan_kind": "readiness_workspace_projection"
                    }),
                })
                .unwrap();
            store
                .finish(
                    &id,
                    ActivityCompletion::success(
                        "Scanned workspace metadata",
                        serde_json::json!({
                            "scan_kind": "readiness_workspace_projection",
                            "discovered_sessions": 0,
                        }),
                    ),
                )
                .unwrap();
        }
    }

    #[test]
    fn assessment_always_contains_the_contract_phases() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(dir.path().to_path_buf());
        let readiness = assess(None, None);
        crate::config::reset_test_home_dir();
        assert!(PHASES
            .into_iter()
            .all(|phase| readiness.phases.contains_key(phase)));
    }

    #[test]
    fn workspace_validation_rejects_relative_paths() {
        assert!(validate_workspace(Some("relative/path"))
            .unwrap_err()
            .to_string()
            .contains("absolute"));
    }

    #[test]
    fn empty_database_with_workspace_is_not_session_ready() {
        let _guard = test_guard();
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());

        let readiness = assess(workspace.path().to_str(), None);

        crate::config::reset_test_home_dir();
        assert_eq!(readiness.phases["sessions"].state, PhaseState::Partial);
    }

    #[test]
    fn projected_row_without_complete_provider_checkpoints_is_not_ready() {
        let _guard = test_guard();
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let workspace = workspace.path().canonicalize().unwrap();
        let workspace_text = workspace.to_string_lossy().into_owned();
        let conn = local_store::open_database().unwrap();
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, workspace_dir,
              first_seen_at_ms, last_seen_at_ms)
             VALUES ('source', 'cursor', 'provider-session', '/tmp/source', ?1, 1, 1)",
            [&workspace_text],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, workspace_dir)
             VALUES ('session', 'cursor', 'provider-session', 'source', ?1)",
            [&workspace_text],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_snapshots
             (session_id, provider_id, workspace_dir, updated_at_ms)
             VALUES ('session', 'cursor', ?1, 1)",
            [&workspace_text],
        )
        .unwrap();

        let readiness = assess(Some(&workspace_text), None);

        crate::config::reset_test_home_dir();
        assert_eq!(readiness.phases["sessions"].state, PhaseState::Partial);
    }

    #[test]
    fn workspace_checkpoint_does_not_satisfy_another_workspace() {
        let _guard = test_guard();
        let home = tempfile::tempdir().unwrap();
        let workspace_a = tempfile::tempdir().unwrap();
        let workspace_b = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let a = workspace_a.path().canonicalize().unwrap();
        let b = workspace_b.path().canonicalize().unwrap();
        record_successful_workspace_scan(&a.to_string_lossy());

        let readiness_a = assess(a.to_str(), None);
        let readiness_b = assess(b.to_str(), None);

        crate::config::reset_test_home_dir();
        assert_eq!(readiness_a.phases["sessions"].state, PhaseState::Ready);
        assert_eq!(readiness_b.phases["sessions"].state, PhaseState::Partial);
    }

    #[test]
    fn no_workspace_marks_sessions_not_applicable_but_usage_remains_global() {
        let _guard = test_guard();
        let home = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());

        let readiness = assess(None, None);

        crate::config::reset_test_home_dir();
        assert_eq!(readiness.phases["sessions"].state, PhaseState::Ready);
        assert_eq!(readiness.phases["usage"].state, PhaseState::Partial);
    }

    #[test]
    fn global_skill_scan_does_not_cover_a_newly_selected_workspace() {
        let _guard = test_guard();
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        reconcile_skills(None).unwrap();

        let global = assess_skills(None);
        let scoped = assess_skills(workspace.path().to_str());

        crate::config::reset_test_home_dir();
        assert_eq!(global.state, PhaseState::Ready);
        assert_eq!(scoped.state, PhaseState::Partial);
    }

    #[test]
    fn complete_usage_aggregate_does_not_cover_a_new_session_source() {
        let _guard = test_guard();
        let home = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let conn = local_store::open_database().unwrap();
        conn.execute(
            "INSERT INTO skill_scan_state
             (state_key, state_kind, completeness_status, updated_at_ms)
             VALUES ('skill-sessions:all', 'aggregate', 'complete', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, source_path, file_mtime_ms, file_size_bytes,
              source_cursor, first_seen_at_ms, last_seen_at_ms)
             VALUES ('new-source', 'cursor', '/tmp/new-source', 1, 2, 'cursor-1', 1, 1)",
            [],
        )
        .unwrap();

        let usage = assess_usage(None);

        crate::config::reset_test_home_dir();
        assert_eq!(usage.state, PhaseState::Partial);
        assert!(usage.message.unwrap().contains("1 stale session sources"));
    }

    #[test]
    fn scoped_safe_reconcile_records_successful_zero_session_checkpoints() {
        let _guard = test_guard();
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());

        let workspace = workspace.path().canonicalize().unwrap();
        let report = projection::reconcile_workspace_session_projections_for(
            &workspace,
            ActivityActor::Api,
            &["cursor"],
        )
        .unwrap();
        let latest = HashMap::from([("cursor".to_string(), ActivityStatus::Success)]);
        let readiness = assess_expected_workspace_checkpoints(&["cursor"], &latest);

        crate::config::reset_test_home_dir();
        assert_eq!(report.discovered_sessions, 0);
        assert_eq!(report.scanned_providers, 1);
        assert_eq!(report.unsupported_providers, 0);
        assert_eq!(readiness.state, PhaseState::Ready);
    }

    #[test]
    fn relevant_unsupported_provider_checkpoint_degrades_readiness() {
        let latest = HashMap::from([("claude".to_string(), ActivityStatus::Failed)]);

        let readiness = assess_expected_workspace_checkpoints(&["claude"], &latest);

        assert_eq!(readiness.state, PhaseState::Degraded);
    }

    #[test]
    fn irrelevant_unsupported_provider_checkpoint_does_not_block_readiness() {
        let latest = HashMap::from([
            ("cursor".to_string(), ActivityStatus::Success),
            ("claude".to_string(), ActivityStatus::Failed),
        ]);

        let readiness = assess_expected_workspace_checkpoints(&["cursor"], &latest);

        assert_eq!(readiness.state, PhaseState::Ready);
    }

    #[test]
    fn safe_provider_checkpoint_is_complete() {
        let latest = HashMap::from([("cursor".to_string(), ActivityStatus::Success)]);

        let readiness = assess_expected_workspace_checkpoints(&["cursor"], &latest);

        assert_eq!(readiness.state, PhaseState::Ready);
    }
}
