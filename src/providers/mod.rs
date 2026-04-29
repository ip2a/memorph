pub mod claude;
pub mod codex;
pub mod cursor;
pub mod opencode;

use crate::provider::Provider;

const PROVIDER_IDS: &[&str] = &["claude", "codex", "cursor", "opencode"];

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
