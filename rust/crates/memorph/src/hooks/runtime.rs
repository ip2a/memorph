//! In-memory runtime session state derived from ingested hook events.

#[cfg(test)]
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::hooks::identity::runtime_session_id_for_event;
use crate::hooks::model::{
    HookEvent, HookEventType, RuntimeSession, RuntimeSessionId, RuntimeSessionStatus,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RuntimeState {
    #[serde(default)]
    pub sessions: BTreeMap<RuntimeSessionId, RuntimeSession>,
}

impl RuntimeState {
    /// Apply one canonical hook event to the process-local runtime snapshot.
    pub fn apply_event(&mut self, event: &HookEvent) {
        let runtime_id = runtime_session_id_for_event(event);
        let session = self
            .sessions
            .entry(runtime_id.clone())
            .or_insert_with(|| RuntimeSession {
                runtime_id,
                provider: event.provider.clone(),
                provider_session_id: event.provider_session_id.clone(),
                cwd: event.cwd.clone(),
                status: status_for_event(&event.event_type),
                started_at: event.timestamp,
                last_event_at: event.timestamp,
            });

        session.provider = event.provider.clone();
        if event.provider_session_id.is_some() {
            session.provider_session_id = event.provider_session_id.clone();
        }
        if event.cwd.is_some() {
            session.cwd = event.cwd.clone();
        }
        session.status = status_for_event(&event.event_type);
        session.last_event_at = event.timestamp;
    }

    #[cfg(test)]
    fn session(&self, id: &str) -> &RuntimeSession {
        self.sessions
            .get(&RuntimeSessionId::new(id))
            .expect("runtime session exists")
    }
}

fn status_for_event(event_type: &HookEventType) -> RuntimeSessionStatus {
    match event_type {
        HookEventType::PermissionRequested => RuntimeSessionStatus::WaitingPermission,
        HookEventType::QuestionRequested => RuntimeSessionStatus::WaitingUser,
        HookEventType::SessionCompleted => RuntimeSessionStatus::Completed,
        HookEventType::SessionFailed => RuntimeSessionStatus::Failed,
        _ => RuntimeSessionStatus::Running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(event_type: HookEventType) -> HookEvent {
        HookEvent {
            event_id: "event-1".to_string(),
            provider: "generic".to_string(),
            event_type,
            provider_session_id: Some("session-1".to_string()),
            run_id: None,
            timestamp: Utc::now(),
            cwd: Some("/tmp/project".into()),
            pid: None,
            parent_pid: None,
            pid_start_time: None,
            tty: None,
            terminal_vars: Default::default(),
            process_ancestry: Vec::new(),
            tool: None,
            message: None,
            permission: None,
            question: None,
            raw: json!({}),
        }
    }

    #[test]
    fn tracks_identity_environment_and_status_only() {
        let mut state = RuntimeState::default();
        state.apply_event(&event(HookEventType::SessionStarted));
        let id = runtime_session_id_for_event(&event(HookEventType::SessionStarted));
        let session = state.session(&id.0);
        assert_eq!(session.provider, "generic");
        assert_eq!(session.provider_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            session.cwd.as_deref().and_then(|path| path.to_str()),
            Some("/tmp/project")
        );
        assert_eq!(session.status, RuntimeSessionStatus::Running);

        state.apply_event(&event(HookEventType::PermissionRequested));
        assert_eq!(
            state.session(&id.0).status,
            RuntimeSessionStatus::WaitingPermission
        );
        state.apply_event(&event(HookEventType::SessionCompleted));
        assert_eq!(state.session(&id.0).status, RuntimeSessionStatus::Completed);
    }
}
