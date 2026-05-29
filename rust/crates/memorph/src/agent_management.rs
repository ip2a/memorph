use anyhow::{Context, Result};
use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentManagementEntry {
    pub provider_id: String,
    pub name: String,
    pub environment: crate::agent_environment::AgentEnvironmentStatus,
    pub settings: Vec<crate::provider_settings::ProviderSettingItem>,
}

impl Serialize for AgentManagementEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AgentManagementEntry", 9)?;
        state.serialize_field("provider_id", &self.provider_id)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("environment", &self.environment)?;
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
    crate::providers::all_provider_ids()
        .iter()
        .map(|provider_id| build_agent_management_entry(provider_id))
        .collect()
}

pub fn get_agent_management_entry(provider_id: &str) -> Result<AgentManagementEntry> {
    build_agent_management_entry(provider_id)
}

pub fn detect_agent_management_entry(provider_id: &str) -> Result<AgentManagementEntry> {
    build_agent_management_entry(provider_id)
}

fn build_agent_management_entry(provider_id: &str) -> Result<AgentManagementEntry> {
    let provider = crate::providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let settings = crate::provider_settings::list_provider_settings(provider_id)?;

    Ok(AgentManagementEntry {
        provider_id: provider_id.to_string(),
        name: provider.name().to_string(),
        environment: crate::agent_environment::detect_provider_environment(provider_id),
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_management_entry_exposes_settings() {
        let codex = build_agent_management_entry("codex").unwrap();
        assert!(codex
            .settings
            .iter()
            .any(|setting| setting.id == "repair_workspace_sessions"));

        let opencode = build_agent_management_entry("opencode").unwrap();
        assert!(opencode
            .settings
            .iter()
            .any(|setting| setting.id == "show_subagents"));
    }

    #[test]
    fn agent_management_entry_groups_common_environment_fields() {
        let codex = build_agent_management_entry("codex").unwrap();
        let environment = crate::agent_environment::detect_provider_environment("codex");
        assert_eq!(codex.environment.config_path, environment.config_path);
        assert!(!codex.environment.install_method.trim().is_empty());
    }

    #[test]
    fn agent_management_entry_serializes_environment_block_and_flat_compat_fields() {
        let codex = build_agent_management_entry("codex").unwrap();
        let value = serde_json::to_value(&codex).unwrap();

        assert_eq!(value["provider_id"], "codex");
        assert!(value["environment"].is_object());
        assert_eq!(value["environment"]["config_path"], value["config_path"]);
        assert_eq!(
            value["environment"]["install_method"],
            value["install_method"]
        );
        assert_eq!(value["environment"]["installed"], value["installed"]);
    }
}
