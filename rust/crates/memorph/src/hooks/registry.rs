//! Hook provider descriptor registry.
//!
//! This is the standard read-side entry point for hook provider metadata.
//! It derives descriptors from provider profiles and common capability metadata;
//! provider-specific install/status logic remains owned by provider modules.

use crate::hooks::capabilities::HookProviderCapabilities;
use crate::hooks::profiles::HookProviderProfile;
use crate::hooks::strategies::HookConfigStrategyKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookProviderDescriptor {
    pub profile: &'static HookProviderProfile,
    pub strategy_kind: HookConfigStrategyKind,
    pub capabilities: HookProviderCapabilities,
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

fn from_profile(profile: &'static HookProviderProfile) -> Option<HookProviderDescriptor> {
    Some(HookProviderDescriptor {
        profile,
        strategy_kind: profile.strategy_kind,
        capabilities: crate::hooks::capabilities::for_provider(profile.provider),
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
