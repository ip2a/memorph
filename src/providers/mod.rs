pub mod claude;
pub mod codex;
pub mod opencode;

use crate::provider::Provider;

/// Find provider by ID
pub fn find_provider(id: &str) -> Option<Box<dyn Provider>> {
    match id {
        "claude" => Some(Box::new(claude::ClaudeProvider)),
        "codex" => Some(Box::new(codex::CodexProvider)),
        "opencode" => Some(Box::new(opencode::OpenCodeProvider)),
        _ => None,
    }
}

/// Get the resume command for a provider
pub fn resume_command(provider_id: &str, session_id: &str) -> Option<String> {
    match provider_id {
        "claude" => Some(format!("claude --resume {}", session_id)),
        "codex" => Some(format!("codex resume {}", session_id)),
        "opencode" => Some(format!("opencode --session {}", session_id)),
        _ => None,
    }
}
