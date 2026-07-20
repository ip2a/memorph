mod aliases;
pub mod antigravity;
pub mod catalog;
pub mod claude;
pub mod cline;
pub mod codebuddy;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod deepseek;
pub mod droid;
pub mod emerging;
pub(crate) mod environment_profiles;
pub mod gemini;
mod generic_json;
pub mod hermes;
pub mod kimi;
pub mod kiro;
pub mod opencode;
pub mod pi;
pub mod qoder;
pub mod qwen;
pub mod stepfun;
pub mod trae;
pub mod workbuddy;

pub(crate) mod hook_profiles;
mod hook_registry;

use crate::provider::Provider;

const PROVIDER_IDS: &[&str] = &[
    "claude",
    "codex",
    "cline",
    "cursor",
    "opencode",
    "kiro",
    "deepseek",
    "kimi",
    "gemini",
    "antigravity",
    "copilot",
    "windsurf",
    "codebuddy",
    "qoder",
    "qwen",
    "trae",
    "droid",
    "stepfun",
    "workbuddy",
    "hermes",
    "pi",
];

pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn ids() -> &'static [&'static str] {
        PROVIDER_IDS
    }

    pub fn find(id: &str) -> Option<Box<dyn Provider>> {
        let id = aliases::canonical_provider_id(id);
        match id.as_str() {
            "claude" => Some(Box::new(claude::ClaudeProvider)),
            "codex" => Some(Box::new(codex::CodexProvider)),
            "cline" => Some(Box::new(cline::ClineProvider)),
            "cursor" => Some(Box::new(cursor::CursorProvider)),
            "deepseek" => Some(Box::new(deepseek::DeepseekProvider)),
            "antigravity" => Some(Box::new(emerging::AntigravityProvider)),
            "copilot" => Some(Box::new(copilot::CopilotProvider)),
            "windsurf" => Some(Box::new(emerging::WindsurfProvider)),
            "codebuddy" => Some(Box::new(emerging::CodeBuddyProvider)),
            "qoder" => Some(Box::new(emerging::QoderProvider)),
            "qwen" => Some(Box::new(qwen::QwenProvider)),
            "trae" => Some(Box::new(emerging::TraeProvider)),
            "droid" => Some(Box::new(emerging::DroidProvider)),
            "stepfun" => Some(Box::new(emerging::StepFunProvider)),
            "workbuddy" => Some(Box::new(emerging::WorkBuddyProvider)),
            "hermes" => Some(Box::new(hermes::HermesProvider)),
            "pi" => Some(Box::new(emerging::PiProvider)),
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

pub(crate) fn canonical_provider_id(provider_id: &str) -> String {
    aliases::canonical_provider_id(provider_id)
}

/// Find provider-owned hook management implementation by ID or hook profile alias.
pub fn find_provider_hook(
    provider: &str,
) -> Option<&'static dyn crate::hooks::contract::ProviderHook> {
    hook_registry::find_provider_hook(provider)
}

/// Find provider-owned hook payload adapter by ID or hook profile alias.
pub fn find_hook_adapter(
    provider: &str,
) -> Option<&'static dyn crate::hooks::contract::HookAdapter> {
    hook_registry::find_hook_adapter(provider)
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
            "cline",
            "copilot",
            "windsurf",
            "codebuddy",
            "qoder",
            "qwen",
            "trae",
            "droid",
            "stepfun",
            "workbuddy",
            "hermes",
            "pi",
        ] {
            assert!(
                all_provider_ids().iter().any(|known| *known == id),
                "missing provider id: {id}"
            );
            assert!(find_provider(id).is_some(), "provider not found: {id}");
        }
    }

    #[test]
    fn registry_uses_native_qwen_jsonl_provider() {
        let provider = find_provider("qwen").expect("qwen provider");
        let capabilities = provider.capabilities();
        assert_eq!(provider.name(), "Qwen Code");
        assert_eq!(
            capabilities.storage_shape,
            crate::provider::StorageShape::Jsonl
        );
        assert!(capabilities.resume);
        assert!(capabilities.delete);
        assert!(capabilities.rename);
        assert!(!capabilities.export);
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert_eq!(
            capabilities.write_risk.level,
            crate::provider::WriteRiskLevel::High
        );
        assert!(capabilities.write_risk.multiple_files);
        assert!(capabilities.write_risk.sidecar_files);
        assert!(!capabilities.write_risk.sqlite);
    }

    #[test]
    fn registry_uses_native_hermes_sqlite_provider() {
        let provider = find_provider("hermes").expect("hermes provider");
        let capabilities = provider.capabilities();
        assert_eq!(
            capabilities.storage_shape,
            crate::provider::StorageShape::Sqlite
        );
        assert_eq!(
            capabilities.page_strategy,
            crate::provider::PageStrategy::FullImport
        );
        assert!(capabilities.resume);
        assert!(!capabilities.delete);
        assert!(!capabilities.rename);
        assert!(!capabilities.export);
    }

    #[test]
    fn factory_alias_resolves_to_droid_provider() {
        let provider = find_provider("factory").expect("factory alias");
        assert_eq!(provider.id(), "droid");
    }
}
