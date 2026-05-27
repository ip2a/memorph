use super::{FeatureKind, FeatureScope, ProviderFeature, ProviderFeatureModule};

pub struct OpenCodeFeatureModule;

const FEATURES: &[ProviderFeature] = &[ProviderFeature {
    id: "show_subagents",
    title: "Show subagents",
    description: "Expose OpenCode subagent sessions in provider-specific views.",
    scope: FeatureScope::Global,
    kind: FeatureKind::Toggle,
}];

impl ProviderFeatureModule for OpenCodeFeatureModule {
    fn provider_id(&self) -> &'static str {
        "opencode"
    }

    fn features(&self) -> &'static [ProviderFeature] {
        FEATURES
    }
}
