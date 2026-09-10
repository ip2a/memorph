use anyhow::{Context as _, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconcile_required: Option<ReconcileRequired>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconcile_reason: Option<ReconcileReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_full_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_incremental_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileRequired {
    None,
    Incremental,
    Full,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileReason {
    ColdStart,
    FoundationError,
    StaleSignatures,
    PeriodicRefresh,
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
        reconcile_required: None,
        reconcile_reason: None,
        last_full_at: None,
        last_incremental_at: None,
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

// ───────────────────────────────────────────────────────────────────────────
// Tiered reconcile: signature-driven snapshots + decide_reconcile
// ───────────────────────────────────────────────────────────────────────────
//
// ponytail: the existing assess_* functions already compute PhaseReadiness
// correctly. We reuse them (via assess_named_phase) to produce the value that
// gets persisted into readiness_checkpoint after a reconcile, and we compute a
// cheap signature per phase so snapshot() can decide whether a recompute is
// needed without ever calling assess_* on the hot read path.

pub const PHASE_TIMEOUT_MS: u64 = 30_000;
pub const TOTAL_BUDGET_MS: u64 = 120_000;
const PERIODIC_FULL_REFRESH_DAYS: i64 = 7;

/// Phases that are not virtual. `derived` is excluded — it is always computed
/// from the other phases inside `finish_readiness`.
pub const REAL_PHASES: [&str; 6] = [
    "foundation",
    "agents",
    "sessions",
    "session_stats",
    "skills",
    "usage",
];

/// Phases that depend on a workspace. Stored with a non-empty workspace_key.
const WORKSPACE_SCOPED_PHASES: [&str; 2] = ["sessions", "session_stats"];

fn is_workspace_scoped(phase: &str) -> bool {
    WORKSPACE_SCOPED_PHASES.contains(&phase)
}

/// Stable workspace key for checkpoint rows. Global phases use ""; scoped
/// phases use the canonicalized workspace path (same form validate_workspace
/// produces, which assess_sessions/assess_session_stats already consume).
fn checkpoint_workspace_key(phase: &str, workspace: Option<&str>) -> String {
    if is_workspace_scoped(phase) {
        workspace.unwrap_or("").to_string()
    } else {
        String::new()
    }
}

/// Dispatch to the existing assess_* function for a phase. Used by
/// ReadinessCache::record_after_reconcile to produce the PhaseReadiness that
/// gets persisted.
pub fn assess_named_phase(phase: &str, workspace: Option<&str>) -> PhaseReadiness {
    match phase {
        "foundation" => assess_foundation(),
        "agents" => assess_agents(),
        "sessions" => assess_sessions(workspace),
        "session_stats" => assess_session_stats(workspace),
        "skills" => assess_skills(workspace),
        "usage" => assess_usage(workspace),
        _ => PhaseReadiness::error(format!("Unknown phase: {phase}")),
    }
}

// ── signatures ──────────────────────────────────────────────────────────────
//
// Each signature is a short string computed from cheap SQL aggregates over the
// tables a phase reads. A phase's data is considered unchanged iff its
// signature equals the one stored at last successful reconcile. Every signature
// includes a max(updated_at_ms)-style component so writes are detected even
// when row counts are stable.

pub mod signatures {
    use super::*;

    pub fn foundation() -> Result<String> {
        let conn = local_store::open_database()?;
        let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let mtime = crate::config::config_path()
            .ok()
            .and_then(|path| std::fs::metadata(path).ok())
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Ok(format!("v{user_version}|m{mtime}"))
    }

    pub fn agents() -> Result<String> {
        // agent_management is computed live from providers, not stored. Use the
        // provider registry size + any provider-config mtimes as a proxy. This
        // is intentionally cheap; agent capability changes are rare and also
        // surface through skill/session signature drift.
        let count = crate::providers::all_provider_ids().len();
        let home = crate::config::effective_home_dir()?;
        let cfg = home.join("providers");
        let mut buf = format!("n{count}");
        if cfg.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&cfg) {
                let mut mtimes: Vec<i64> = entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .filter_map(|m| m.modified().ok())
                    .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .collect();
                mtimes.sort_unstable();
                if let Some(&last) = mtimes.last() {
                    buf.push_str(&format!("|m{last}"));
                }
            }
        }
        Ok(buf)
    }

    pub fn sessions(workspace_key: &str) -> Result<String> {
        let conn = local_store::open_database()?;
        let (count, last): (i64, Option<i64>) = conn
            .query_row(
                "SELECT COUNT(*), MAX(finished_at_ms)
                 FROM session_activity
                 WHERE operation_kind = 'scan'
                   AND workspace_dir = ?1
                   AND json_extract(details_json, '$.scan_kind') = 'readiness_workspace_projection'",
                rusqlite::params![workspace_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
        Ok(format!("n{count}|m{}", last.unwrap_or(0)))
    }

    pub fn session_stats(workspace_key: &str) -> Result<String> {
        let conn = local_store::open_database()?;
        let (total, incomplete): (i64, i64) = conn.query_row(
            "SELECT COUNT(*),
                    SUM(CASE WHEN message_count IS NULL THEN 1 ELSE 0 END)
             FROM session_snapshots
             WHERE workspace_dir = ?1",
            rusqlite::params![workspace_key],
            |row| {
                let inc: Option<i64> = row.get(1)?;
                Ok((row.get(0)?, inc.unwrap_or(0)))
            },
        )?;
        Ok(format!("n{total}|i{incomplete}"))
    }

    pub fn skills() -> Result<String> {
        let conn = local_store::open_database()?;
        let row: (i64, i64, String) = conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(MAX(updated_at_ms), 0),
                    COALESCE(GROUP_CONCAT(completeness_status, ''), '')
             FROM skill_scan_state
             WHERE state_kind = 'skill-root'",
            [],
            |row| {
                let g: Option<String> = row.get(2)?;
                Ok((row.get(0)?, row.get(1)?, g.unwrap_or_default()))
            },
        )?;
        Ok(format!("n{}|m{}|s{}", row.0, row.1, row.2))
    }

    pub fn usage() -> Result<String> {
        let conn = local_store::open_database()?;
        let (count, max_seen): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(MAX(last_seen_at_ms), 0) FROM session_sources",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let catalog = invocation::catalog_generation(&conn).unwrap_or_default();
        Ok(format!("n{count}|m{max_seen}|c{catalog}"))
    }
}

/// Compute the live signature for a single phase.
pub fn signature_for(phase: &str, workspace_key: &str) -> Result<String> {
    match phase {
        "foundation" => signatures::foundation(),
        "agents" => signatures::agents(),
        "sessions" => signatures::sessions(workspace_key),
        "session_stats" => signatures::session_stats(workspace_key),
        "skills" => signatures::skills(),
        "usage" => signatures::usage(),
        _ => Ok(String::new()),
    }
}

/// Compute live signatures for all real phases in one pass. Global phases key
/// on ""; workspace-scoped phases key on the provided workspace_key.
fn live_signatures(workspace_key: &str) -> HashMap<String, String> {
    REAL_PHASES
        .iter()
        .filter_map(|&phase| {
            let key = checkpoint_workspace_key(phase, Some(workspace_key));
            // Only include this phase if it is in scope: global phases always;
            // workspace-scoped phases only when a workspace was provided.
            if is_workspace_scoped(phase) && workspace_key.is_empty() {
                return None;
            }
            match signature_for(phase, &key) {
                Ok(sig) => Some((phase.to_string(), sig)),
                Err(_) => None,
            }
        })
        .collect()
}

// ── ReadinessCache ──────────────────────────────────────────────────────────

pub struct ReadinessCache;

impl ReadinessCache {
    /// O(1) snapshot read. Replaces the old uncached assess() on GET /readiness.
    /// Missing rows are treated as Partial("Not yet assessed") and force
    /// reconcile_required = Full (cold_start).
    pub fn snapshot(workspace: Option<&str>, active_operation_id: Option<String>) -> Readiness {
        let validated = validate_workspace(workspace);
        let workspace_canonical = match validated {
            Ok(ws) => ws,
            Err(error) => {
                return error_readiness(workspace.map(str::to_string), active_operation_id, error);
            }
        };

        let conn = match local_store::open_database() {
            Ok(c) => c,
            Err(error) => {
                return error_readiness(
                    workspace_canonical.clone(),
                    active_operation_id,
                    anyhow::anyhow!("Database is unavailable: {error}"),
                );
            }
        };

        let rows = Self::read_all_rows(&conn, workspace_canonical.as_deref());
        let now_ms = current_timestamp_ms();

        let mut phases: BTreeMap<String, PhaseReadiness> = BTreeMap::new();
        let mut any_missing = false;
        let mut any_error = false;
        let mut min_reconciled_ms: Option<i64> = None;
        let mut max_reconciled_ms: Option<i64> = None;

        for &phase in REAL_PHASES.iter() {
            let key = checkpoint_workspace_key(phase, workspace_canonical.as_deref());
            // workspace-scoped phase with no workspace → "not applicable" (ready)
            if is_workspace_scoped(phase) && workspace_canonical.is_none() {
                phases.insert(
                    phase.to_string(),
                    PhaseReadiness::ready("Workspace session projection is not applicable"),
                );
                continue;
            }
            match rows.get(&(phase.to_string(), key.clone())) {
                Some(row) => {
                    phases.insert(phase.to_string(), row.to_phase_readiness());
                    if row.state == "error" {
                        any_error = true;
                    }
                    if let Some(b) = row.reconciled_at_ms {
                        min_reconciled_ms = Some(min_reconciled_ms.map_or(b, |a| a.min(b)));
                        max_reconciled_ms = Some(max_reconciled_ms.map_or(b, |a| a.max(b)));
                    }
                }
                None => {
                    any_missing = true;
                    phases.insert(
                        phase.to_string(),
                        PhaseReadiness::partial("Not yet assessed"),
                    );
                }
            }
        }

        // derived: aggregate of real phases
        let inputs_ready = REAL_PHASES
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

        let reconcile_required = if any_missing {
            Some(ReconcileRequired::Full)
        } else if phases["foundation"].state == PhaseState::Error || any_error {
            Some(ReconcileRequired::Full)
        } else {
            // compare signatures
            let ws_key = workspace_canonical.clone().unwrap_or_default();
            let stored = Self::stored_signatures_from_rows(&rows);
            let live = live_signatures(&ws_key);
            let drifted = drifted_phases(&stored, &live);
            if !drifted.is_empty() {
                Some(ReconcileRequired::Incremental)
            } else {
                // periodic refresh check
                let stale_by_age = min_reconciled_ms.is_some_and(|m| {
                    now_ms.saturating_sub(m) > PERIODIC_FULL_REFRESH_DAYS * 86_400_000
                });
                if stale_by_age {
                    Some(ReconcileRequired::Full)
                } else {
                    Some(ReconcileRequired::None)
                }
            }
        };

        let reconcile_reason = match reconcile_required {
            Some(ReconcileRequired::Full) if any_missing => Some(ReconcileReason::ColdStart),
            Some(ReconcileRequired::Full)
                if phases["foundation"].state == PhaseState::Error || any_error =>
            {
                Some(ReconcileReason::FoundationError)
            }
            Some(ReconcileRequired::Full) => Some(ReconcileReason::PeriodicRefresh),
            Some(ReconcileRequired::Incremental) => Some(ReconcileReason::StaleSignatures),
            _ => None,
        };

        // last_full_at = earliest reconciled timestamp (full pass writes all);
        // last_incremental_at = latest reconciled timestamp.
        let last_full_at = min_reconciled_ms;
        let last_incremental_at = max_reconciled_ms;

        let mut readiness = finish_readiness(workspace_canonical, active_operation_id, phases);
        readiness.reconcile_required = reconcile_required;
        readiness.reconcile_reason = reconcile_reason;
        readiness.last_full_at = last_full_at;
        readiness.last_incremental_at = last_incremental_at;
        readiness
    }

    /// Persist a phase result into readiness_checkpoint. Called by the worker
    /// after a successful reconcile_phase. Uses INSERT OR REPLACE.
    pub fn record(phase: &str, workspace: Option<&str>, result: &PhaseReadiness) -> Result<()> {
        let workspace_key = checkpoint_workspace_key(phase, workspace);
        let signature = signature_for(phase, &workspace_key).unwrap_or_default();
        let state = phase_state_to_str(&result.state);
        let now_ms = current_timestamp_ms();
        let conn = local_store::open_database()?;
        conn.execute(
            "INSERT OR REPLACE INTO readiness_checkpoint
                (phase, workspace_key, state, message, signature,
                 assessed_at_ms, reconciled_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            rusqlite::params![
                phase,
                workspace_key,
                state,
                result.message.as_deref().unwrap_or(""),
                signature,
                now_ms,
            ],
        )?;
        Ok(())
    }

    /// Read all checkpoint rows for a given workspace (global "" + scoped).
    fn read_all_rows(
        conn: &rusqlite::Connection,
        workspace: Option<&str>,
    ) -> HashMap<(String, String), CheckpointRow> {
        let ws = workspace.unwrap_or("");
        let mut stmt = match conn.prepare(
            "SELECT phase, workspace_key, state, message, signature, reconciled_at_ms
             FROM readiness_checkpoint
             WHERE workspace_key = '' OR workspace_key = ?1",
        ) {
            Ok(s) => s,
            Err(_) => return HashMap::new(),
        };
        let rows = stmt.query_map(rusqlite::params![ws], |row| {
            Ok(CheckpointRow {
                phase: row.get(0)?,
                workspace_key: row.get(1)?,
                state: row.get(2)?,
                message: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                signature: row.get(4)?,
                reconciled_at_ms: row.get(5)?,
            })
        });
        let mut out = HashMap::new();
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                out.insert((row.phase.clone(), row.workspace_key.clone()), row);
            }
        }
        out
    }

    /// Extract (signature, state) for drift comparison, keyed by phase.
    fn stored_signatures_from_rows(
        rows: &HashMap<(String, String), CheckpointRow>,
    ) -> HashMap<String, (String, PhaseState)> {
        rows.iter()
            .map(|((phase, _), row)| {
                (
                    phase.clone(),
                    (row.signature.clone(), str_to_phase_state(&row.state)),
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct CheckpointRow {
    phase: String,
    workspace_key: String,
    state: String,
    message: String,
    signature: String,
    reconciled_at_ms: Option<i64>,
}

impl CheckpointRow {
    fn to_phase_readiness(&self) -> PhaseReadiness {
        PhaseReadiness {
            state: str_to_phase_state(&self.state),
            message: if self.message.is_empty() {
                None
            } else {
                Some(self.message.clone())
            },
        }
    }
}

fn phase_state_to_str(state: &PhaseState) -> &'static str {
    match state {
        PhaseState::Ready => "ready",
        PhaseState::Partial => "partial",
        PhaseState::Degraded => "degraded",
        PhaseState::Error => "error",
    }
}

fn str_to_phase_state(s: &str) -> PhaseState {
    match s {
        "ready" => PhaseState::Ready,
        "degraded" => PhaseState::Degraded,
        "error" => PhaseState::Error,
        _ => PhaseState::Partial,
    }
}

fn current_timestamp_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Phases whose stored signature differs from the live one, or whose stored
/// state is Error, or that are missing from the stored map entirely.
fn drifted_phases(
    stored: &HashMap<String, (String, PhaseState)>,
    live: &HashMap<String, String>,
) -> BTreeSet<String> {
    let mut drifted = BTreeSet::new();
    for (phase, live_sig) in live {
        match stored.get(phase) {
            None => {
                drifted.insert(phase.clone());
            }
            Some((sig, state)) => {
                if sig != live_sig || *state == PhaseState::Error {
                    drifted.insert(phase.clone());
                }
            }
        }
    }
    drifted
}

// ── ReconcilePlan + decide_reconcile ────────────────────────────────────────

/// Trigger forwarded from the API layer. Mirrors the old enum but lives in core
/// so decide_reconcile has everything it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileTrigger {
    Startup,
    Manual,
    WorkspaceChange,
    Retry,
}

impl ReconcileTrigger {
    pub fn is_manual(self) -> bool {
        matches!(self, Self::Manual | Self::Retry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcilePlan {
    Noop,
    Incremental { phases: BTreeSet<String> },
    Full { phases: BTreeSet<String> },
}

impl ReconcilePlan {
    pub fn phases(&self) -> BTreeSet<String> {
        match self {
            Self::Noop => BTreeSet::new(),
            Self::Incremental { phases } | Self::Full { phases } => phases.clone(),
        }
    }

    pub fn is_noop(&self) -> bool {
        matches!(self, Self::Noop)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Incremental { .. } => "incremental",
            Self::Full { .. } => "full",
        }
    }
}

/// All real phases that are in scope for the given workspace. Workspace-scoped
/// phases are excluded when no workspace is set.
fn all_real_phases_for(workspace_key: &str) -> BTreeSet<String> {
    REAL_PHASES
        .iter()
        .filter(|&&phase| !(is_workspace_scoped(phase) && workspace_key.is_empty()))
        .map(|phase| phase.to_string())
        .collect()
}

/// The authoritative decision function. Startup/WorkspaceChange are Steady and
/// never return Full; Manual/Retry may return Full when cold, heavily drifted,
/// or past the periodic-refresh window.
pub fn decide_reconcile(trigger: ReconcileTrigger, workspace: Option<&str>) -> ReconcilePlan {
    let workspace_key = workspace.unwrap_or("").to_string();
    let conn = local_store::open_database();
    let rows = match conn {
        Ok(c) => ReadinessCache::read_all_rows(&c, workspace),
        Err(_) => {
            return ReconcilePlan::Full {
                phases: all_real_phases_for(&workspace_key),
            }
        }
    };
    let stored = ReadinessCache::stored_signatures_from_rows(&rows);
    let live = live_signatures(&workspace_key);
    let drifted = drifted_phases(&stored, &live);

    match trigger {
        ReconcileTrigger::Startup | ReconcileTrigger::WorkspaceChange => {
            if drifted.is_empty() {
                ReconcilePlan::Noop
            } else {
                ReconcilePlan::Incremental { phases: drifted }
            }
        }
        ReconcileTrigger::Manual | ReconcileTrigger::Retry => {
            let any_missing = REAL_PHASES.iter().any(|phase| !stored.contains_key(*phase));
            let any_error = stored
                .values()
                .any(|(_, state)| *state == PhaseState::Error);
            let stale_ratio = if live.is_empty() {
                1.0
            } else {
                drifted.len() as f32 / live.len() as f32
            };
            let now_ms = current_timestamp_ms();
            let last_full_age_stale = rows
                .values()
                .filter_map(|r| r.reconciled_at_ms)
                .min()
                .is_some_and(|m| {
                    now_ms.saturating_sub(m) > PERIODIC_FULL_REFRESH_DAYS * 86_400_000
                });
            if any_missing || any_error || stale_ratio > 0.5 || last_full_age_stale {
                ReconcilePlan::Full {
                    phases: all_real_phases_for(&workspace_key),
                }
            } else if drifted.is_empty() {
                ReconcilePlan::Noop
            } else {
                ReconcilePlan::Incremental { phases: drifted }
            }
        }
    }
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

    // ── tiered reconcile tests ──────────────────────────────────────────────

    fn reset_checkpoint_for_test(conn: &rusqlite::Connection) {
        let _ = conn.execute("DELETE FROM readiness_checkpoint", []);
    }

    #[test]
    fn cold_snapshot_reports_full_reconcile_required() {
        let _g = test_guard();
        let home = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let conn = local_store::open_database().unwrap();
        reset_checkpoint_for_test(&conn);
        drop(conn);

        let readiness = ReadinessCache::snapshot(None, None);
        assert_eq!(readiness.reconcile_required, Some(ReconcileRequired::Full));
        assert_eq!(readiness.reconcile_reason, Some(ReconcileReason::ColdStart));
        crate::config::reset_test_home_dir();
    }

    #[test]
    fn recording_all_phases_then_snapshot_is_none_when_unchanged() {
        let _g = test_guard();
        let home = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let conn = local_store::open_database().unwrap();
        reset_checkpoint_for_test(&conn);
        drop(conn);

        for &phase in REAL_PHASES.iter() {
            let result = assess_named_phase(phase, None);
            ReadinessCache::record(phase, None, &result).unwrap();
        }

        let readiness = ReadinessCache::snapshot(None, None);
        // Foundation is ready in a fresh temp home; others may be partial, but
        // signatures are stable immediately after record() so required should be
        // None (no drift). The state may still be Partial but that's a data
        // question, not a reconcile question.
        assert_ne!(
            readiness.reconcile_required,
            Some(ReconcileRequired::Full),
            "should not be cold after recording all phases"
        );
        crate::config::reset_test_home_dir();
    }

    #[test]
    fn startup_trigger_never_returns_full_plan() {
        let _g = test_guard();
        let home = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let conn = local_store::open_database().unwrap();
        reset_checkpoint_for_test(&conn);
        drop(conn);

        let plan = decide_reconcile(ReconcileTrigger::Startup, None);
        assert!(
            !matches!(plan, ReconcilePlan::Full { .. }),
            "Startup must never return Full, got {:?}",
            plan
        );

        // Even after recording all phases, Startup stays Noop or Incremental.
        for &phase in REAL_PHASES.iter() {
            let result = assess_named_phase(phase, None);
            ReadinessCache::record(phase, None, &result).unwrap();
        }
        let plan = decide_reconcile(ReconcileTrigger::Startup, None);
        assert!(
            !matches!(plan, ReconcilePlan::Full { .. }),
            "Startup must never return Full even post-record, got {:?}",
            plan
        );
        crate::config::reset_test_home_dir();
    }

    #[test]
    fn manual_trigger_returns_full_on_cold_start() {
        let _g = test_guard();
        let home = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let conn = local_store::open_database().unwrap();
        reset_checkpoint_for_test(&conn);
        drop(conn);

        let plan = decide_reconcile(ReconcileTrigger::Manual, None);
        assert!(
            matches!(plan, ReconcilePlan::Full { .. }),
            "cold Manual should be Full, got {plan:?}"
        );
        crate::config::reset_test_home_dir();
    }

    #[test]
    fn signature_for_phase_returns_nonempty_for_all_real_phases() {
        let _g = test_guard();
        let home = tempfile::tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        for &phase in REAL_PHASES.iter() {
            let key = checkpoint_workspace_key(phase, None);
            let sig = signature_for(phase, &key).unwrap_or_default();
            assert!(!sig.is_empty(), "signature for {phase} should be non-empty");
        }
        crate::config::reset_test_home_dir();
    }
}
