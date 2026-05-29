mod codex;
mod opencode;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct ProviderSettingContext {
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ProviderSettingOutput {
    CodexWorkspaceRepair(crate::providers::codex::CodexWorkspaceRepairReport),
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettingScope {
    Global,
    Workspace,
    Session,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    Toggle,
    Action,
    View,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderSettingItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub scope: SettingScope,
    pub kind: SettingKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SettingDefinition {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub scope: SettingScope,
    pub kind: SettingKind,
}

trait ProviderSettingModule: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn settings(&self) -> &'static [SettingDefinition];

    fn run(
        &self,
        setting_id: &str,
        _context: ProviderSettingContext,
    ) -> Result<ProviderSettingOutput> {
        anyhow::bail!(
            "Provider setting action is not implemented for provider: {} ({})",
            self.provider_id(),
            setting_id
        )
    }
}

struct ProviderSettingRegistry;

impl ProviderSettingRegistry {
    fn find(provider_id: &str) -> Option<&'static dyn ProviderSettingModule> {
        match provider_id {
            "codex" => Some(&codex::CodexSettingModule),
            "opencode" => Some(&opencode::OpenCodeSettingModule),
            _ => None,
        }
    }
}

pub fn list_provider_settings(provider_id: &str) -> Result<Vec<ProviderSettingItem>> {
    ensure_known_provider(provider_id)?;
    let prefs = crate::config::web_preferences()?;
    let Some(module) = ProviderSettingRegistry::find(provider_id) else {
        return Ok(Vec::new());
    };

    Ok(module
        .settings()
        .into_iter()
        .map(|setting| ProviderSettingItem {
            id: setting.id.to_string(),
            title: setting.title.to_string(),
            description: setting.description.to_string(),
            scope: setting.scope,
            kind: setting.kind,
            value: setting_value(&prefs, provider_id, setting),
        })
        .collect())
}

pub fn get_provider_setting(provider_id: &str, setting_id: &str) -> Result<ProviderSettingItem> {
    list_provider_settings(provider_id)?
        .into_iter()
        .find(|setting| setting.id == setting_id)
        .with_context(|| format!("Unknown provider setting: {}.{}", provider_id, setting_id))
}

pub fn update_provider_setting(
    provider_id: &str,
    setting_id: &str,
    value: Option<Value>,
) -> Result<ProviderSettingItem> {
    let setting = setting(provider_id, setting_id)?;

    if setting.scope != SettingScope::Global {
        anyhow::bail!(
            "Provider setting updates are only implemented for global scope: {}.{}",
            provider_id,
            setting_id
        );
    }

    match setting.kind {
        SettingKind::Toggle => {
            if let Some(raw) = value.as_ref() {
                if !raw.is_boolean() {
                    anyhow::bail!(
                        "Toggle setting expects a boolean value: {}.{}",
                        provider_id,
                        setting_id
                    );
                }
            }
            crate::config::set_provider_preference(provider_id, setting_id, value)?;
        }
        _ => {
            anyhow::bail!(
                "Provider setting updates are not implemented for kind {:?}: {}.{}",
                setting.kind,
                provider_id,
                setting_id
            );
        }
    }

    list_provider_settings(provider_id)?
        .into_iter()
        .find(|current| current.id == setting_id)
        .with_context(|| {
            format!(
                "Provider setting disappeared after update: {}.{}",
                provider_id, setting_id
            )
        })
}

pub fn run_provider_setting(
    provider_id: &str,
    setting_id: &str,
    context: ProviderSettingContext,
) -> Result<ProviderSettingOutput> {
    let setting = setting(provider_id, setting_id)?;
    if setting.kind != SettingKind::Action {
        anyhow::bail!(
            "Provider setting is not an action: {}.{}",
            provider_id,
            setting_id
        );
    }

    let module = ProviderSettingRegistry::find(provider_id)
        .with_context(|| format!("Provider has no registered settings: {}", provider_id))?;
    module.run(setting_id, context)
}

fn setting(provider_id: &str, setting_id: &str) -> Result<&'static SettingDefinition> {
    ensure_known_provider(provider_id)?;
    let module = ProviderSettingRegistry::find(provider_id)
        .with_context(|| format!("Provider has no registered settings: {}", provider_id))?;
    module
        .settings()
        .iter()
        .find(|setting| setting.id == setting_id)
        .with_context(|| format!("Unknown provider setting: {}.{}", provider_id, setting_id))
}

fn setting_value(
    prefs: &crate::config::WebPreferences,
    provider_id: &str,
    setting: &SettingDefinition,
) -> Option<Value> {
    match setting.scope {
        SettingScope::Global => {
            crate::config::provider_preference_from_prefs(prefs, provider_id, setting.id).cloned()
        }
        SettingScope::Workspace | SettingScope::Session => None,
    }
}

fn ensure_known_provider(provider_id: &str) -> Result<()> {
    if crate::providers::all_provider_ids()
        .iter()
        .any(|known| *known == provider_id)
    {
        return Ok(());
    }

    anyhow::bail!("Unknown provider: {}", provider_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_settings_expose_opencode_toggle() {
        let settings = list_provider_settings("opencode").unwrap();
        assert!(settings
            .iter()
            .any(|setting| setting.id == "show_subagents"));
    }

    #[test]
    fn provider_settings_expose_codex_action() {
        let settings = list_provider_settings("codex").unwrap();
        let setting = settings
            .iter()
            .find(|setting| setting.id == "repair_workspace_sessions")
            .expect("missing codex repair setting");
        assert_eq!(setting.kind, SettingKind::Action);
        assert_eq!(setting.scope, SettingScope::Workspace);
    }
}
