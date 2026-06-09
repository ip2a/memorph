pub mod claude;
pub mod codex;
pub mod cursor;
pub mod deepseek;
pub mod emerging;
pub mod gemini;
mod generic_json;
pub mod kimi;
pub mod kiro;
pub mod opencode;

use crate::provider::Provider;

const PROVIDER_IDS: &[&str] = &[
    "claude",
    "codex",
    "cursor",
    "opencode",
    "kiro",
    "deepseek",
    "kimi",
    "gemini",
    "antigravity",
    "copilot",
    "windsurf",
    "cidebuddy",
    "qoder",
    "trae",
];

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn ids() -> &'static [&'static str] {
        PROVIDER_IDS
    }

    pub fn find(id: &str) -> Option<Box<dyn Provider>> {
        match id {
            "claude" => Some(Box::new(claude::ClaudeProvider)),
            "codex" => Some(Box::new(codex::CodexProvider)),
            "cursor" => Some(Box::new(cursor::CursorProvider)),
            "deepseek" => Some(Box::new(deepseek::DeepseekProvider)),
            "antigravity" => Some(Box::new(emerging::AntigravityProvider)),
            "copilot" => Some(Box::new(emerging::CopilotProvider)),
            "windsurf" => Some(Box::new(emerging::WindsurfProvider)),
            "cidebuddy" | "codebuddy" => Some(Box::new(emerging::CideBuddyProvider)),
            "qoder" => Some(Box::new(emerging::QoderProvider)),
            "trae" => Some(Box::new(emerging::TraeProvider)),
            "gemini" => Some(Box::new(gemini::GeminiProvider)),
            "kiro" => Some(Box::new(kiro::KiroProvider)),
            "kimi" => Some(Box::new(kimi::KimiProvider)),
            "opencode" => Some(Box::new(opencode::OpenCodeProvider)),
            _ => None,
        }
    }
}

pub fn all_provider_ids() -> &'static [&'static str] {
    ProviderRegistry::ids()
}

/// Find provider by ID.
pub fn find_provider(id: &str) -> Option<Box<dyn Provider>> {
    ProviderRegistry::find(id)
}

/// Default switch target for a given source provider.
pub fn default_switch_target(from: &str) -> &'static str {
    let ids = all_provider_ids();
    match from {
        "codex" => "claude",
        _ => ids
            .iter()
            .find(|&&id| id != from)
            .copied()
            .unwrap_or("codex"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_requested_emerging_providers() {
        for id in [
            "gemini",
            "antigravity",
            "copilot",
            "windsurf",
            "cidebuddy",
            "qoder",
            "trae",
        ] {
            assert!(
                all_provider_ids().iter().any(|known| *known == id),
                "missing provider id: {id}"
            );
            assert!(find_provider(id).is_some(), "provider not found: {id}");
        }
    }

    #[test]
    fn codebuddy_alias_resolves_to_cidebuddy_provider() {
        let provider = find_provider("codebuddy").expect("codebuddy alias");
        assert_eq!(provider.id(), "cidebuddy");
    }
}
