//! Correlate runtime hook sessions with memorph provider sessions.
//!
//! Hook runtime state is transient. Correlation links it back to provider-native
//! sessions so Web/Desktop/TUI can treat hooks as an enhancement layer instead
//! of a separate session system.

use anyhow::Result;

use crate::hooks::model::{HookEvent, RuntimeSessionCorrelation};
use crate::provider::ProviderSessionSummary;

pub fn correlate_event(event: &HookEvent) -> Option<RuntimeSessionCorrelation> {
    match try_correlate_event(event) {
        Ok(correlation) => correlation,
        Err(error) => {
            let _ = crate::hooks::store::append_error("correlate_event", error.to_string());
            None
        }
    }
}

fn try_correlate_event(event: &HookEvent) -> Result<Option<RuntimeSessionCorrelation>> {
    let Some(provider) = crate::providers::find_provider(&event.provider) else {
        return Ok(None);
    };
    let capabilities = provider.capabilities();
    if !capabilities.scan {
        return Ok(None);
    }

    if let Some(session_id) = event.provider_session_id.as_deref() {
        if let Some(meta) = provider.get_session_meta(session_id)? {
            return Ok(Some(correlation_from_meta(
                &event.provider,
                meta,
                "provider_session_id",
            )));
        }
    }

    let Some(cwd) = event
        .cwd
        .as_deref()
        .map(|path| path.to_string_lossy().to_string())
    else {
        return Ok(None);
    };
    let mut candidates: Vec<ProviderSessionSummary> = provider
        .scan_sessions()?
        .into_iter()
        .filter(|session| provider.workspace_matches(session.project_dir.as_deref(), Some(&cwd)))
        .collect();
    candidates.sort_by_key(|session| std::cmp::Reverse(session.last_active_at.unwrap_or(0)));
    Ok(candidates
        .into_iter()
        .next()
        .map(|meta| correlation_from_meta(&event.provider, meta, "workspace")))
}

fn correlation_from_meta(
    provider: &str,
    meta: ProviderSessionSummary,
    matched_by: &str,
) -> RuntimeSessionCorrelation {
    RuntimeSessionCorrelation {
        provider: provider.to_string(),
        session_id: meta.session_id,
        title: meta.title,
        project_dir: meta.project_dir,
        source_path: meta.source_path,
        matched_by: Some(matched_by.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{HookEvent, HookEventType};
    use serde_json::Value;
    use std::path::PathBuf;

    #[test]
    fn unknown_provider_has_no_correlation() {
        let mut event = HookEvent::new("unknown-provider", HookEventType::Heartbeat, Value::Null);
        event.provider_session_id = Some("s1".to_string());
        assert!(correlate_event(&event).is_none());
    }

    #[test]
    fn event_without_provider_session_or_cwd_has_no_correlation() {
        let event = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        assert!(try_correlate_event(&event).unwrap().is_none());
    }

    #[test]
    fn event_with_missing_workspace_does_not_fail() {
        let mut event = HookEvent::new("claude", HookEventType::Heartbeat, Value::Null);
        event.cwd = Some(PathBuf::from("/path/that/does/not/exist"));
        let _ = try_correlate_event(&event);
    }
}
