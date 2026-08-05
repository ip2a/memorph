use super::*;
use memorph::core::readiness::{self as core_readiness, PhaseState, Readiness, PHASES};
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReconcileFocus {
    Overview,
    Agents,
    Sessions,
    Skills,
    Usage,
    Derived,
}

impl Default for ReconcileFocus {
    fn default() -> Self {
        Self::Overview
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReconcilePriority {
    Background,
    Foreground,
}

impl Default for ReconcilePriority {
    fn default() -> Self {
        Self::Foreground
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReconcileTrigger {
    Startup,
    Manual,
    WorkspaceChange,
    IncompletePanel,
    Retry,
}

impl Default for ReconcileTrigger {
    fn default() -> Self {
        Self::Manual
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ReadinessQuery {
    workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReconcileRequest {
    workspace: Option<String>,
    #[serde(default)]
    focus: ReconcileFocus,
    #[serde(default)]
    priority: ReconcilePriority,
    #[serde(default)]
    trigger: ReconcileTrigger,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OperationStatus {
    Queued,
    Running,
    Completed,
    Failed,
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
    priority: ReconcilePriority,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_phase: Option<String>,
    completed_phases: Vec<String>,
    running_phases: Vec<String>,
    pending_phases: Vec<String>,
    failures: Vec<OperationFailure>,
    #[serde(skip)]
    workspace: Option<String>,
    #[serde(skip)]
    required_phases: BTreeSet<String>,
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
    let readiness = core_readiness::assess(workspace.as_deref(), active_operation_id);
    ApiResponse::success(readiness).into_response()
}

pub(super) async fn reconcile_readiness(
    Json(request): Json<ReconcileRequest>,
) -> impl IntoResponse {
    let workspace = match core_readiness::validate_workspace(request.workspace.as_deref()) {
        Ok(workspace) => workspace,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, error).into_response(),
    };
    let required_phases = phases_for_focus(request.focus);
    let mut initial = core_readiness::assess(workspace.as_deref(), None);

    let mut state = coordinator()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(operation) = state
        .operations
        .values_mut()
        .find(|operation| operation.workspace == workspace && operation.status.active())
    {
        if request.priority == ReconcilePriority::Foreground {
            operation.priority = ReconcilePriority::Foreground;
        }
        let requested_pending = pending_phases(
            &required_phases,
            &initial,
            request.trigger,
            workspace.as_deref(),
        );
        operation.required_phases.extend(required_phases);
        for phase in requested_pending {
            if !operation.completed_phases.contains(&phase)
                && !operation.running_phases.contains(&phase)
                && !operation.pending_phases.contains(&phase)
            {
                operation.pending_phases.push(phase);
            }
        }
        return ApiResponse::success(response(operation, ReconcileDisposition::Joined))
            .into_response();
    }

    let pending = pending_phases(
        &required_phases,
        &initial,
        request.trigger,
        workspace.as_deref(),
    );
    if pending.is_empty() {
        let operation_id = Uuid::new_v4().to_string();
        let operation = ReadinessOperation {
            operation_id: operation_id.clone(),
            status: OperationStatus::Completed,
            readiness: initial,
            trigger: request.trigger,
            priority: request.priority,
            current_phase: None,
            completed_phases: Vec::new(),
            running_phases: Vec::new(),
            pending_phases: Vec::new(),
            failures: Vec::new(),
            workspace,
            required_phases,
        };
        let payload = response(&operation, ReconcileDisposition::Noop);
        state.operations.insert(operation_id, operation);
        prune_completed_history(&mut state, Some(&payload.operation_id));
        return ApiResponse::success(payload).into_response();
    }

    let operation_id = Uuid::new_v4().to_string();
    initial.active_operation_id = Some(operation_id.clone());
    let operation = ReadinessOperation {
        operation_id: operation_id.clone(),
        status: OperationStatus::Queued,
        readiness: initial,
        trigger: request.trigger,
        priority: request.priority,
        current_phase: None,
        completed_phases: Vec::new(),
        running_phases: Vec::new(),
        pending_phases: pending,
        failures: Vec::new(),
        workspace: workspace.clone(),
        required_phases,
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

fn phases_for_focus(focus: ReconcileFocus) -> BTreeSet<String> {
    let phases: &[&str] = match focus {
        ReconcileFocus::Overview => &PHASES,
        ReconcileFocus::Agents => &["foundation", "agents", "derived"],
        ReconcileFocus::Sessions => &["foundation", "sessions", "session_stats", "derived"],
        ReconcileFocus::Skills => &["foundation", "skills", "derived"],
        ReconcileFocus::Usage => &[
            "foundation",
            "sessions",
            "session_stats",
            "skills",
            "usage",
            "derived",
        ],
        ReconcileFocus::Derived => &PHASES,
    };
    phases.iter().map(|phase| (*phase).to_string()).collect()
}

fn pending_phases(
    required_phases: &BTreeSet<String>,
    readiness: &Readiness,
    trigger: ReconcileTrigger,
    workspace: Option<&str>,
) -> Vec<String> {
    let repeat_real_phases = matches!(
        trigger,
        ReconcileTrigger::Manual | ReconcileTrigger::Retry | ReconcileTrigger::WorkspaceChange
    );
    required_phases
        .iter()
        .filter(|phase| phase.as_str() != "derived")
        .filter(|phase| phase.as_str() != "sessions" || workspace.is_some())
        .filter(|phase| {
            repeat_real_phases || readiness.phases[phase.as_str()].state != PhaseState::Ready
        })
        .cloned()
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
    let workspace = {
        let mut state = coordinator()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(operation) = state.operations.get_mut(operation_id) else {
            return;
        };
        operation.status = OperationStatus::Running;
        operation.workspace.clone()
    };

    let mut foundation_failed = false;
    loop {
        let phase = {
            let mut state = coordinator()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let operation = state
                .operations
                .get_mut(operation_id)
                .expect("operation exists");
            begin_next_phase_or_close(operation, foundation_failed)
        };
        let Some(phase) = phase else {
            break;
        };
        if phase == "usage" {
            let mut state = coordinator()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let operation = state
                .operations
                .get_mut(operation_id)
                .expect("operation exists");
            let inputs_failed = operation
                .failures
                .iter()
                .any(|failure| matches!(failure.phase.as_str(), "sessions" | "skills"));
            if inputs_failed {
                operation.running_phases.clear();
                operation.failures.push(OperationFailure {
                    phase,
                    message: "skipped: session or skill inputs failed".into(),
                });
                continue;
            }
        }
        let result = core_readiness::reconcile_phase(&phase, workspace.as_deref());
        let mut state = coordinator()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let operation = state
            .operations
            .get_mut(operation_id)
            .expect("operation exists");
        operation.running_phases.clear();
        match result {
            Ok(()) => operation.completed_phases.push(phase),
            Err(error) => {
                foundation_failed = phase == "foundation";
                operation.failures.push(OperationFailure {
                    phase,
                    message: format!("{error:#}"),
                });
            }
        }
        operation.readiness =
            core_readiness::assess(workspace.as_deref(), Some(operation_id.to_string()));
        if foundation_failed {
            operation.status = OperationStatus::Failed;
            operation.current_phase = None;
            operation.pending_phases.clear();
            break;
        }
    }

    let mut state = coordinator()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let operation = state
        .operations
        .get_mut(operation_id)
        .expect("operation exists");
    operation.readiness = core_readiness::assess(workspace.as_deref(), None);
    if !operation.failures.is_empty() && !foundation_failed {
        for failure in &operation.failures {
            if let Some(phase) = operation.readiness.phases.get_mut(&failure.phase) {
                phase.state = PhaseState::Degraded;
                phase.message = Some(failure.message.clone());
            }
        }
        operation.readiness.state = PhaseState::Degraded;
    }
    prune_completed_history(&mut state, Some(operation_id));
}

fn begin_next_phase_or_close(
    operation: &mut ReadinessOperation,
    foundation_failed: bool,
) -> Option<String> {
    let phase = ordered_pending(operation).into_iter().next();
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

fn ordered_pending(operation: &ReadinessOperation) -> Vec<String> {
    PHASES
        .iter()
        .filter(|phase| **phase != "derived")
        .filter(|phase| {
            operation
                .pending_phases
                .iter()
                .any(|pending| pending == **phase)
        })
        .map(|phase| (*phase).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_focus_expands_to_real_baseline_dependencies() {
        let phases = phases_for_focus(ReconcileFocus::Derived);
        for phase in [
            "foundation",
            "agents",
            "sessions",
            "session_stats",
            "skills",
            "usage",
            "derived",
        ] {
            assert!(phases.contains(phase));
        }
    }

    #[test]
    fn ordered_pending_never_queues_virtual_derived_phase() {
        let operation = ReadinessOperation {
            operation_id: "test".into(),
            status: OperationStatus::Queued,
            readiness: core_readiness::assess(None, None),
            trigger: ReconcileTrigger::Manual,
            priority: ReconcilePriority::Foreground,
            current_phase: None,
            completed_phases: Vec::new(),
            running_phases: Vec::new(),
            pending_phases: vec!["sessions".into(), "derived".into()],
            failures: Vec::new(),
            workspace: None,
            required_phases: BTreeSet::new(),
        };
        assert_eq!(ordered_pending(&operation), vec!["sessions"]);
    }

    #[test]
    fn manual_trigger_repeats_ready_real_phases_but_startup_is_gap_only() {
        let home = tempfile::tempdir().unwrap();
        memorph::config::set_test_home_dir(home.path().to_path_buf());
        let readiness = core_readiness::assess(None, None);
        let required = [
            "foundation".to_string(),
            "agents".to_string(),
            "derived".to_string(),
        ]
        .into_iter()
        .collect();

        let manual = pending_phases(&required, &readiness, ReconcileTrigger::Manual, None);
        let startup = pending_phases(&required, &readiness, ReconcileTrigger::Startup, None);

        memorph::config::reset_test_home_dir();
        assert_eq!(manual, vec!["agents", "foundation"]);
        assert!(startup.is_empty());
    }

    #[test]
    fn no_pending_phase_closes_operation_before_joiners_can_observe_it() {
        let mut operation = ReadinessOperation {
            operation_id: "closing".into(),
            status: OperationStatus::Running,
            readiness: core_readiness::assess(None, None),
            trigger: ReconcileTrigger::Manual,
            priority: ReconcilePriority::Foreground,
            current_phase: None,
            completed_phases: vec!["foundation".into()],
            running_phases: Vec::new(),
            pending_phases: Vec::new(),
            failures: Vec::new(),
            workspace: None,
            required_phases: BTreeSet::new(),
        };

        assert_eq!(begin_next_phase_or_close(&mut operation, false), None);
        assert_eq!(operation.status, OperationStatus::Completed);
        assert!(!operation.status.active());
    }

    #[test]
    fn safe_skill_phase_completes_in_one_operation() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        memorph::config::set_test_home_dir(home.path().to_path_buf());
        let workspace = workspace.path().canonicalize().unwrap();
        let workspace_text = workspace.to_string_lossy().into_owned();
        let operation_id = Uuid::new_v4().to_string();
        let operation = ReadinessOperation {
            operation_id: operation_id.clone(),
            status: OperationStatus::Queued,
            readiness: core_readiness::assess(Some(&workspace_text), None),
            trigger: ReconcileTrigger::Manual,
            priority: ReconcilePriority::Foreground,
            current_phase: None,
            completed_phases: Vec::new(),
            running_phases: Vec::new(),
            pending_phases: vec!["skills".into()],
            failures: Vec::new(),
            workspace: Some(workspace_text),
            required_phases: ["skills".to_string()].into_iter().collect(),
        };
        coordinator()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .operations
            .insert(operation_id.clone(), operation);

        run_worker(&operation_id);

        let state = coordinator()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let operation = &state.operations[&operation_id];
        assert_eq!(operation.status, OperationStatus::Completed);
        assert!(operation.failures.is_empty());
        assert!(operation
            .completed_phases
            .iter()
            .any(|phase| phase == "skills"));
        drop(state);
        memorph::config::reset_test_home_dir();
    }
}
