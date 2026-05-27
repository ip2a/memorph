mod opencode;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeatureScope {
    Global,
    Workspace,
    Session,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    Toggle,
    Action,
    View,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct ProviderFeature {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub scope: FeatureScope,
    pub kind: FeatureKind,
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

pub trait ProviderFeatureModule: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn features(&self) -> &'static [ProviderFeature];
}

struct ProviderFeatureRegistry;

impl ProviderFeatureRegistry {
    fn find(provider_id: &str) -> Option<&'static dyn ProviderFeatureModule> {
        match provider_id {
            "opencode" => Some(&opencode::OpenCodeFeatureModule),
            _ => None,
        }
    }
}

pub fn list_provider_features(provider_id: &str) -> Result<Vec<ResolvedProviderFeature>> {
    ensure_known_provider(provider_id)?;
    let prefs = crate::config::web_preferences()?;
    let Some(module) = ProviderFeatureRegistry::find(provider_id) else {
        return Ok(Vec::new());
    };

    Ok(module
        .features()
        .iter()
        .map(|feature| ResolvedProviderFeature {
            id: feature.id.to_string(),
            title: feature.title.to_string(),
            description: feature.description.to_string(),
            scope: feature.scope,
            kind: feature.kind,
            value: feature_value(&prefs, provider_id, feature),
        })
        .collect())
}

pub fn update_provider_feature(
    provider_id: &str,
    feature_id: &str,
    value: Option<Value>,
) -> Result<ResolvedProviderFeature> {
    let feature = feature(provider_id, feature_id)?;

    if feature.scope != FeatureScope::Global {
        anyhow::bail!(
            "Feature updates are only implemented for global scope: {}.{}",
            provider_id,
            feature_id
        );
    }

    match feature.kind {
        FeatureKind::Toggle => {
            if let Some(raw) = value.as_ref() {
                if !raw.is_boolean() {
                    anyhow::bail!(
                        "Toggle feature expects a boolean value: {}.{}",
                        provider_id,
                        feature_id
                    );
                }
            }
            crate::config::set_provider_preference(provider_id, feature_id, value)?;
        }
        _ => {
            anyhow::bail!(
                "Feature updates are not implemented for kind {:?}: {}.{}",
                feature.kind,
                provider_id,
                feature_id
            );
        }
    }

    list_provider_features(provider_id)?
        .into_iter()
        .find(|feature| feature.id == feature_id)
        .with_context(|| format!("Feature disappeared after update: {}.{}", provider_id, feature_id))
}

fn feature(provider_id: &str, feature_id: &str) -> Result<&'static ProviderFeature> {
    ensure_known_provider(provider_id)?;
    let module = ProviderFeatureRegistry::find(provider_id)
        .with_context(|| format!("Provider has no registered features: {}", provider_id))?;
    module
        .features()
        .iter()
        .find(|feature| feature.id == feature_id)
        .with_context(|| format!("Unknown provider feature: {}.{}", provider_id, feature_id))
}

fn feature_value(
    prefs: &crate::config::WebPreferences,
    provider_id: &str,
    feature: &ProviderFeature,
) -> Option<Value> {
    match feature.scope {
        FeatureScope::Global => {
            crate::config::provider_preference_from_prefs(prefs, provider_id, feature.id).cloned()
        }
        FeatureScope::Workspace | FeatureScope::Session => None,
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
    fn opencode_feature_registry_exposes_toggle() {
        let features = list_provider_features("opencode").unwrap();
        assert!(features.iter().any(|feature| feature.id == "show_subagents"));
    }
}
