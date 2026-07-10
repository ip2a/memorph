//! Legacy compatibility layer for feature-named routes and commands.

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeatureScope {
    Global,
    Workspace,
    Session,
}

impl From<crate::provider_settings::SettingScope> for FeatureScope {
    fn from(scope: crate::provider_settings::SettingScope) -> Self {
        match scope {
            crate::provider_settings::SettingScope::Global => Self::Global,
            crate::provider_settings::SettingScope::Workspace => Self::Workspace,
            crate::provider_settings::SettingScope::Session => Self::Session,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    Toggle,
    Action,
    View,
}

impl From<crate::provider_settings::SettingKind> for FeatureKind {
    fn from(kind: crate::provider_settings::SettingKind) -> Self {
        match kind {
            crate::provider_settings::SettingKind::Toggle => Self::Toggle,
            crate::provider_settings::SettingKind::Action => Self::Action,
            crate::provider_settings::SettingKind::View => Self::View,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedProviderFeature {
    pub id: String,
    pub title: String,
    pub description: String,
    pub scope: FeatureScope,
    pub kind: FeatureKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

impl From<crate::provider_settings::ProviderSettingItem> for ResolvedProviderFeature {
    fn from(setting: crate::provider_settings::ProviderSettingItem) -> Self {
        Self {
            id: setting.id,
            title: setting.title,
            description: setting.description,
            scope: setting.scope.into(),
            kind: setting.kind.into(),
            value: setting.value,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderFeatureContext {
    pub workspace: Option<String>,
    pub actor: crate::storage::activity_store::ActivityActor,
}

impl From<ProviderFeatureContext> for crate::provider_settings::ProviderSettingContext {
    fn from(context: ProviderFeatureContext) -> Self {
        Self {
            workspace: context.workspace,
            actor: context.actor,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ProviderFeatureOutput {
    CodexWorkspaceRepair(crate::providers::codex::CodexWorkspaceRepairReport),
    HookOperation(crate::hooks::model::HookOperationReport),
}

impl From<crate::provider_settings::ProviderSettingOutput> for ProviderFeatureOutput {
    fn from(output: crate::provider_settings::ProviderSettingOutput) -> Self {
        match output {
            crate::provider_settings::ProviderSettingOutput::CodexWorkspaceRepair(report) => {
                Self::CodexWorkspaceRepair(report)
            }
            crate::provider_settings::ProviderSettingOutput::HookOperation(report) => {
                Self::HookOperation(report)
            }
        }
    }
}

pub fn list_provider_features(provider_id: &str) -> Result<Vec<ResolvedProviderFeature>> {
    Ok(
        crate::provider_settings::list_provider_settings(provider_id)?
            .into_iter()
            .map(ResolvedProviderFeature::from)
            .collect(),
    )
}

pub fn get_provider_feature(
    provider_id: &str,
    feature_id: &str,
) -> Result<ResolvedProviderFeature> {
    list_provider_features(provider_id)?
        .into_iter()
        .find(|feature| feature.id == feature_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown provider feature: {}.{}", provider_id, feature_id))
}

pub fn update_provider_feature(
    provider_id: &str,
    feature_id: &str,
    value: Option<Value>,
) -> Result<ResolvedProviderFeature> {
    crate::provider_settings::update_provider_setting(provider_id, feature_id, value)
        .map(ResolvedProviderFeature::from)
}

pub fn run_provider_feature(
    provider_id: &str,
    feature_id: &str,
    context: ProviderFeatureContext,
) -> Result<ProviderFeatureOutput> {
    crate::provider_settings::run_provider_setting(provider_id, feature_id, context.into())
        .map(ProviderFeatureOutput::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_feature_registry_exposes_toggle() {
        let features = list_provider_features("opencode").unwrap();
        assert!(features
            .iter()
            .any(|feature| feature.id == "show_subagents"));
    }

    #[test]
    fn codex_feature_registry_exposes_repair_action() {
        let features = list_provider_features("codex").unwrap();
        let feature = features
            .iter()
            .find(|feature| feature.id == "repair_workspace_sessions")
            .expect("missing codex repair action");
        assert_eq!(feature.kind, FeatureKind::Action);
        assert_eq!(feature.scope, FeatureScope::Workspace);
    }

    #[test]
    fn legacy_feature_wrapper_matches_provider_settings() {
        for provider_id in ["codex", "opencode"] {
            let legacy = list_provider_features(provider_id).unwrap();
            let settings = crate::provider_settings::list_provider_settings(provider_id).unwrap();

            assert_eq!(legacy.len(), settings.len());
            for (feature, setting) in legacy.iter().zip(settings.iter()) {
                assert_eq!(feature.id, setting.id);
                assert_eq!(feature.title, setting.title);
                assert_eq!(feature.description, setting.description);
                assert_eq!(feature.value, setting.value);
                assert_eq!(feature.scope, setting.scope.into());
                assert_eq!(feature.kind, setting.kind.into());
            }
        }
    }
}
