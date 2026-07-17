//! Hook provider profile types and read-side facade.
//!
//! Provider-specific profile data lives under `providers::hook_profiles`; this
//! module exposes the common metadata types and stable lookup functions used by
//! hooks, API, diagnostics, and UI code.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookFormat {
    ClaudeNestedJson,
    CodexJson,
    CursorFlatJson,
    GeminiNestedJson,
    CopilotJson,
    KimiToml,
    KiroAgentJson,
    OpenCodePlugin,
    QwenNestedJson,
    TraeYaml,
    QoderClaudeJson,
    FactoryClaudeJson,
    CodeBuddyClaudeJson,
    StepFunClaudeJson,
    AntiGravityClaudeJson,
    WorkBuddyClaudeJson,
    HermesClaudeJson,
    PiExtension,
    ClineFiles,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct HookProviderEventProfile {
    pub name: &'static str,
    pub blocking: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct HookProviderProfile {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub format: HookFormat,
    pub strategy_kind: crate::hooks::strategies::HookConfigStrategyKind,
    pub config_hint: &'static str,
    pub events: &'static [HookProviderEventProfile],
}

pub fn all() -> &'static [HookProviderProfile] {
    crate::providers::hook_profiles::all()
}

pub fn provider_ids() -> impl Iterator<Item = &'static str> {
    crate::providers::hook_profiles::provider_ids()
}

pub fn find(provider: &str) -> Option<&'static HookProviderProfile> {
    crate::providers::hook_profiles::find(provider)
}

pub fn supports_provider(provider: &str) -> bool {
    crate::providers::hook_profiles::supports_provider(provider)
}

pub fn event_names(profile: &HookProviderProfile) -> Vec<&'static str> {
    crate::providers::hook_profiles::event_names(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_exposes_non_empty_provider_profiles() {
        assert!(!all().is_empty());
        for profile in all() {
            assert!(find(profile.provider).is_some());
            assert!(supports_provider(profile.provider));
            assert_eq!(event_names(profile).len(), profile.events.len());
        }
    }

    #[test]
    fn provider_ids_are_derived_from_profiles() {
        let ids: Vec<_> = provider_ids().collect();
        assert_eq!(ids.len(), all().len());
        for profile in all() {
            assert!(ids.contains(&profile.provider));
        }
    }
}
