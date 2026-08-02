use anyhow::{Context as _, Result};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

const AGENT_BUILD_PARALLELISM: usize = 6;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentManagementEntry {
    pub provider_id: String,
    pub name: String,
    pub environment: crate::agent_environment::AgentEnvironmentStatus,
    pub capabilities: crate::agent_capabilities::AgentCapabilityManifest,
    pub settings: Vec<crate::provider_settings::ProviderSettingItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentManagementSummaryEntry {
    pub provider_id: String,
    pub name: String,
    pub environment: crate::agent_environment::AgentEnvironmentStatus,
    pub capabilities: crate::agent_capabilities::AgentCapabilityManifest,
    pub settings: Vec<crate::provider_settings::ProviderSettingItem>,
}

pub fn list_agent_management_entries() -> Result<Vec<AgentManagementEntry>> {
    build_provider_results_parallel(
        |provider_id| build_agent_management_entry(provider_id, false),
        "agent management worker did not return an entry",
    )
}

pub fn list_agent_management_summaries() -> Result<Vec<AgentManagementSummaryEntry>> {
    build_provider_results_parallel(
        build_agent_management_summary,
        "agent management summary worker did not return an entry",
    )
}

pub fn get_agent_management_entry(provider_id: &str) -> Result<AgentManagementEntry> {
    build_agent_management_entry(provider_id, false)
}

pub fn detect_agent_management_entry(provider_id: &str) -> Result<AgentManagementEntry> {
    build_agent_management_entry(provider_id, true)
}

fn build_provider_results_parallel<T, F>(build: F, missing_context: &'static str) -> Result<Vec<T>>
where
    T: Send,
    F: Fn(&str) -> Result<T> + Sync,
{
    let provider_ids = crate::providers::all_provider_ids();
    let mut entries: Vec<Option<T>> = std::iter::repeat_with(|| None)
        .take(provider_ids.len())
        .collect();
    let workers = AGENT_BUILD_PARALLELISM.min(provider_ids.len()).max(1);
    let next_index = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel();

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let tx = tx.clone();
            let build = &build;
            let next_index = &next_index;
            scope.spawn(move || loop {
                let index = next_index.fetch_add(1, Ordering::Relaxed);
                if index >= provider_ids.len() {
                    return;
                }
                let result = build(provider_ids[index]);
                if tx.send((index, result)).is_err() {
                    return;
                }
            });
        }
    });
    drop(tx);

    for (index, result) in rx {
        entries[index] = Some(result?);
    }

    entries
        .into_iter()
        .map(|entry| entry.context(missing_context))
        .collect()
}

fn build_agent_management_summary(provider_id: &str) -> Result<AgentManagementSummaryEntry> {
    let provider = crate::providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    let settings = crate::provider_settings::list_provider_settings(provider_id)?;
    let capabilities = crate::agent_capabilities::build_manifest(provider_id)?;

    Ok(AgentManagementSummaryEntry {
        provider_id: provider_id.to_string(),
        name: provider.name().to_string(),
        environment: crate::agent_environment::detect_provider_environment_fast(provider_id),
        capabilities,
        settings,
    })
}

fn build_agent_management_entry(
    provider_id: &str,
    refresh_environment: bool,
) -> Result<AgentManagementEntry> {
    let provider = crate::providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    let settings = crate::provider_settings::list_provider_settings(provider_id)?;
    let capabilities = crate::agent_capabilities::build_manifest(provider_id)?;

    Ok(AgentManagementEntry {
        provider_id: provider_id.to_string(),
        name: provider.name().to_string(),
        environment: if refresh_environment {
            crate::agent_environment::refresh_provider_environment(provider_id)
        } else {
            crate::agent_environment::detect_provider_environment(provider_id)
        },
        capabilities,
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_management_entry_exposes_capabilities() {
        let claude = build_agent_management_entry("claude", false).unwrap();
        assert_eq!(claude.capabilities.provider_id, "claude");
        assert!(!claude
            .capabilities
            .hook_management
            .as_ref()
            .unwrap()
            .status
            .is_empty());
    }

    #[test]
    fn agent_management_entry_exposes_settings() {
        let codex = build_agent_management_entry("codex", false).unwrap();
        assert!(codex
            .settings
            .iter()
            .any(|setting| setting.id == "repair_workspace_sessions"));

        let opencode = build_agent_management_entry("opencode", false).unwrap();
        assert!(opencode
            .settings
            .iter()
            .any(|setting| setting.id == "show_subagents"));
    }

    #[test]
    fn every_hook_profile_provider_has_hook_capabilities() {
        let entries = list_agent_management_entries().unwrap();
        for descriptor in crate::hooks::registry::all() {
            let entry = entries
                .iter()
                .find(|entry| entry.provider_id == descriptor.provider())
                .unwrap_or_else(|| {
                    panic!(
                        "missing agent management entry for {}",
                        descriptor.provider()
                    )
                });
            let hook = entry.capabilities.hook_management.as_ref().unwrap();
            assert_eq!(hook.install, descriptor.capabilities.install);
            assert_eq!(hook.verify, descriptor.capabilities.verify);
            assert_eq!(hook.repair, descriptor.capabilities.repair);
            assert_eq!(hook.uninstall, descriptor.capabilities.uninstall);
            assert_eq!(hook.discovery, descriptor.capabilities.scan_existing);
        }
    }

    #[test]
    fn agent_management_entry_groups_common_environment_fields() {
        let codex = build_agent_management_entry("codex", false).unwrap();
        let environment = crate::agent_environment::detect_provider_environment("codex");
        assert_eq!(codex.environment.config_path, environment.config_path);
        assert!(!codex.environment.install_method.trim().is_empty());
    }

    #[test]
    fn agent_management_entry_serializes_new_contract() {
        let codex = build_agent_management_entry("codex", false).unwrap();
        let value = serde_json::to_value(&codex).unwrap();

        assert_eq!(value["provider_id"], "codex");
        assert!(value["environment"].is_object());
        assert!(value["capabilities"].is_object());
        assert!(value["capabilities"]["session_management"].is_object());
        assert!(value["capabilities"]["hook_management"].is_object());
        assert!(value["settings"].is_array());
        assert!(value.get("hook").is_none());
    }
}
