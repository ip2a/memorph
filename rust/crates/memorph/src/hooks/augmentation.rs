use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::hooks::model::{
    HookHealthStatus, HookInstallStatus, RuntimeSession, RuntimeSessionStatus,
};
use crate::storage::snapshot_store::ProjectedSessionSnapshotRow;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookLinkConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookRuntimeSummary {
    pub linked_sessions: usize,
    pub waiting_sessions: usize,
    pub status: RuntimeSessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_tool_name: Option<String>,
    #[serde(default)]
    pub has_pending_permission: bool,
    #[serde(default)]
    pub has_pending_question: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<HookLinkConfidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionHookDiagnosisKind {
    Linked,
    WeaklyLinked,
    HookUnsupported,
    HookNotInstalled,
    HookNeedsAttention,
    NoEventsYet,
    NoActiveRuntime,
    NoSessionMatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionHookActionHint {
    pub setting_id: String,
    pub label: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionHookDiagnosis {
    pub kind: SessionHookDiagnosisKind,
    pub provider_status: HookHealthStatus,
    pub linked_runtime_sessions: usize,
    pub provider_runtime_sessions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<HookLinkConfidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<SessionHookActionHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionHookAugmentation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_sessions: Vec<RuntimeSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_summary: Option<HookRuntimeSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<SessionHookDiagnosis>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProviderHookDiagnosisAggregate {
    pub total_sessions: usize,
    pub linked: usize,
    pub weakly_linked: usize,
    pub hook_not_installed: usize,
    pub hook_needs_attention: usize,
    pub no_events_yet: usize,
    pub no_active_runtime: usize,
    pub no_session_match: usize,
    pub hook_unsupported: usize,
    pub active_runtime_sessions: usize,
    pub sessions_with_runtime: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommended_actions: Vec<SessionHookActionHint>,
}

pub fn augment_session(
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> SessionHookAugmentation {
    let snapshot = crate::hooks::server::runtime_sessions_snapshot();
    build_augmentation(
        &snapshot,
        Some(safe_hook_status(provider)),
        provider,
        session_id,
        workspace_dir,
        true,
    )
}

pub fn augment_session_from_snapshot(
    snapshot: &[RuntimeSession],
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> SessionHookAugmentation {
    build_augmentation(snapshot, None, provider, session_id, workspace_dir, false)
}

pub fn augment_session_from_snapshot_with_status(
    snapshot: &[RuntimeSession],
    hook_status: HookInstallStatus,
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> SessionHookAugmentation {
    build_augmentation(
        snapshot,
        Some(hook_status),
        provider,
        session_id,
        workspace_dir,
        true,
    )
}

pub fn aggregate_provider_snapshots(
    snapshot: &[RuntimeSession],
    hook_status: HookInstallStatus,
    provider: &str,
    sessions: &[ProjectedSessionSnapshotRow],
) -> ProviderHookDiagnosisAggregate {
    let active_runtime_sessions = snapshot
        .iter()
        .filter(|session| {
            session.provider == provider
                && !matches!(
                    session.status,
                    RuntimeSessionStatus::Completed | RuntimeSessionStatus::Failed
                )
        })
        .count();
    let provider_sessions: Vec<_> = sessions
        .iter()
        .filter(|session| session.provider_id == provider)
        .collect();
    let mut aggregate = ProviderHookDiagnosisAggregate {
        total_sessions: provider_sessions.len(),
        active_runtime_sessions,
        ..ProviderHookDiagnosisAggregate::default()
    };

    for session in &provider_sessions {
        let session_id = session
            .provider_session_id
            .as_deref()
            .unwrap_or(&session.canonical_session_id);
        let augmentation = augment_session_from_snapshot_with_status(
            snapshot,
            hook_status.clone(),
            provider,
            session_id,
            session.workspace_dir.as_deref(),
        );
        if let Some(summary) = augmentation.runtime_summary.as_ref() {
            aggregate.sessions_with_runtime += summary.linked_sessions;
        }
        if let Some(diagnosis) = augmentation.diagnosis.as_ref() {
            aggregate.record(diagnosis);
        }
    }

    if aggregate.recommended_actions.is_empty() && provider_sessions.is_empty() {
        aggregate.recommended_actions = default_provider_actions(&hook_status.status);
    }

    aggregate
}

pub fn summarize_runtime_sessions(
    runtime_sessions: &[RuntimeSession],
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> Option<HookRuntimeSummary> {
    let primary = runtime_sessions.first()?;
    let waiting_sessions = runtime_sessions
        .iter()
        .filter(|session| {
            matches!(
                session.status,
                RuntimeSessionStatus::WaitingPermission | RuntimeSessionStatus::WaitingUser
            )
        })
        .count();
    let evidence = runtime_sessions
        .iter()
        .find_map(|session| match_evidence(session, provider, session_id, workspace_dir));

    Some(HookRuntimeSummary {
        linked_sessions: runtime_sessions.len(),
        waiting_sessions,
        status: primary.status.clone(),
        current_tool_name: runtime_sessions
            .iter()
            .find_map(|session| session.current_tool.as_ref().map(|tool| tool.name.clone())),
        has_pending_permission: runtime_sessions
            .iter()
            .any(|session| session.pending_permission.is_some()),
        has_pending_question: runtime_sessions
            .iter()
            .any(|session| session.pending_question.is_some()),
        last_event_at: runtime_sessions
            .iter()
            .map(|session| session.last_event_at)
            .max(),
        matched_by: evidence.as_ref().map(|(matched_by, _)| matched_by.clone()),
        confidence: evidence.map(|(_, confidence)| confidence),
    })
}

fn match_evidence(
    session: &RuntimeSession,
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> Option<(String, HookLinkConfidence)> {
    let provider_session_id = session
        .provider_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if provider_session_id == Some(session_id)
        || provider_session_id == Some(&format!("{provider}-{session_id}"))
        || provider_session_id
            .map(|value| value.ends_with(&format!("-{session_id}")))
            .unwrap_or(false)
    {
        return Some(("provider_session_id".to_string(), HookLinkConfidence::High));
    }

    if let Some(correlation) = session.correlation.as_ref() {
        if correlation.session_id == session_id {
            let matched_by = correlation
                .matched_by
                .clone()
                .unwrap_or_else(|| "correlation".to_string());
            let confidence = if matched_by == "provider_session_id" {
                HookLinkConfidence::High
            } else {
                HookLinkConfidence::Medium
            };
            return Some((format!("correlation:{matched_by}"), confidence));
        }
    }

    let workspace_dir = workspace_dir.filter(|value| !value.trim().is_empty())?;
    let session_workspace = session
        .correlation
        .as_ref()
        .and_then(|correlation| correlation.project_dir.as_deref())
        .or_else(|| session.cwd.as_deref().and_then(|cwd| cwd.to_str()));

    if crate::core::session_management::workspace_matches(
        provider,
        session_workspace,
        Some(workspace_dir),
    ) {
        return Some(("workspace_fallback".to_string(), HookLinkConfidence::Low));
    }

    None
}

fn build_augmentation(
    snapshot: &[RuntimeSession],
    hook_status: Option<HookInstallStatus>,
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
    include_diagnosis: bool,
) -> SessionHookAugmentation {
    let runtime_sessions = crate::hooks::server::linked_runtime_sessions_from_snapshot(
        snapshot,
        provider,
        session_id,
        workspace_dir,
    );
    let runtime_summary =
        summarize_runtime_sessions(&runtime_sessions, provider, session_id, workspace_dir);
    let diagnosis = include_diagnosis.then(|| {
        diagnose_session_hook(
            provider,
            session_id,
            &hook_status.unwrap_or_else(|| safe_hook_status(provider)),
            snapshot,
            &runtime_sessions,
            runtime_summary.as_ref(),
        )
    });

    SessionHookAugmentation {
        runtime_sessions,
        runtime_summary,
        diagnosis,
    }
}

fn diagnose_session_hook(
    provider: &str,
    session_id: &str,
    hook_status: &HookInstallStatus,
    snapshot: &[RuntimeSession],
    linked_runtime_sessions: &[RuntimeSession],
    runtime_summary: Option<&HookRuntimeSummary>,
) -> SessionHookDiagnosis {
    let provider_runtime_sessions = snapshot
        .iter()
        .filter(|session| {
            session.provider == provider
                && !matches!(
                    session.status,
                    RuntimeSessionStatus::Completed | RuntimeSessionStatus::Failed
                )
        })
        .count();

    if let Some(summary) = runtime_summary {
        let kind = if summary.confidence == Some(HookLinkConfidence::Low) {
            SessionHookDiagnosisKind::WeaklyLinked
        } else {
            SessionHookDiagnosisKind::Linked
        };
        let message = match summary.confidence {
            Some(HookLinkConfidence::High) => {
                "Hook runtime is linked directly to this session.".to_string()
            }
            Some(HookLinkConfidence::Medium) => {
                "Hook runtime is linked through provider correlation metadata.".to_string()
            }
            Some(HookLinkConfidence::Low) => {
                "Hook runtime is linked by workspace fallback; verify the session identity."
                    .to_string()
            }
            None => "Hook runtime is linked to this session.".to_string(),
        };
        let actions = diagnosis_actions(&kind);
        return SessionHookDiagnosis {
            kind,
            provider_status: hook_status.status.clone(),
            linked_runtime_sessions: linked_runtime_sessions.len(),
            provider_runtime_sessions,
            matched_by: summary.matched_by.clone(),
            confidence: summary.confidence.clone(),
            last_event_at: summary.last_event_at.or(hook_status.last_event_at),
            message,
            actions,
        };
    }

    let (kind, message) = match hook_status.status {
        HookHealthStatus::Unsupported => (
            SessionHookDiagnosisKind::HookUnsupported,
            hook_status
                .message
                .clone()
                .unwrap_or_else(|| format!("Hook integration is not supported for {provider}.")),
        ),
        HookHealthStatus::NotInstalled => (
            SessionHookDiagnosisKind::HookNotInstalled,
            hook_status.message.clone().unwrap_or_else(|| {
                format!("Install {provider} hooks to enable runtime session linking.")
            }),
        ),
        HookHealthStatus::InstalledDisabled
        | HookHealthStatus::InstalledStaleBinary
        | HookHealthStatus::InstalledStaleEndpoint
        | HookHealthStatus::InstalledBrokenConfig
        | HookHealthStatus::InstalledConflict
        | HookHealthStatus::Repairable
        | HookHealthStatus::NeedsUserAction => (
            SessionHookDiagnosisKind::HookNeedsAttention,
            hook_status.message.clone().unwrap_or_else(|| {
                format!("Hook integration for {provider} needs attention before session linking can work.")
            }),
        ),
        HookHealthStatus::InstalledOk => {
            if provider_runtime_sessions > 0 {
                (
                    SessionHookDiagnosisKind::NoSessionMatch,
                    format!(
                        "Observed {} active hook runtime session(s) for {}, but none matched session {}.",
                        provider_runtime_sessions, provider, session_id
                    ),
                )
            } else if hook_status.last_event_at.is_some() {
                (
                    SessionHookDiagnosisKind::NoActiveRuntime,
                    format!(
                        "Hooks are installed for {}, but there is no active runtime session right now.",
                        provider
                    ),
                )
            } else {
                (
                    SessionHookDiagnosisKind::NoEventsYet,
                    format!(
                        "Hooks are installed for {}, but memorph has not observed any hook events yet.",
                        provider
                    ),
                )
            }
        }
    };
    let actions = diagnosis_actions(&kind);

    SessionHookDiagnosis {
        kind,
        provider_status: hook_status.status.clone(),
        linked_runtime_sessions: linked_runtime_sessions.len(),
        provider_runtime_sessions,
        matched_by: None,
        confidence: None,
        last_event_at: hook_status.last_event_at,
        message,
        actions,
    }
}

fn diagnosis_actions(kind: &SessionHookDiagnosisKind) -> Vec<SessionHookActionHint> {
    match kind {
        SessionHookDiagnosisKind::Linked => Vec::new(),
        SessionHookDiagnosisKind::WeaklyLinked => vec![
            action_hint(
                "verify_hook",
                "Verify memorph hook",
                "Confirm hook linkage and provider session correlation.",
            ),
            action_hint(
                "repair_hook",
                "Repair memorph hook",
                "Refresh hook config when linkage falls back to workspace matching.",
            ),
        ],
        SessionHookDiagnosisKind::HookUnsupported => Vec::new(),
        SessionHookDiagnosisKind::HookNotInstalled => vec![action_hint(
            "install_hook",
            "Install memorph hook",
            "Install provider hook integration before expecting runtime session linkage.",
        )],
        SessionHookDiagnosisKind::HookNeedsAttention => vec![
            action_hint(
                "repair_hook",
                "Repair memorph hook",
                "Repair missing, stale, or broken hook configuration.",
            ),
            action_hint(
                "verify_hook",
                "Verify memorph hook",
                "Re-check hook health after repair.",
            ),
        ],
        SessionHookDiagnosisKind::NoEventsYet => vec![action_hint(
            "verify_hook",
            "Verify memorph hook",
            "Confirm the hook is installed and ready before expecting first events.",
        )],
        SessionHookDiagnosisKind::NoActiveRuntime => vec![action_hint(
            "verify_hook",
            "Verify memorph hook",
            "Confirm hook health when no active runtime session is visible.",
        )],
        SessionHookDiagnosisKind::NoSessionMatch => vec![
            action_hint(
                "verify_hook",
                "Verify memorph hook",
                "Check whether provider events carry the expected session identity.",
            ),
            action_hint(
                "repair_hook",
                "Repair memorph hook",
                "Repair hook config if provider runtime sessions are visible but never match this session.",
            ),
        ],
    }
}

fn action_hint(setting_id: &str, label: &str, reason: &str) -> SessionHookActionHint {
    SessionHookActionHint {
        setting_id: setting_id.to_string(),
        label: label.to_string(),
        reason: reason.to_string(),
    }
}

fn default_provider_actions(status: &HookHealthStatus) -> Vec<SessionHookActionHint> {
    match status {
        HookHealthStatus::NotInstalled => {
            diagnosis_actions(&SessionHookDiagnosisKind::HookNotInstalled)
        }
        HookHealthStatus::InstalledDisabled
        | HookHealthStatus::InstalledStaleBinary
        | HookHealthStatus::InstalledStaleEndpoint
        | HookHealthStatus::InstalledBrokenConfig
        | HookHealthStatus::InstalledConflict
        | HookHealthStatus::Repairable
        | HookHealthStatus::NeedsUserAction => {
            diagnosis_actions(&SessionHookDiagnosisKind::HookNeedsAttention)
        }
        _ => Vec::new(),
    }
}

impl ProviderHookDiagnosisAggregate {
    fn record(&mut self, diagnosis: &SessionHookDiagnosis) {
        match diagnosis.kind {
            SessionHookDiagnosisKind::Linked => self.linked += 1,
            SessionHookDiagnosisKind::WeaklyLinked => self.weakly_linked += 1,
            SessionHookDiagnosisKind::HookNotInstalled => self.hook_not_installed += 1,
            SessionHookDiagnosisKind::HookNeedsAttention => self.hook_needs_attention += 1,
            SessionHookDiagnosisKind::NoEventsYet => self.no_events_yet += 1,
            SessionHookDiagnosisKind::NoActiveRuntime => self.no_active_runtime += 1,
            SessionHookDiagnosisKind::NoSessionMatch => self.no_session_match += 1,
            SessionHookDiagnosisKind::HookUnsupported => self.hook_unsupported += 1,
        }

        for action in &diagnosis.actions {
            if self
                .recommended_actions
                .iter()
                .all(|current| current.setting_id != action.setting_id)
            {
                self.recommended_actions.push(action.clone());
            }
        }
    }
}

fn safe_hook_status(provider: &str) -> HookInstallStatus {
    match crate::hooks::operations::status(provider) {
        Ok(status) => status,
        Err(error) => HookInstallStatus {
            provider: provider.to_string(),
            status: HookHealthStatus::InstalledBrokenConfig,
            config_path: None,
            installed_version: None,
            current_version: None,
            message: Some(format!("Failed to inspect hook status: {error}")),
            last_event_at: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{RuntimeSessionCorrelation, RuntimeSessionId};
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn runtime_session(id: &str) -> RuntimeSession {
        let now = Utc::now();
        RuntimeSession {
            runtime_id: RuntimeSessionId::new(id),
            provider: "sample".to_string(),
            provider_session_id: None,
            run_id: None,
            cwd: None,
            pid: None,
            parent_pid: None,
            pid_start_time: None,
            tty: None,
            terminal_vars: BTreeMap::new(),
            process_ancestry: Vec::new(),
            correlation: None,
            model: None,
            session_title: None,
            transcript_path: None,
            workspace_roots: Vec::new(),
            last_user_prompt: None,
            last_assistant_message: None,
            last_tool_result: None,
            last_error: None,
            stop_reason: None,
            compact_count: 0,
            tool_call_count: 0,
            failed_tool_count: 0,
            permission_request_count: 0,
            question_count: 0,
            status: RuntimeSessionStatus::Running,
            current_tool: None,
            pending_permission: None,
            pending_question: None,
            recent_activity: Vec::new(),
            subagents: BTreeMap::new(),
            last_event_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn summarize_runtime_sessions_marks_provider_session_id_high_confidence() {
        let mut session = runtime_session("rt-1");
        session.provider_session_id = Some("session-1".to_string());

        let summary =
            summarize_runtime_sessions(&[session], "sample", "session-1", Some("/tmp/project"))
                .unwrap();

        assert_eq!(summary.matched_by.as_deref(), Some("provider_session_id"));
        assert_eq!(summary.confidence, Some(HookLinkConfidence::High));
    }

    #[test]
    fn summarize_runtime_sessions_marks_correlation_workspace_medium_confidence() {
        let mut session = runtime_session("rt-2");
        session.correlation = Some(RuntimeSessionCorrelation {
            provider: "sample".to_string(),
            session_id: "session-1".to_string(),
            title: None,
            project_dir: Some("/tmp/project".to_string()),
            source_path: None,
            matched_by: Some("workspace".to_string()),
        });

        let summary =
            summarize_runtime_sessions(&[session], "sample", "session-1", Some("/tmp/project"))
                .unwrap();

        assert_eq!(summary.matched_by.as_deref(), Some("correlation:workspace"));
        assert_eq!(summary.confidence, Some(HookLinkConfidence::Medium));
    }

    #[test]
    fn summarize_runtime_sessions_marks_workspace_fallback_low_confidence() {
        let mut session = runtime_session("rt-3");
        session.cwd = Some(PathBuf::from("/tmp/project"));

        let summary =
            summarize_runtime_sessions(&[session], "sample", "missing", Some("/tmp/project"))
                .unwrap();

        assert_eq!(summary.matched_by.as_deref(), Some("workspace_fallback"));
        assert_eq!(summary.confidence, Some(HookLinkConfidence::Low));
    }

    #[test]
    fn diagnose_session_hook_marks_not_installed() {
        let diagnosis = diagnose_session_hook(
            "sample",
            "session-1",
            &hook_status(HookHealthStatus::NotInstalled, None),
            &[],
            &[],
            None,
        );

        assert_eq!(diagnosis.kind, SessionHookDiagnosisKind::HookNotInstalled);
        assert_eq!(diagnosis.actions.len(), 1);
        assert_eq!(diagnosis.actions[0].setting_id, "install_hook");
    }

    #[test]
    fn diagnose_session_hook_marks_no_events_yet() {
        let diagnosis = diagnose_session_hook(
            "sample",
            "session-1",
            &hook_status(HookHealthStatus::InstalledOk, None),
            &[],
            &[],
            None,
        );

        assert_eq!(diagnosis.kind, SessionHookDiagnosisKind::NoEventsYet);
    }

    #[test]
    fn diagnose_session_hook_marks_no_session_match_when_provider_runtime_exists() {
        let provider_runtime = runtime_session("rt-4");
        let diagnosis = diagnose_session_hook(
            "sample",
            "session-1",
            &hook_status(HookHealthStatus::InstalledOk, None),
            &[provider_runtime],
            &[],
            None,
        );

        assert_eq!(diagnosis.kind, SessionHookDiagnosisKind::NoSessionMatch);
        assert_eq!(diagnosis.provider_runtime_sessions, 1);
        assert_eq!(diagnosis.actions.len(), 2);
        assert_eq!(diagnosis.actions[0].setting_id, "verify_hook");
        assert_eq!(diagnosis.actions[1].setting_id, "repair_hook");
    }

    #[test]
    fn aggregate_provider_snapshots_counts_linked_and_no_match() {
        let mut linked = runtime_session("rt-5");
        linked.provider_session_id = Some("session-1".to_string());
        let sessions = vec![
            projected_snapshot("session-1", "/tmp/project"),
            projected_snapshot("session-2", "/tmp/project-2"),
        ];

        let aggregate = aggregate_provider_snapshots(
            &[linked],
            hook_status(HookHealthStatus::InstalledOk, None),
            "sample",
            &sessions,
        );

        assert_eq!(aggregate.total_sessions, 2);
        assert_eq!(aggregate.linked, 1);
        assert_eq!(aggregate.no_session_match, 1);
        assert_eq!(aggregate.active_runtime_sessions, 1);
        assert!(aggregate
            .recommended_actions
            .iter()
            .any(|action| action.setting_id == "verify_hook"));
    }

    fn projected_snapshot(
        provider_session_id: &str,
        workspace_dir: &str,
    ) -> ProjectedSessionSnapshotRow {
        ProjectedSessionSnapshotRow {
            canonical_session_id: format!("canonical-{provider_session_id}"),
            provider_id: "sample".to_string(),
            provider_session_id: Some(provider_session_id.to_string()),
            title: None,
            display_title: None,
            workspace_dir: Some(workspace_dir.to_string()),
            last_active_at_ms: None,
            source_path: None,
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

    fn hook_status(
        status: HookHealthStatus,
        last_event_at: Option<DateTime<Utc>>,
    ) -> HookInstallStatus {
        HookInstallStatus {
            provider: "sample".to_string(),
            status,
            config_path: None,
            installed_version: None,
            current_version: None,
            message: None,
            last_event_at,
        }
    }
}
