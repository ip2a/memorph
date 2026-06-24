use anyhow::{Context, Result};
use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentManagementEntry {
    pub provider_id: String,
    pub name: String,
    pub environment: crate::agent_environment::AgentEnvironmentStatus,
    pub hook: crate::hooks::model::HookInstallStatus,
    pub hook_strategy: Option<crate::hooks::strategies::HookConfigStrategyKind>,
    pub hook_capabilities: crate::hooks::capabilities::HookProviderCapabilities,
    pub hook_diagnosis: crate::hooks::augmentation::ProviderHookDiagnosisAggregate,
    pub hook_profile: Option<crate::hooks::profiles::HookProviderProfile>,
    pub hook_required_events: Vec<&'static str>,
    pub settings: Vec<crate::provider_settings::ProviderSettingItem>,
}

impl Serialize for AgentManagementEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AgentManagementEntry", 15)?;
        state.serialize_field("provider_id", &self.provider_id)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("environment", &self.environment)?;
        state.serialize_field("hook", &self.hook)?;
        state.serialize_field("hook_strategy", &self.hook_strategy)?;
        state.serialize_field("hook_capabilities", &self.hook_capabilities)?;
        state.serialize_field("hook_diagnosis", &self.hook_diagnosis)?;
        state.serialize_field("hook_profile", &self.hook_profile)?;
        state.serialize_field("hook_required_events", &self.hook_required_events)?;
        state.serialize_field("installed", &self.environment.installed)?;
        if let Some(path) = &self.environment.executable_path {
            state.serialize_field("executable_path", path)?;
        }
        if let Some(path) = &self.environment.executable_dir {
            state.serialize_field("executable_dir", path)?;
        }
        state.serialize_field("config_path", &self.environment.config_path)?;
        state.serialize_field("install_method", &self.environment.install_method)?;
        state.serialize_field("settings", &self.settings)?;
        state.end()
    }
}

pub fn list_agent_management_entries() -> Result<Vec<AgentManagementEntry>> {
    let runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
    crate::providers::all_provider_ids()
        .iter()
        .map(|provider_id| build_agent_management_entry(provider_id, &runtime_snapshot))
        .collect()
}

pub fn get_agent_management_entry(provider_id: &str) -> Result<AgentManagementEntry> {
    let runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
    build_agent_management_entry(provider_id, &runtime_snapshot)
}

pub fn detect_agent_management_entry(provider_id: &str) -> Result<AgentManagementEntry> {
    let runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
    build_agent_management_entry(provider_id, &runtime_snapshot)
}

fn build_agent_management_entry(
    provider_id: &str,
    runtime_snapshot: &[crate::hooks::model::RuntimeSession],
) -> Result<AgentManagementEntry> {
    let provider = crate::providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let settings = crate::provider_settings::list_provider_settings(provider_id)?;
    let hook = crate::hooks::operations::status(provider_id)?;
    let hook_descriptor = crate::hooks::registry::find(provider_id);
    let session_summaries = if provider.capabilities().scan {
        let cache = crate::cache::global_cache();
        cache.get_or_refresh(provider_id, || provider.scan_sessions())?
    } else {
        Vec::new()
    };
    let hook_diagnosis = crate::hooks::augmentation::aggregate_provider_sessions(
        runtime_snapshot,
        hook.clone(),
        provider_id,
        &session_summaries,
    );

    Ok(AgentManagementEntry {
        provider_id: provider_id.to_string(),
        name: provider.name().to_string(),
        environment: crate::agent_environment::detect_provider_environment(provider_id),
        hook,
        hook_strategy: hook_descriptor.map(|descriptor| descriptor.strategy_kind),
        hook_capabilities: hook_descriptor
            .map(|descriptor| descriptor.capabilities)
            .unwrap_or_else(crate::hooks::capabilities::HookProviderCapabilities::unsupported),
        hook_diagnosis,
        hook_profile: hook_descriptor.map(|descriptor| *descriptor.profile),
        hook_required_events: hook_descriptor
            .map(|descriptor| descriptor.required_events.to_vec())
            .unwrap_or_default(),
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_management_entry_exposes_hook_status() {
        let runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
        let claude = build_agent_management_entry("claude", &runtime_snapshot).unwrap();
        assert_eq!(claude.hook.provider, "claude");
    }

    #[test]
    fn agent_management_entry_exposes_settings() {
        let runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
        let codex = build_agent_management_entry("codex", &runtime_snapshot).unwrap();
        assert!(codex
            .settings
            .iter()
            .any(|setting| setting.id == "repair_workspace_sessions"));

        let opencode = build_agent_management_entry("opencode", &runtime_snapshot).unwrap();
        assert!(opencode
            .settings
            .iter()
            .any(|setting| setting.id == "show_subagents"));
    }
    #[test]
    fn agent_management_exposes_every_hook_profile_provider() {
        let entries = list_agent_management_entries().unwrap();
        for descriptor in crate::hooks::registry::all() {
            let profile = descriptor.profile;
            let entry = entries
                .iter()
                .find(|entry| entry.provider_id == profile.provider)
                .unwrap_or_else(|| {
                    panic!("missing agent management entry for {}", profile.provider)
                });
            assert_eq!(entry.hook.provider, profile.provider);
            assert_eq!(
                entry.hook_profile.as_ref().map(|profile| profile.provider),
                Some(profile.provider)
            );
            assert_eq!(
                entry.hook_required_events, descriptor.required_events,
                "required event payload drifted from profile for {}",
                profile.provider
            );
            assert_eq!(
                entry.hook_capabilities,
                crate::hooks::capabilities::HookProviderCapabilities::managed_hook(),
                "missing hook capabilities for {}",
                profile.provider
            );
            assert!(
                entry
                    .settings
                    .iter()
                    .any(|setting| setting.id == "install_hook"),
                "missing install_hook action for {}",
                profile.provider
            );
            assert!(
                !entry.environment.config_path.trim().is_empty(),
                "missing environment config path for {}",
                profile.provider
            );
        }
    }

    #[test]
    fn agent_management_entry_groups_common_environment_fields() {
        let runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
        let codex = build_agent_management_entry("codex", &runtime_snapshot).unwrap();
        let environment = crate::agent_environment::detect_provider_environment("codex");
        assert_eq!(codex.environment.config_path, environment.config_path);
        assert!(!codex.environment.install_method.trim().is_empty());
    }

    #[test]
    fn agent_management_entry_serializes_environment_block_and_flat_compat_fields() {
        let runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
        let codex = build_agent_management_entry("codex", &runtime_snapshot).unwrap();
        let value = serde_json::to_value(&codex).unwrap();

        assert_eq!(value["provider_id"], "codex");
        assert!(value["environment"].is_object());
        assert!(value["hook"].is_object());
        assert!(value["hook_strategy"].is_string());
        assert!(value["hook_capabilities"].is_object());
        assert_eq!(value["hook_capabilities"]["install"], true);
        assert!(value["hook_diagnosis"].is_object());
        assert!(value["hook_profile"].is_object());
        assert!(value["hook_required_events"].is_array());
        assert!(!value["hook_required_events"].as_array().unwrap().is_empty());
        assert_eq!(value["environment"]["config_path"], value["config_path"]);
        assert_eq!(
            value["environment"]["install_method"],
            value["install_method"]
        );
        assert_eq!(value["environment"]["installed"], value["installed"]);
    }
}
