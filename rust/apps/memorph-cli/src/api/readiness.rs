use super::*;
use memorph::core::readiness::{
    self as core_readiness, PhaseState, Readiness, ReconcileTrigger, REAL_PHASES,
};
use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub(super) struct ReadinessQuery {
    workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReconcileRequest {
    workspace: Option<String>,
    #[serde(default = "default_trigger")]
    trigger: ReconcileTrigger,
}

fn default_trigger() -> ReconcileTrigger {
    ReconcileTrigger::Manual
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OperationStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Superseded,
}

impl OperationStatus {
    fn active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Debug, Clone, Serialize)]
struct OperationFailure {
    phase: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ReadinessOperation {
    operation_id: String,
    status: OperationStatus,
    readiness: Readiness,
    trigger: ReconcileTrigger,
    plan: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_phase: Option<String>,
    completed_phases: Vec<String>,
    running_phases: Vec<String>,
    pending_phases: Vec<String>,
    failures: Vec<OperationFailure>,
    #[serde(skip)]
    workspace: Option<String>,
    #[serde(skip)]
    started_at: Option<Instant>,
}

#[derive(Default)]
struct Coordinator {
    operations: HashMap<String, ReadinessOperation>,
}

static COORDINATOR: OnceLock<Mutex<Coordinator>> = OnceLock::new();

fn coordinator() -> &'static Mutex<Coordinator> {
    COORDINATOR.get_or_init(|| Mutex::new(Coordinator::default()))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReconcileDisposition {
    Started,
    Joined,
    Noop,
}

#[derive(Debug, Serialize)]
pub(super) struct ReconcileResponse {
    operation_id: String,
    disposition: ReconcileDisposition,
    status: OperationStatus,
    readiness: Readiness,
    status_url: String,
}

pub(super) async fn get_readiness(Query(query): Query<ReadinessQuery>) -> impl IntoResponse {
    let workspace = match core_readiness::validate_workspace(query.workspace.as_deref()) {
        Ok(workspace) => workspace,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, error).into_response(),
    };
    let active_operation_id = coordinator()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .operations
        .values()
        .find(|operation| operation.workspace == workspace && operation.status.active())
        .map(|operation| operation.operation_id.clone());
    let readiness =
        core_readiness::ReadinessCache::snapshot(workspace.as_deref(), active_operation_id);
    ApiResponse::success(readiness).into_response()
}

pub(super) async fn reconcile_readiness(
    Json(request): Json<ReconcileRequest>,
) -> impl IntoResponse {
    let workspace = match core_readiness::validate_workspace(request.workspace.as_deref()) {
        Ok(workspace) => workspace,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, error).into_response(),
    };

    let plan = core_readiness::decide_reconcile(request.trigger, workspace.as_deref());

    let mut state = coordinator()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Manual/Retry preempts any running Steady operation on the same workspace.
    if request.trigger.is_manual() {
        for operation in state.operations.values_mut() {
            if operation.workspace == workspace
                && operation.status.active()
                && !operation.trigger.is_manual()
            {
                operation.status = OperationStatus::Superseded;
                operation.current_phase = None;
                operation.running_phases.clear();
                operation.pending_phases.clear();
                operation.readiness.active_operation_id.take();
            }
        }
    }

    // Join an already-active operation on the same workspace.
    if let Some(operation) = state
        .operations
        .values()
        .find(|operation| operation.workspace == workspace && operation.status.active())
    {
        let payload = response(operation, ReconcileDisposition::Joined);
        return ApiResponse::success(payload).into_response();
    }

    if plan.is_noop() {
        let operation_id = Uuid::new_v4().to_string();
        let readiness = core_readiness::ReadinessCache::snapshot(workspace.as_deref(), None);
        let operation = ReadinessOperation {
            operation_id: operation_id.clone(),
            status: OperationStatus::Completed,
            readiness,
            trigger: request.trigger,
            plan: plan.label(),
            current_phase: None,
            completed_phases: Vec::new(),
            running_phases: Vec::new(),
            pending_phases: Vec::new(),
            failures: Vec::new(),
            workspace,
            started_at: None,
        };
        let payload = response(&operation, ReconcileDisposition::Noop);
        state.operations.insert(operation_id, operation);
        prune_completed_history(&mut state, None);
        return ApiResponse::success(payload).into_response();
    }

    let pending: Vec<String> = ordered_phases(&plan.phases());
    let operation_id = Uuid::new_v4().to_string();
    let mut initial =
        core_readiness::ReadinessCache::snapshot(workspace.as_deref(), Some(operation_id.clone()));
    initial.active_operation_id = Some(operation_id.clone());
    let operation = ReadinessOperation {
        operation_id: operation_id.clone(),
        status: OperationStatus::Queued,
        readiness: initial,
        trigger: request.trigger,
        plan: plan.label(),
        current_phase: None,
        completed_phases: Vec::new(),
        running_phases: Vec::new(),
        pending_phases: pending,
        failures: Vec::new(),
        workspace: workspace.clone(),
        started_at: Some(Instant::now()),
    };
    state.operations.insert(operation_id.clone(), operation);
    drop(state);

    if let Err(error) = spawn_worker(operation_id.clone()) {
        let mut state = coordinator()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let operation = state
            .operations
            .get_mut(&operation_id)
            .expect("operation exists");
        operation.status = OperationStatus::Failed;
        operation.pending_phases.clear();
        operation.failures.push(OperationFailure {
            phase: "foundation".into(),
            message: format!("Failed to start readiness worker: {error}"),
        });
        operation.readiness.active_operation_id = None;
        if let Some(foundation) = operation.readiness.phases.get_mut("foundation") {
            foundation.state = PhaseState::Error;
            foundation.message = Some(format!("Failed to start readiness worker: {error}"));
        }
        operation.readiness.state = PhaseState::Error;
    }

    let state = coordinator()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let payload = response(
        state
            .operations
            .get(&operation_id)
            .expect("operation exists"),
        ReconcileDisposition::Started,
    );
    ApiResponse::success(payload).into_response()
}

pub(super) async fn get_readiness_operation(Path(operation_id): Path<String>) -> impl IntoResponse {
    let state = coordinator()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match state.operations.get(&operation_id) {
        Some(operation) => ApiResponse::success(operation.clone()).into_response(),
        None => api_error(
            StatusCode::NOT_FOUND,
            format!("Unknown readiness operation: {operation_id}"),
        )
        .into_response(),
    }
}

fn ordered_phases(phases: &BTreeSet<String>) -> Vec<String> {
    REAL_PHASES
        .iter()
        .filter(|phase| phases.contains(**phase))
        .map(|phase| (*phase).to_string())
        .collect()
}

fn prune_completed_history(state: &mut Coordinator, protected_id: Option<&str>) {
    const COMPLETED_HISTORY_LIMIT: usize = 100;
    let mut terminal = state
        .operations
        .iter()
        .filter(|(id, operation)| !operation.status.active() && protected_id != Some(id.as_str()))
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let terminal_count = terminal.len() + usize::from(protected_id.is_some());
    let remove_count = terminal_count.saturating_sub(COMPLETED_HISTORY_LIMIT);
    for id in terminal.drain(..remove_count.min(terminal.len())) {
        state.operations.remove(&id);
    }
}

fn response(
    operation: &ReadinessOperation,
    disposition: ReconcileDisposition,
) -> ReconcileResponse {
    ReconcileResponse {
        operation_id: operation.operation_id.clone(),
        disposition,
        status: operation.status,
        readiness: operation.readiness.clone(),
        status_url: status_url(&operation.operation_id),
    }
}

fn status_url(operation_id: &str) -> String {
    format!("/api/v1/readiness/operations/{operation_id}")
}

fn spawn_worker(operation_id: String) -> std::io::Result<()> {
    let test_home = memorph::config::test_home_dir();
    std::thread::Builder::new()
        .name(format!("memorph-readiness-{}", &operation_id[..8]))
        .spawn(move || {
            if let Some(home) = test_home {
                memorph::config::set_test_home_dir(home);
            }
            run_worker(&operation_id);
            memorph::config::reset_test_home_dir();
        })
        .map(|_| ())
}

fn run_worker(operation_id: &str) {
    let (workspace, deadline) = {
        let mut state = coordinator()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(operation) = state.operations.get_mut(operation_id) else {
            return;
        };
        operation.status = OperationStatus::Running;
        let deadline = operation
            .started_at
            .map(|s| s + Duration::from_millis(core_readiness::TOTAL_BUDGET_MS));
        (operation.workspace.clone(), deadline)
    };

    let mut foundation_failed = false;
    loop {
        // Total budget watchdog.
        if deadline.is_some_and(|d| Instant::now() > d) {
            force_fail(operation_id, "total budget exceeded");
            return;
        }

        // Superseded? Exit quietly.
        let superseded = coordinator()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .operations
            .get(operation_id)
            .map(|op| matches!(op.status, OperationStatus::Superseded))
            .unwrap_or(true);
        if superseded {
            return;
        }

        let phase = {
            let mut state = coordinator()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(operation) = state.operations.get_mut(operation_id) else {
                return;
            };
            begin_next_phase_or_close(operation, foundation_failed)
        };
        let Some(phase) = phase else {
            break;
        };

        // usage depends on sessions/skills — skip if those failed.
        if phase == "usage" {
            let inputs_failed = coordinator()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .operations
                .get(operation_id)
                .map(|op| {
                    op.failures
                        .iter()
                        .any(|f| matches!(f.phase.as_str(), "sessions" | "skills"))
                })
                .unwrap_or(false);
            if inputs_failed {
                let mut state = coordinator()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let operation = state.operations.get_mut(operation_id).expect("exists");
                operation.running_phases.clear();
                operation.failures.push(OperationFailure {
                    phase,
                    message: "skipped: session or skill inputs failed".into(),
                });
                continue;
            }
        }

        let result = run_phase_with_timeout(&phase, workspace.as_deref());

        let mut state = coordinator()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let operation = state.operations.get_mut(operation_id).expect("exists");
        operation.running_phases.clear();
        match result {
            Ok(()) => {
                // Record into checkpoint cache, then refresh snapshot for this
                // operation's readiness view.
                let pr = core_readiness::assess_named_phase(&phase, workspace.as_deref());
                let _ = core_readiness::ReadinessCache::record(&phase, workspace.as_deref(), &pr);
                operation.completed_phases.push(phase.clone());
            }
            Err(error) => {
                foundation_failed = phase == "foundation";
                operation.failures.push(OperationFailure {
                    phase: phase.clone(),
                    message: format!("{error:#}"),
                });
            }
        }
        operation.readiness = core_readiness::ReadinessCache::snapshot(
            workspace.as_deref(),
            Some(operation_id.to_string()),
        );
        if foundation_failed {
            operation.status = OperationStatus::Failed;
            operation.current_phase = None;
            operation.pending_phases.clear();
            operation.readiness.active_operation_id = None;
            break;
        }
    }

    finalize_operation(operation_id);
}

/// Run a single reconcile_phase in a detached thread, waiting up to
/// PHASE_TIMEOUT_MS. On timeout the thread is abandoned (its late writes are
/// made idempotent by SQLite PK upsert semantics in record()).
fn run_phase_with_timeout(phase: &str, workspace: Option<&str>) -> anyhow::Result<()> {
    let phase_label = phase.to_string();
    let (tx, rx) = mpsc::channel::<anyhow::Result<()>>();
    let phase = phase_label.clone();
    let workspace = workspace.map(str::to_string);
    std::thread::Builder::new()
        .name(format!("memorph-readiness-phase-{phase}"))
        .spawn(move || {
            let result = core_readiness::reconcile_phase(&phase, workspace.as_deref());
            let _ = tx.send(result);
        })?;
    match rx.recv_timeout(Duration::from_millis(core_readiness::PHASE_TIMEOUT_MS)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(anyhow::anyhow!(
            "phase {phase_label} timed out after {}ms",
            core_readiness::PHASE_TIMEOUT_MS
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(anyhow::anyhow!("phase {phase_label} worker thread died"))
        }
    }
}

fn begin_next_phase_or_close(
    operation: &mut ReadinessOperation,
    foundation_failed: bool,
) -> Option<String> {
    let phase = REAL_PHASES
        .iter()
        .filter(|p| operation.pending_phases.contains(&(**p).to_string()))
        .map(|p| (*p).to_string())
        .next();
    if let Some(phase) = phase {
        operation.current_phase = Some(phase.clone());
        operation.running_phases = vec![phase.clone()];
        operation.pending_phases.retain(|pending| pending != &phase);
        return Some(phase);
    }
    operation.current_phase = None;
    operation.running_phases.clear();
    operation.pending_phases.clear();
    operation.status = if foundation_failed {
        OperationStatus::Failed
    } else {
        OperationStatus::Completed
    };
    None
}

fn finalize_operation(operation_id: &str) {
    let mut state = coordinator()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(operation) = state.operations.get_mut(operation_id) else {
        return;
    };
    // Non-foundation phase failures surface as Degraded in readiness.state but
    // the operation itself is Completed. Failed is only set on foundation
    // failure (early break in run_worker) or budget timeout (force_fail).
    if !matches!(
        operation.status,
        OperationStatus::Superseded | OperationStatus::Failed
    ) {
        operation.status = OperationStatus::Completed;
    }
    operation.current_phase = None;
    operation.running_phases.clear();
    operation.pending_phases.clear();
    operation.readiness =
        core_readiness::ReadinessCache::snapshot(operation.workspace.as_deref(), None);
    operation.readiness.active_operation_id = None;
    prune_completed_history(&mut state, Some(operation_id));
}

fn force_fail(operation_id: &str, reason: &str) {
    let mut state = coordinator()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(operation) = state.operations.get_mut(operation_id) else {
        return;
    };
    if matches!(operation.status, OperationStatus::Superseded) {
        return;
    }
    let orphaned = std::mem::take(&mut operation.running_phases);
    for phase in orphaned {
        operation.failures.push(OperationFailure {
            phase,
            message: reason.to_string(),
        });
    }
    operation.status = OperationStatus::Failed;
    operation.current_phase = None;
    operation.pending_phases.clear();
    operation.readiness =
        core_readiness::ReadinessCache::snapshot(operation.workspace.as_deref(), None);
    operation.readiness.active_operation_id = None;
    prune_completed_history(&mut state, Some(operation_id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_phases_preserves_real_phase_canonical_order() {
        let mut phases = BTreeSet::new();
        phases.insert("usage".to_string());
        phases.insert("foundation".to_string());
        assert_eq!(
            ordered_phases(&phases),
            vec!["foundation".to_string(), "usage".to_string()]
        );
    }

    #[test]
    fn ordered_phases_filters_unknown_and_derived() {
        let mut phases = BTreeSet::new();
        phases.insert("derived".to_string());
        phases.insert("bogus".to_string());
        phases.insert("agents".to_string());
        assert_eq!(ordered_phases(&phases), vec!["agents".to_string()]);
    }

    #[test]
    fn begin_next_phase_clears_running_when_pending_empty() {
        let home = tempfile::tempdir().unwrap();
        memorph::config::set_test_home_dir(home.path().to_path_buf());
        let mut operation = ReadinessOperation {
            operation_id: "x".into(),
            status: OperationStatus::Running,
            readiness: core_readiness::ReadinessCache::snapshot(None, None),
            trigger: ReconcileTrigger::Manual,
            plan: "incremental",
            current_phase: None,
            completed_phases: vec!["foundation".into()],
            running_phases: vec!["foundation".into()],
            pending_phases: Vec::new(),
            failures: Vec::new(),
            workspace: None,
            started_at: Some(Instant::now()),
        };
        assert_eq!(begin_next_phase_or_close(&mut operation, false), None);
        assert_eq!(operation.status, OperationStatus::Completed);
        assert!(operation.running_phases.is_empty());
        memorph::config::reset_test_home_dir();
    }

    #[test]
    fn manual_preemption_supersedes_running_steady_operation() {
        let home = tempfile::tempdir().unwrap();
        memorph::config::set_test_home_dir(home.path().to_path_buf());
        let mut state = Coordinator::default();
        let mut readiness = core_readiness::ReadinessCache::snapshot(None, None);
        readiness.active_operation_id = Some("op-steady".into());
        state.operations.insert(
            "op-steady".into(),
            ReadinessOperation {
                operation_id: "op-steady".into(),
                status: OperationStatus::Running,
                readiness,
                trigger: ReconcileTrigger::Startup,
                plan: "incremental",
                current_phase: Some("sessions".into()),
                completed_phases: vec!["foundation".into()],
                running_phases: vec!["sessions".into()],
                pending_phases: vec!["sessions".into()],
                failures: Vec::new(),
                workspace: None,
                started_at: Some(Instant::now()),
            },
        );
        // Simulate manual preemption pass.
        for operation in state.operations.values_mut() {
            if operation.status.active() && !operation.trigger.is_manual() {
                operation.status = OperationStatus::Superseded;
                operation.current_phase = None;
                operation.running_phases.clear();
                operation.pending_phases.clear();
            }
        }
        let steady = &state.operations["op-steady"];
        assert_eq!(steady.status, OperationStatus::Superseded);
        assert!(!steady.status.active());
        memorph::config::reset_test_home_dir();
    }

    #[test]
    fn force_fail_marks_failures_and_clears_active_operation_id() {
        let home = tempfile::tempdir().unwrap();
        memorph::config::set_test_home_dir(home.path().to_path_buf());
        // Seed coordinator with a running operation that has an orphaned phase.
        {
            let mut state = coordinator().lock().unwrap_or_else(|p| p.into_inner());
            state.operations.clear();
            let mut readiness = core_readiness::ReadinessCache::snapshot(None, None);
            readiness.active_operation_id = Some("op-budget".into());
            state.operations.insert(
                "op-budget".into(),
                ReadinessOperation {
                    operation_id: "op-budget".into(),
                    status: OperationStatus::Running,
                    readiness,
                    trigger: ReconcileTrigger::Manual,
                    plan: "full",
                    current_phase: Some("skills".into()),
                    completed_phases: vec!["foundation".into()],
                    running_phases: vec!["skills".into()],
                    pending_phases: vec!["skills".into(), "usage".into()],
                    failures: Vec::new(),
                    workspace: None,
                    started_at: Some(Instant::now()),
                },
            );
        }
        force_fail("op-budget", "total budget exceeded");
        let state = coordinator().lock().unwrap_or_else(|p| p.into_inner());
        let op = &state.operations["op-budget"];
        assert_eq!(op.status, OperationStatus::Failed);
        assert!(op.readiness.active_operation_id.is_none());
        assert!(op.failures.iter().any(|f| f.phase == "skills"));
        memorph::config::reset_test_home_dir();
    }

    #[test]
    fn reconcile_request_trigger_defaults_to_manual() {
        let req: ReconcileRequest = serde_json::from_str(r#"{"workspace": null}"#).unwrap();
        assert_eq!(req.trigger, ReconcileTrigger::Manual);
    }
}
