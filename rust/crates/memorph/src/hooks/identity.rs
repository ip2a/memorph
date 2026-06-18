//! Runtime session identity and correlation.
//!
//! Runtime identity must be stronger than workspace path alone. The resolver
//! prefers provider-native identifiers, then run identifiers, then process and
//! terminal fingerprints.

use std::path::Path;

use crate::hooks::model::{HookEvent, RuntimeSessionId, SessionFingerprint};

pub fn fingerprint_for_event(event: &HookEvent) -> SessionFingerprint {
    event.fingerprint()
}

pub fn runtime_session_id_for_event(event: &HookEvent) -> RuntimeSessionId {
    if let Some(session_id) = event
        .provider_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return RuntimeSessionId::new(format!(
            "{}:session:{}",
            normalize_component(&event.provider),
            normalize_component(session_id)
        ));
    }

    if let Some(run_id) = event
        .run_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return RuntimeSessionId::new(format!(
            "{}:run:{}",
            normalize_component(&event.provider),
            normalize_component(run_id)
        ));
    }

    let fingerprint = stable_fingerprint(event);
    RuntimeSessionId::new(format!(
        "{}:fp:{}",
        normalize_component(&event.provider),
        fingerprint
    ))
}

fn stable_fingerprint(event: &HookEvent) -> String {
    let mut parts = Vec::new();
    parts.push(format!("provider={}", event.provider));
    if let Some(cwd) = event.cwd.as_deref() {
        parts.push(format!("cwd={}", normalize_path(cwd)));
    }
    if let Some(pid) = event.pid {
        parts.push(format!("pid={pid}"));
    }
    if let Some(parent_pid) = event.parent_pid {
        parts.push(format!("ppid={parent_pid}"));
    }
    if let Some(tty) = event.tty.as_deref() {
        parts.push(format!("tty={tty}"));
    }
    for key in TERMINAL_IDENTITY_KEYS {
        if let Some(value) = event
            .terminal_vars
            .get(*key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("{key}={value}"));
        }
    }
    if parts.len() == 1 {
        parts.push(format!("event={}", event.event_id));
    }
    format!("{:x}", md5::compute(parts.join("|")))
}

const TERMINAL_IDENTITY_KEYS: &[&str] = &[
    "ITERM_SESSION_ID",
    "TERM_SESSION_ID",
    "TMUX_PANE",
    "KITTY_WINDOW_ID",
    "ZELLIJ_PANE_ID",
    "ZELLIJ_SESSION_NAME",
    "WEZTERM_PANE",
    "CMUX_SURFACE_ID",
    "CMUX_WORKSPACE_ID",
    "WARP_SESSION_ID",
];

fn normalize_component(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{HookEvent, HookEventType};
    use serde_json::Value;
    use std::path::PathBuf;

    #[test]
    fn prefers_provider_session_id() {
        let mut event = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        event.provider_session_id = Some("abc/123".to_string());
        let id = runtime_session_id_for_event(&event);
        assert_eq!(id.0, "claude:session:abc_123");
    }

    #[test]
    fn falls_back_to_run_id() {
        let mut event = HookEvent::new("codex", HookEventType::Heartbeat, Value::Null);
        event.run_id = Some("run-1".to_string());
        let id = runtime_session_id_for_event(&event);
        assert_eq!(id.0, "codex:run:run-1");
    }

    #[test]
    fn uses_stable_process_fingerprint_without_session_ids() {
        let mut first = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
        first.cwd = Some(PathBuf::from("/tmp/project"));
        first.pid = Some(10);
        first.parent_pid = Some(9);
        first.tty = Some("/dev/ttys001".to_string());

        let mut second = first.clone();
        second.event_id = "different-event".to_string();

        assert_eq!(
            runtime_session_id_for_event(&first),
            runtime_session_id_for_event(&second)
        );
    }

    #[test]
    fn terminal_identity_vars_disambiguate_fallback_sessions() {
        let mut first = HookEvent::new("generic", HookEventType::Heartbeat, Value::Null);
        first.cwd = Some(PathBuf::from("/tmp/project"));
        first
            .terminal_vars
            .insert("TMUX_PANE".to_string(), "%1".to_string());

        let mut second = first.clone();
        second
            .terminal_vars
            .insert("TMUX_PANE".to_string(), "%2".to_string());

        assert_ne!(
            runtime_session_id_for_event(&first),
            runtime_session_id_for_event(&second)
        );
    }
}
