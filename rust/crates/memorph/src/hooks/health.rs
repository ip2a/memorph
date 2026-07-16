//! Hook verification and repair.

use crate::hooks::model::HookInstallStatus;
use anyhow::Result;

pub fn status(provider: &str) -> Result<HookInstallStatus> {
    crate::hooks::operations::status(provider)
}

pub fn verify(provider: &str) -> Result<HookInstallStatus> {
    status(provider)
}

pub(crate) fn summarize_versions(versions: impl Iterator<Item = Option<String>>) -> Option<String> {
    let mut normalized: Vec<String> = versions
        .map(|version| version.unwrap_or_else(|| "legacy".to_string()))
        .collect();
    normalized.sort();
    normalized.dedup();
    match normalized.len() {
        0 => None,
        1 => normalized.pop(),
        _ => Some("mixed".to_string()),
    }
}

pub(crate) fn last_event_at(provider: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    last_event_at_in(provider, None)
}

pub(crate) fn last_event_at_in(
    provider: &str,
    cached: Option<&std::collections::HashMap<String, i64>>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let max_ms = if let Some(map) = cached {
        map.get(provider).copied()
    } else {
        crate::hooks::store::last_event_observed_at_ms_for_providers(&[provider.to_string()])
            .ok()
            .and_then(|map| map.get(provider).copied())
    }?;
    chrono::DateTime::from_timestamp_millis(max_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::HookHealthStatus;

    #[test]
    fn unsupported_provider_reports_unsupported() {
        let status = status("unknown-provider").unwrap();
        assert_eq!(status.status, HookHealthStatus::Unsupported);
    }

    #[test]
    fn summarizes_legacy_and_current_hook_versions() {
        assert_eq!(
            summarize_versions(vec![None].into_iter()).as_deref(),
            Some("legacy")
        );
        assert_eq!(
            summarize_versions(
                vec![Some(
                    crate::hooks::shared::current_hook_managed_version().to_string()
                )]
                .into_iter()
            )
            .as_deref(),
            Some(crate::hooks::shared::current_hook_managed_version())
        );
        assert_eq!(
            summarize_versions(
                vec![
                    None,
                    Some(crate::hooks::shared::current_hook_managed_version().to_string())
                ]
                .into_iter()
            )
            .as_deref(),
            Some("mixed")
        );
    }
}
