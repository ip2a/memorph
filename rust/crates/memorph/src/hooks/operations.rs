//! Hook operation dispatch.
//!
//! This is the public execution boundary for hook management. It resolves the
//! canonical provider and dispatches only to provider-owned hook implementations.
//! Missing provider hook implementations are reported explicitly instead of
//! falling back to common-layer provider-specific code.

use anyhow::{anyhow, Result};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};
use crate::hooks::strategies::HookConfigOperation;

pub fn status(provider: &str) -> Result<HookInstallStatus> {
    let Some(profile) = crate::hooks::profiles::find(provider) else {
        return Ok(unsupported_status(provider, None));
    };
    let provider = profile.provider;
    if let Some(hook) = find_provider_hook(provider) {
        return hook.status();
    }
    Ok(unsupported_status(
        provider,
        Some("Hook provider is registered but has no provider-owned implementation."),
    ))
}

pub fn run_operation(
    provider: &str,
    operation: HookConfigOperation,
) -> Result<HookOperationReport> {
    let provider = canonical_provider(provider)?;
    let hook = find_provider_hook(provider).ok_or_else(|| {
        anyhow!("Hook provider is registered but has no provider-owned implementation: {provider}")
    })?;
    match operation {
        HookConfigOperation::Install => hook.install(),
        HookConfigOperation::Verify => hook.verify(),
        HookConfigOperation::Repair => hook.repair(),
        HookConfigOperation::Uninstall => hook.uninstall(),
    }
}

pub fn run_setting_operation(provider: &str, setting_id: &str) -> Result<HookOperationReport> {
    let operation = HookConfigOperation::from_setting_id(setting_id)
        .ok_or_else(|| anyhow!("Unknown hook config operation: {setting_id}"))?;
    run_operation(provider, operation)
}

pub fn find_provider_hook(provider: &str) -> Option<&'static dyn ProviderHook> {
    crate::providers::find_provider_hook(provider)
}

fn unsupported_status(provider: &str, message: Option<&str>) -> HookInstallStatus {
    HookInstallStatus {
        provider: provider.to_string(),
        status: HookHealthStatus::Unsupported,
        config_path: None,
        installed_version: None,
        current_version: None,
        message: Some(message.map(str::to_string).unwrap_or_else(|| {
            format!("Hook management is not implemented for provider: {provider}")
        })),
        last_event_at: crate::hooks::health::last_event_at(provider),
    }
}

fn canonical_provider(provider: &str) -> Result<&'static str> {
    crate::hooks::profiles::find(provider)
        .map(|profile| profile.provider)
        .ok_or_else(|| anyhow!("Unsupported hook provider: {provider}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_profiles_have_provider_owned_hooks() {
        for descriptor in crate::hooks::registry::all() {
            assert_eq!(
                find_provider_hook(descriptor.provider())
                    .unwrap_or_else(|| panic!("missing hook for {}", descriptor.provider()))
                    .provider_id(),
                descriptor.provider()
            );
        }
    }

    #[test]
    fn setting_operation_names_are_parsed_at_operation_boundary() {
        assert_eq!(
            HookConfigOperation::from_setting_id("repair_hook"),
            Some(HookConfigOperation::Repair)
        );
        assert_eq!(HookConfigOperation::from_setting_id("approve"), None);
    }

    #[test]
    fn unknown_status_is_reported_without_legacy_fallback() {
        let status = status("unknown-provider").expect("status report");
        assert_eq!(status.provider, "unknown-provider");
        assert_eq!(status.status, HookHealthStatus::Unsupported);
        assert!(status
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("not implemented"));
    }
}
