//! Provider-owned hook and adapter registry.
//!
//! This module is intentionally under `providers/`: it wires provider-specific
//! `hook.rs` and `adapter.rs` implementations without moving their install,
//! status, repair, uninstall, or payload parsing details into the common
//! `hooks/` infrastructure layer.

use crate::hooks::contract::{HookAdapter, ProviderHook};

static ANTIGRAVITY_ADAPTER: super::antigravity::adapter::AntiGravityHookAdapter =
    super::antigravity::adapter::AntiGravityHookAdapter;
static CLAUDE_ADAPTER: super::claude::adapter::ClaudeHookAdapter =
    super::claude::adapter::ClaudeHookAdapter;
static CODEX_ADAPTER: super::codex::adapter::CodexHookAdapter =
    super::codex::adapter::CodexHookAdapter;
static COPILOT_ADAPTER: super::copilot::adapter::CopilotHookAdapter =
    super::copilot::adapter::CopilotHookAdapter;
static CURSOR_ADAPTER: super::cursor::adapter::CursorHookAdapter =
    super::cursor::adapter::CursorHookAdapter;
static GEMINI_ADAPTER: super::gemini::adapter::GeminiHookAdapter =
    super::gemini::adapter::GeminiHookAdapter;
static KIMI_ADAPTER: super::kimi::adapter::KimiHookAdapter = super::kimi::adapter::KimiHookAdapter;
static KIRO_ADAPTER: super::kiro::adapter::KiroHookAdapter = super::kiro::adapter::KiroHookAdapter;
static OPENCODE_ADAPTER: super::opencode::adapter::OpenCodeHookAdapter =
    super::opencode::adapter::OpenCodeHookAdapter;
static PI_ADAPTER: super::pi::adapter::PiHookAdapter = super::pi::adapter::PiHookAdapter;
static QODER_ADAPTER: super::qoder::adapter::QoderHookAdapter =
    super::qoder::adapter::QoderHookAdapter;
static WORKBUDDY_ADAPTER: super::workbuddy::adapter::WorkBuddyHookAdapter =
    super::workbuddy::adapter::WorkBuddyHookAdapter;

pub fn find_provider_hook(provider: &str) -> Option<&'static dyn ProviderHook> {
    let provider = crate::hooks::profiles::find(provider)?.provider;
    match provider {
        "claude" => Some(&super::claude::hook::CLAUDE_HOOK),
        "codex" => Some(&super::codex::hook::CODEX_HOOK),
        "copilot" => Some(&super::copilot::hook::COPILOT_HOOK),
        "cursor" => Some(&super::cursor::hook::CURSOR_HOOK),
        "antigravity" => Some(&super::antigravity::hook::ANTIGRAVITY_HOOK),
        "workbuddy" => Some(&super::workbuddy::hook::WORKBUDDY_HOOK),
        "gemini" => Some(&super::gemini::hook::GEMINI_HOOK),
        "kiro" => Some(&super::kiro::hook::KIRO_HOOK),
        "kimi" => Some(&super::kimi::hook::KIMI_HOOK),
        "opencode" => Some(&super::opencode::hook::OPENCODE_HOOK),
        "pi" => Some(&super::pi::hook::PI_HOOK),
        "qoder" => Some(&super::qoder::hook::QODER_HOOK),
        _ => None,
    }
}

pub fn find_hook_adapter(provider: &str) -> Option<&'static dyn HookAdapter> {
    let provider = crate::hooks::profiles::find(provider)?.provider;
    match provider {
        "claude" => Some(&CLAUDE_ADAPTER),
        "codex" => Some(&CODEX_ADAPTER),
        "copilot" => Some(&COPILOT_ADAPTER),
        "cursor" => Some(&CURSOR_ADAPTER),
        "antigravity" => Some(&ANTIGRAVITY_ADAPTER),
        "workbuddy" => Some(&WORKBUDDY_ADAPTER),
        "gemini" => Some(&GEMINI_ADAPTER),
        "kiro" => Some(&KIRO_ADAPTER),
        "kimi" => Some(&KIMI_ADAPTER),
        "opencode" => Some(&OPENCODE_ADAPTER),
        "pi" => Some(&PI_ADAPTER),
        "qoder" => Some(&QODER_ADAPTER),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{HookEventType, HookHealthStatus};
    use crate::hooks::protocol::HookIngestRequest;
    use crate::hooks::test_support::TestHookHomeGuard;
    use serde_json::{json, Value};

    fn basic_tool_started_payload(provider: &str) -> (&'static str, Value) {
        match provider {
            "codex" => (
                "PreToolUse",
                json!({"session_id": "s1", "tool_name": "shell"}),
            ),
            "copilot" | "kiro" => (
                "preToolUse",
                json!({"session_id": "s1", "tool_name": "shell"}),
            ),
            "cursor" => (
                "beforeShellExecution",
                json!({"session_id": "s1", "command": "cargo check"}),
            ),
            "gemini" => (
                "BeforeTool",
                json!({"session_id": "s1", "tool_name": "run_shell_command"}),
            ),
            _ => (
                "PreToolUse",
                json!({"session_id": "s1", "tool_name": "Bash"}),
            ),
        }
    }

    #[test]
    fn hook_registry_resolves_profile_aliases() {
        assert_eq!(
            find_provider_hook("claude-code")
                .expect("claude hook")
                .provider_id(),
            "claude"
        );
    }

    #[test]
    fn hook_adapter_registry_resolves_profile_aliases() {
        assert_eq!(
            find_hook_adapter("claude-code")
                .expect("claude adapter")
                .provider_id(),
            "claude"
        );
    }

    #[test]
    fn provider_adapters_normalize_basic_tool_started_events() {
        for descriptor in crate::hooks::registry::all() {
            let provider = descriptor.provider();
            let (event_name, raw) = basic_tool_started_payload(provider);
            let request = HookIngestRequest::new(provider, event_name, raw);
            let events = find_hook_adapter(provider)
                .unwrap_or_else(|| panic!("missing adapter for {provider}"))
                .normalize(&request)
                .unwrap_or_else(|err| panic!("adapter failed for {provider}: {err}"));
            assert_eq!(events[0].provider, provider);
            assert_eq!(
                events[0].event_type,
                HookEventType::ToolStarted,
                "unexpected event type for {provider}"
            );
        }
    }

    #[test]
    fn installs_repairs_and_uninstalls_codeisland_claude_fork_providers() {
        let _home = TestHookHomeGuard::new();
        for provider in [
            "qoder",
            "antigravity",
            "workbuddy",
        ] {
            assert_eq!(
                find_provider_hook(provider)
                    .unwrap()
                    .verify()
                    .unwrap()
                    .status
                    .status,
                HookHealthStatus::NotInstalled
            );

            let installed = find_provider_hook(provider).unwrap().install().unwrap();
            assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
            assert!(installed.changed, "{provider} install should write hooks");

            let verified = find_provider_hook(provider).unwrap().verify().unwrap();
            assert_eq!(verified.status.status, HookHealthStatus::InstalledOk);
            assert_eq!(
                verified.status.installed_version.as_deref(),
                Some(crate::hooks::shared::HOOK_MANAGED_VERSION)
            );

            let removed = find_provider_hook(provider).unwrap().uninstall().unwrap();
            assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
            assert!(removed.changed, "{provider} uninstall should remove hooks");
        }
    }
}
