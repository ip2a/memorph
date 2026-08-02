//! Hook provider descriptor registry.
//!
//! This is the standard read-side entry point for hook provider metadata.
//! It derives descriptors from provider profiles and common capability metadata;
//! provider-specific install/status logic remains owned by provider modules.

use crate::hooks::profiles::HookProviderProfile;
use crate::hooks::strategies::HookConfigStrategyKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookOperationCapabilities {
    pub scan_existing: bool,
    pub verify: bool,
    pub install: bool,
    pub repair: bool,
    pub uninstall: bool,
}

impl HookOperationCapabilities {
    const fn unsupported() -> Self {
        Self {
            scan_existing: false,
            verify: false,
            install: false,
            repair: false,
            uninstall: false,
        }
    }

    const fn managed_hook(scan_existing: bool) -> Self {
        Self {
            scan_existing,
            verify: true,
            install: true,
            repair: true,
            uninstall: true,
        }
    }

    pub fn supports_setting(self, setting_id: &str) -> bool {
        match setting_id {
            "install_hook" => self.install,
            "verify_hook" => self.verify,
            "repair_hook" => self.repair,
            "uninstall_hook" => self.uninstall,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookProviderDescriptor {
    pub profile: &'static HookProviderProfile,
    pub strategy_kind: HookConfigStrategyKind,
    pub capabilities: HookOperationCapabilities,
    pub required_events: &'static [&'static str],
}

impl HookProviderDescriptor {
    pub fn provider(self) -> &'static str {
        self.profile.provider
    }

    pub fn display_name(self) -> &'static str {
        self.profile.display_name
    }
}

pub fn all() -> Vec<HookProviderDescriptor> {
    crate::hooks::profiles::all()
        .iter()
        .filter_map(from_profile)
        .collect()
}

pub fn profiles() -> Vec<HookProviderProfile> {
    all()
        .into_iter()
        .map(|descriptor| *descriptor.profile)
        .collect()
}

pub fn provider_ids() -> impl Iterator<Item = &'static str> {
    crate::hooks::profiles::provider_ids()
}

pub fn find(provider: &str) -> Option<HookProviderDescriptor> {
    let profile = crate::hooks::profiles::find(provider)?;
    from_profile(profile)
}

pub fn profile(provider: &str) -> Option<&'static HookProviderProfile> {
    find(provider).map(|descriptor| descriptor.profile)
}

pub fn supports_provider(provider: &str) -> bool {
    find(provider).is_some()
}

pub fn required_events(provider: &str) -> Option<&'static [&'static str]> {
    find(provider).map(|descriptor| descriptor.required_events)
}

pub fn supports_setting(provider: &str, setting_id: &str) -> bool {
    find(provider)
        .map(|descriptor| descriptor.capabilities.supports_setting(setting_id))
        .unwrap_or(false)
}

fn from_profile(profile: &'static HookProviderProfile) -> Option<HookProviderDescriptor> {
    Some(HookProviderDescriptor {
        profile,
        strategy_kind: profile.strategy_kind,
        capabilities: if crate::hooks::operations::find_provider_hook(profile.provider).is_some() {
            HookOperationCapabilities::managed_hook(crate::hooks::discovery::supports_provider(
                profile.provider,
            ))
        } else {
            HookOperationCapabilities {
                scan_existing: crate::hooks::discovery::supports_provider(profile.provider),
                ..HookOperationCapabilities::unsupported()
            }
        },
        required_events: required_events_for_profile(profile),
    })
}

type EventNameRegistry = Vec<(&'static str, Box<[&'static str]>)>;

fn required_events_for_profile(profile: &'static HookProviderProfile) -> &'static [&'static str] {
    static EVENT_NAMES: std::sync::OnceLock<EventNameRegistry> = std::sync::OnceLock::new();
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
        .find_map(|(id, events)| (*id == profile.provider).then_some(events.as_ref()))
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_every_profiled_provider_as_descriptor() {
        let descriptors = all();
        assert_eq!(descriptors.len(), crate::hooks::profiles::all().len());
        for profile in crate::hooks::profiles::all() {
            let descriptor = find(profile.provider)
                .unwrap_or_else(|| panic!("missing descriptor for {}", profile.provider));
            assert_eq!(descriptor.provider(), profile.provider);
            assert_eq!(descriptor.profile, profile);
            assert_eq!(descriptor.strategy_kind, profile.strategy_kind);
        }
    }

    #[test]
    fn every_profiled_provider_has_full_managed_hook_capabilities() {
        for descriptor in all() {
            assert_eq!(
                descriptor.capabilities,
                HookOperationCapabilities::managed_hook(
                    crate::hooks::discovery::supports_provider(descriptor.provider())
                ),
                "missing capability coverage for {}",
                descriptor.provider()
            );
        }
    }

    #[test]
    fn unknown_provider_has_no_hook_capabilities() {
        assert!(!supports_setting("unknown-provider", "install_hook"));
        assert!(!supports_setting("unknown-provider", "verify_hook"));
    }

    #[test]
    fn capabilities_gate_hook_settings_only() {
        let unsupported = HookOperationCapabilities::unsupported();
        assert!(!unsupported.supports_setting("install_hook"));
        assert!(!unsupported.supports_setting("verify_hook"));
        assert!(!unsupported.supports_setting("repair_hook"));
        assert!(!unsupported.supports_setting("uninstall_hook"));
        assert!(unsupported.supports_setting("repair_workspace_sessions"));
    }

    #[test]
    fn descriptor_required_events_match_profile_events() {
        for descriptor in all() {
            assert_eq!(
                crate::hooks::profiles::event_names(descriptor.profile),
                descriptor.required_events,
                "descriptor required events drifted from profile for {}",
                descriptor.provider()
            );
        }
    }
}
