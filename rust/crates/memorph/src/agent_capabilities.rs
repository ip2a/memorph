use anyhow::{Context as _, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentCapabilityManifest {
    pub provider_id: String,
    pub session_management: SessionManagementCapability,
    pub hook_management: Option<HookManagementCapability>,
    pub mcp_management: Option<McpManagementCapability>,
    pub plugin_management: Option<PluginManagementCapability>,
    pub config_views: Vec<ConfigViewCapability>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionManagementCapability {
    pub scan: bool,
    pub import: bool,
    pub export: bool,
    pub delete: bool,
    pub rename: bool,
    pub resume: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookManagementCapability {
    pub install: bool,
    pub verify: bool,
    pub repair: bool,
    pub uninstall: bool,
    pub discovery: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct McpManagementCapability {
    pub list: bool,
    pub inspect: bool,
    pub remove: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginManagementCapability {
    pub list: bool,
    pub inspect: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConfigViewCapability {
    pub id: String,
    pub title: String,
    pub description: String,
}

pub fn build_manifest(provider_id: &str) -> Result<AgentCapabilityManifest> {
    let provider = crate::providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    let capabilities = provider.capabilities();
    let settings = crate::provider_settings::list_provider_settings(provider_id)?;
    let hook_management = crate::hooks::registry::find(provider_id)
        .map(|descriptor| -> Result<_> {
            let status = crate::hooks::operations::status(provider_id)?;
            Ok(HookManagementCapability {
                install: descriptor.capabilities.install,
                verify: descriptor.capabilities.verify,
                repair: descriptor.capabilities.repair,
                uninstall: descriptor.capabilities.uninstall,
                discovery: descriptor.capabilities.scan_existing,
                status: serde_json::to_value(status.status)?
                    .as_str()
                    .context("hook status was not serialized as a string")?
                    .to_string(),
            })
        })
        .transpose()?;

    let has_mcp = settings.iter().any(|setting| setting.id == "view_mcp");
    let has_plugins = settings.iter().any(|setting| setting.id == "view_plugins");
    let config_views = settings
        .iter()
        .filter(|setting| setting.kind == crate::provider_settings::SettingKind::View)
        .map(|setting| ConfigViewCapability {
            id: setting.id.clone(),
            title: setting.title.clone(),
            description: setting.description.clone(),
        })
        .collect();

    Ok(AgentCapabilityManifest {
        provider_id: provider_id.to_string(),
        session_management: SessionManagementCapability {
            scan: capabilities.scan,
            import: capabilities.import,
            export: capabilities.export,
            delete: capabilities.delete,
            rename: capabilities.rename,
            resume: capabilities.resume,
        },
        hook_management,
        mcp_management: has_mcp.then_some(McpManagementCapability {
            list: true,
            inspect: true,
            remove: has_mcp
                && matches!(
                    crate::providers::canonical_provider_id(provider_id).as_str(),
                    "claude" | "codex" | "opencode"
                ),
        }),
        plugin_management: has_plugins.then_some(PluginManagementCapability {
            list: true,
            inspect: true,
        }),
        config_views,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_provider_settings_as_capabilities() {
        let manifest = build_manifest("claude").unwrap();
        assert!(manifest.hook_management.is_some());
        assert!(manifest.mcp_management.is_some());
        assert_eq!(
            manifest.mcp_management.as_ref().map(|mcp| mcp.remove),
            Some(true)
        );
        assert!(manifest.plugin_management.is_some());
        assert!(manifest
            .config_views
            .iter()
            .any(|view| view.id == "view_mcp"));
    }

    #[test]
    fn manifest_rejects_unknown_provider() {
        assert!(build_manifest("unknown-provider").is_err());
    }

    #[test]
    fn manifest_does_not_advertise_mcp_removal_without_a_removable_view() {
        let manifest = build_manifest("gemini").unwrap();
        assert!(manifest.mcp_management.is_none());
    }
}
