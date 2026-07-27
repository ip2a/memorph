mod antigravity;
mod claude;
mod cline;
mod codebuddy;
mod codex;
mod common_hooks;
mod copilot;
mod cursor;
mod droid;
mod gemini;
mod hermes;
mod kimi;
mod kiro;
mod opencode;
mod pi;
mod qoder;
mod registry;
mod trae;
mod workbuddy;

use anyhow::{Context as _, Result};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ProviderSettingContext {
    pub workspace: Option<String>,
    pub actor: crate::storage::activity_store::ActivityActor,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ProviderSettingOutput {
    CodexWorkspaceRepair(crate::providers::codex::CodexWorkspaceRepairReport),
    HookOperation(crate::hooks::model::HookOperationReport),
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

pub fn list_provider_settings(provider_id: &str) -> Result<Vec<ProviderSettingItem>> {
    ensure_known_provider(provider_id)?;
    let prefs = crate::config::web_preferences()?;
    let Some(module) = registry::find_provider_setting_module(provider_id) else {
        return Ok(Vec::new());
    };

    Ok(provider_setting_definitions(module, provider_id)
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

    if common_hooks::is_common_hook_setting(setting_id) {
        return common_hooks::run(provider_id, setting_id);
    }

    let module = registry::find_provider_setting_module(provider_id)
        .with_context(|| format!("Provider has no registered settings: {}", provider_id))?;
    module.run(setting_id, context)
}

fn setting(provider_id: &str, setting_id: &str) -> Result<&'static SettingDefinition> {
    ensure_known_provider(provider_id)?;
    let module = registry::find_provider_setting_module(provider_id)
        .with_context(|| format!("Provider has no registered settings: {}", provider_id))?;
    provider_setting_definitions(module, provider_id)
        .into_iter()
        .find(|setting| setting.id == setting_id)
        .with_context(|| format!("Unknown provider setting: {}.{}", provider_id, setting_id))
}

fn provider_setting_definitions(
    module: &'static dyn ProviderSettingModule,
    provider_id: &str,
) -> Vec<&'static SettingDefinition> {
    let mut settings = Vec::new();
    for setting in module.settings() {
        push_unique_setting(&mut settings, setting);
    }
    for setting in common_hooks::definitions_for(provider_id) {
        push_unique_setting(&mut settings, setting);
    }
    settings
}

fn push_unique_setting(
    settings: &mut Vec<&'static SettingDefinition>,
    setting: &'static SettingDefinition,
) {
    if !settings.iter().any(|current| current.id == setting.id) {
        settings.push(setting);
    }
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
    if crate::providers::find_provider(provider_id).is_some() {
        return Ok(());
    }

    anyhow::bail!("Unknown provider: {}", provider_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_settings_generate_common_hook_actions_for_every_hook_descriptor() {
        for descriptor in crate::hooks::registry::all() {
            let settings = list_provider_settings(descriptor.provider()).unwrap();
            for common in common_hooks::definitions_for(descriptor.provider()) {
                let matches: Vec<_> = settings
                    .iter()
                    .filter(|setting| setting.id == common.id)
                    .collect();
                assert_eq!(
                    matches.len(),
                    1,
                    "expected one {} setting for {}",
                    common.id,
                    descriptor.provider()
                );
                assert_eq!(matches[0].kind, SettingKind::Action);
                assert_eq!(matches[0].scope, SettingScope::Global);
            }
        }
    }

    #[test]
    fn provider_specific_settings_are_preserved() {
        let codex = list_provider_settings("codex").unwrap();
        let repair = codex
            .iter()
            .find(|setting| setting.id == "repair_workspace_sessions")
            .expect("missing codex repair setting");
        assert_eq!(repair.kind, SettingKind::Action);
        assert_eq!(repair.scope, SettingScope::Workspace);

        let opencode = list_provider_settings("opencode").unwrap();
        let show_subagents = opencode
            .iter()
            .find(|setting| setting.id == "show_subagents")
            .expect("missing opencode show_subagents setting");
        assert_eq!(show_subagents.kind, SettingKind::Toggle);
        assert_eq!(show_subagents.scope, SettingScope::Global);
    }

    #[test]
    fn generated_common_hook_action_resolves_through_setting_lookup() {
        let setting = setting("claude", "verify_hook").unwrap();
        assert_eq!(setting.id, "verify_hook");
        assert_eq!(setting.kind, SettingKind::Action);
    }

    #[test]
    fn alias_lists_same_provider_specific_settings_as_canonical_id() {
        let canonical = list_provider_settings("droid").unwrap();
        let alias = list_provider_settings("factory").unwrap();

        assert_eq!(alias.len(), canonical.len());
        assert_eq!(
            alias
                .iter()
                .map(|setting| setting.id.as_str())
                .collect::<Vec<_>>(),
            canonical
                .iter()
                .map(|setting| setting.id.as_str())
                .collect::<Vec<_>>()
        );
    }
}
