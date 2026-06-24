//! Backward-compatible hook installation facade.
//!
//! Provider-specific hook implementation lives in `providers/<provider>/hook.rs`.
//! This module keeps older public entry points and test helpers while delegating
//! runtime operations to `hooks::operations`.

use anyhow::Result;
#[cfg(test)]
const HOOK_COMMAND_MARKER: &str = crate::hooks::shared::HOOK_COMMAND_MARKER;

pub use crate::hooks::model::HookOperationReport;

pub fn supports_provider(provider: &str) -> bool {
    implemented_provider_id(provider).is_some()
}

pub fn required_events(provider: &str) -> Option<&'static [&'static str]> {
    let provider = implemented_provider_id(provider)?;
    static EVENT_NAMES: std::sync::OnceLock<Vec<(&'static str, Box<[&'static str]>)>> =
        std::sync::OnceLock::new();
    EVENT_NAMES
        .get_or_init(|| {
            crate::hooks::profiles::all()
                .iter()
                .map(|profile| {
                    (
                        profile.provider,
                        crate::hooks::profiles::event_names(profile).into_boxed_slice(),
                    )
                })
                .collect()
        })
        .iter()
        .find_map(|(id, events)| (*id == provider).then_some(events.as_ref()))
}

pub fn install(provider: &str) -> Result<HookOperationReport> {
    crate::hooks::operations::run_operation(
        provider,
        crate::hooks::strategies::HookConfigOperation::Install,
    )
}

pub fn uninstall(provider: &str) -> Result<HookOperationReport> {
    crate::hooks::operations::run_operation(
        provider,
        crate::hooks::strategies::HookConfigOperation::Uninstall,
    )
}

pub fn repair(provider: &str) -> Result<HookOperationReport> {
    crate::hooks::operations::run_operation(
        provider,
        crate::hooks::strategies::HookConfigOperation::Repair,
    )
}

pub fn verify(provider: &str) -> Result<HookOperationReport> {
    crate::hooks::operations::run_operation(
        provider,
        crate::hooks::strategies::HookConfigOperation::Verify,
    )
}

fn implemented_provider_id(provider: &str) -> Option<&'static str> {
    let provider = crate::hooks::profiles::find(provider)?.provider;
    crate::hooks::operations::find_provider_hook(provider).map(|hook| hook.provider_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_provider_reflects_implemented_dispatch() {
        for descriptor in crate::hooks::registry::all() {
            assert!(
                supports_provider(descriptor.provider()),
                "profile is missing installer dispatch: {}",
                descriptor.provider()
            );
        }
        assert!(!supports_provider("unknown-provider"));
    }

    #[test]
    fn profile_events_match_installer_required_events() {
        for descriptor in crate::hooks::registry::all() {
            let profile_events = crate::hooks::profiles::event_names(descriptor.profile);
            let required_events = required_events(descriptor.provider())
                .unwrap_or_else(|| panic!("missing required events for {}", descriptor.provider()));
            assert_eq!(
                profile_events,
                required_events,
                "profile event coverage drifted from installer required events for {}",
                descriptor.provider()
            );
        }
    }

    #[test]
    fn command_base_contains_hidden_bridge_marker() {
        let command = crate::hooks::shared::bridge_command_base().unwrap();
        assert!(command.contains(HOOK_COMMAND_MARKER));
    }
}
