//! Provider event normalization.
//!
//! Provider adapters translate raw provider payloads into canonical hook events.
//! They must not update runtime state or write storage; those responsibilities
//! belong to `runtime` and `store`.

use anyhow::{Context, Result};

use crate::hooks::adapters::generic::GenericHookAdapter;
use crate::hooks::contract::HookAdapter;
use crate::hooks::model::HookEvent;
use crate::hooks::protocol::HookIngestRequest;

static GENERIC_ADAPTER: GenericHookAdapter = GenericHookAdapter;

pub fn normalize_request(request: &HookIngestRequest) -> Result<Vec<HookEvent>> {
    let mut events = adapter_for(&request.provider)
        .with_context(|| {
            format!(
                "No hook adapter is registered for provider: {}",
                request.provider
            )
        })?
        .normalize(request)?;
    for event in &mut events {
        if event.terminal_vars.is_empty() && !request.environment.vars.is_empty() {
            event.terminal_vars = request.environment.vars.clone();
        }
    }
    Ok(events)
}

pub fn adapter_for(provider: &str) -> Option<&'static dyn HookAdapter> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "generic" | "custom" | "unknown" => Some(&GENERIC_ADAPTER),
        provider => crate::providers::find_hook_adapter(provider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_generic_request() {
        let request = HookIngestRequest::new(
            "generic",
            "tool_started",
            json!({"session_id": "sess-1", "tool": {"name": "shell", "input": "cargo check"}}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider, "generic");
        assert_eq!(events[0].provider_session_id.as_deref(), Some("sess-1"));
        assert_eq!(events[0].tool.as_ref().unwrap().name, "shell");
    }

    #[test]
    fn carries_bridge_terminal_environment_into_normalized_events() {
        let mut request = HookIngestRequest::new(
            "generic",
            "tool_started",
            json!({"session_id": "s1", "tool": {"name": "shell"}}),
        );
        request
            .environment
            .vars
            .insert("TMUX_PANE".to_string(), "%5".to_string());
        request
            .environment
            .vars
            .insert("WEZTERM_PANE".to_string(), "12".to_string());

        let events = normalize_request(&request).unwrap();
        assert_eq!(
            events[0].terminal_vars.get("TMUX_PANE").map(String::as_str),
            Some("%5")
        );
        assert_eq!(
            events[0]
                .terminal_vars
                .get("WEZTERM_PANE")
                .map(String::as_str),
            Some("12")
        );
    }

    #[test]
    fn rejects_unregistered_provider_explicitly() {
        let request = HookIngestRequest::new("missing-provider", "tool_started", json!({}));
        let err = normalize_request(&request).unwrap_err().to_string();
        assert!(err.contains("No hook adapter"));
    }
}
