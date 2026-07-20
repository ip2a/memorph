//! Provider-owned hook profile inventory.
//!
//! Provider-specific hook event names, config hints, storage formats, and aliases
//! live here so the common `hooks/` layer can expose metadata without owning
//! provider details.

use crate::hooks::profiles::{HookFormat, HookProviderEventProfile, HookProviderProfile};
use crate::hooks::strategies::HookConfigStrategyKind;

const CLAUDE_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", false),
    event("PreToolUse", true),
    event("PostToolUse", false),
    event("PostToolUseFailure", false),
    event("PermissionRequest", true),
    event("Stop", false),
    event("SubagentStart", false),
    event("SubagentStop", false),
    event("SessionStart", false),
    event("SessionEnd", false),
    event("Notification", false),
    event("PreCompact", false),
];

const CLINE_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", false),
    event("PreToolUse", true),
    event("PostToolUse", false),
    event("TaskStart", false),
    event("TaskResume", false),
    event("TaskCancel", false),
    event("TaskComplete", false),
];

const CODEX_EVENTS: &[HookProviderEventProfile] = &[
    event("SessionStart", false),
    event("SessionEnd", false),
    event("UserPromptSubmit", false),
    event("PreToolUse", false),
    event("PostToolUse", false),
    event("PermissionRequest", true),
    event("Stop", false),
];

const CURSOR_EVENTS: &[HookProviderEventProfile] = &[
    event("beforeSubmitPrompt", false),
    event("beforeShellExecution", false),
    event("afterShellExecution", false),
    event("beforeReadFile", false),
    event("afterFileEdit", false),
    event("beforeMCPExecution", false),
    event("afterMCPExecution", false),
    event("afterAgentThought", false),
    event("afterAgentResponse", false),
    event("stop", false),
];

const GEMINI_EVENTS: &[HookProviderEventProfile] = &[
    event("SessionStart", false),
    event("SessionEnd", false),
    event("BeforeTool", false),
    event("AfterTool", false),
    event("BeforeAgent", false),
    event("AfterAgent", false),
];

const KIMI_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("PostToolUseFailure", true),
    event("Stop", true),
    event("SubagentStart", true),
    event("SubagentStop", true),
    event("SessionStart", false),
    event("SessionEnd", true),
    event("Notification", false),
    event("PreCompact", true),
];

const KIRO_EVENTS: &[HookProviderEventProfile] = &[
    event("agentSpawn", false),
    event("userPromptSubmit", true),
    event("preToolUse", false),
    event("postToolUse", true),
    event("stop", true),
];

const COPILOT_EVENTS: &[HookProviderEventProfile] = &[
    event("sessionStart", false),
    event("sessionEnd", true),
    event("userPromptSubmitted", false),
    event("preToolUse", false),
    event("postToolUse", true),
    event("errorOccurred", true),
];

const QWEN_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("PostToolUseFailure", true),
    event("PermissionRequest", false),
    event("Stop", true),
    event("SubagentStart", true),
    event("SubagentStop", true),
    event("SessionStart", false),
    event("SessionEnd", true),
    event("Notification", false),
    event("PreCompact", true),
];

const QODER_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("SessionStart", false),
    event("SessionEnd", true),
    event("Stop", true),
    event("SubagentStart", true),
    event("SubagentStop", true),
    event("Notification", false),
    event("PreCompact", true),
];
const FACTORY_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("SessionStart", false),
    event("SessionEnd", true),
    event("Stop", true),
    event("SubagentStart", true),
    event("SubagentStop", true),
    event("Notification", false),
    event("PreCompact", true),
];
const CODEBUDDY_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("SessionStart", false),
    event("SessionEnd", true),
    event("Stop", true),
    event("SubagentStart", true),
    event("SubagentStop", true),
    event("Notification", false),
    event("PreCompact", true),
];
const ANTIGRAVITY_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("SessionStart", false),
    event("SessionEnd", true),
    event("Stop", true),
    event("SubagentStart", true),
    event("SubagentStop", true),
    event("Notification", false),
    event("PreCompact", true),
];
const WORKBUDDY_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("SessionStart", false),
    event("SessionEnd", true),
    event("Stop", true),
    event("SubagentStart", true),
    event("SubagentStop", true),
    event("Notification", false),
    event("PreCompact", true),
];
const HERMES_EVENTS: &[HookProviderEventProfile] = &[
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("SessionStart", false),
    event("SessionEnd", true),
    event("Stop", true),
    event("SubagentStart", true),
    event("SubagentStop", true),
    event("Notification", false),
    event("PreCompact", true),
];
const TRAE_EVENTS: &[HookProviderEventProfile] = &[
    event("session_start", false),
    event("session_end", false),
    event("user_prompt_submit", false),
    event("pre_tool_use", false),
    event("post_tool_use", false),
    event("post_tool_use_failure", false),
    event("permission_request", true),
    event("notification", false),
    event("subagent_start", false),
    event("subagent_stop", false),
    event("stop", false),
    event("pre_compact", false),
    event("post_compact", false),
];

const PI_EVENTS: &[HookProviderEventProfile] = &[
    event("SessionStart", false),
    event("SessionEnd", true),
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("PermissionRequest", false),
    event("Stop", true),
];

const OPENCODE_EVENTS: &[HookProviderEventProfile] = &[
    event("SessionStart", false),
    event("SessionEnd", false),
    event("UserPromptSubmit", false),
    event("PreToolUse", false),
    event("PostToolUse", false),
    event("PermissionRequest", false),
    event("QuestionRequest", false),
    event("Stop", false),
];

const PROFILES: &[HookProviderProfile] = &[
    profile(
        "claude",
        "Claude Code",
        HookFormat::ClaudeNestedJson,
        HookConfigStrategyKind::ClaudeLikeJson,
        "~/.claude/settings.json",
        CLAUDE_EVENTS,
    ),
    profile(
        "cline",
        "Cline",
        HookFormat::ClineFiles,
        HookConfigStrategyKind::ClineFiles,
        "~/Documents/Cline/Rules/Hooks or ~/Documents/Cline/Hooks",
        CLINE_EVENTS,
    ),
    profile(
        "codex",
        "Codex",
        HookFormat::CodexJson,
        HookConfigStrategyKind::CodexJson,
        "$CODEX_HOME/hooks.json",
        CODEX_EVENTS,
    ),
    profile(
        "copilot",
        "GitHub Copilot",
        HookFormat::CopilotJson,
        HookConfigStrategyKind::CopilotJson,
        "~/.copilot/hooks/memorph.json",
        COPILOT_EVENTS,
    ),
    profile(
        "cursor",
        "Cursor",
        HookFormat::CursorFlatJson,
        HookConfigStrategyKind::FlatJson,
        "~/.cursor/hooks.json",
        CURSOR_EVENTS,
    ),
    profile(
        "gemini",
        "Gemini CLI",
        HookFormat::GeminiNestedJson,
        HookConfigStrategyKind::GeminiNestedJson,
        "~/.gemini/settings.json",
        GEMINI_EVENTS,
    ),
    profile(
        "kimi",
        "Kimi Code CLI",
        HookFormat::KimiToml,
        HookConfigStrategyKind::KimiToml,
        "~/.kimi/config.toml",
        KIMI_EVENTS,
    ),
    profile(
        "kiro",
        "Kiro",
        HookFormat::KiroAgentJson,
        HookConfigStrategyKind::KiroAgentJson,
        "~/.kiro/agents/memorph.json",
        KIRO_EVENTS,
    ),
    profile(
        "opencode",
        "OpenCode",
        HookFormat::OpenCodePlugin,
        HookConfigStrategyKind::OpenCodePlugin,
        "~/.config/opencode/plugins/memorph.js",
        OPENCODE_EVENTS,
    ),
    profile(
        "qwen",
        "Qwen Code",
        HookFormat::QwenNestedJson,
        HookConfigStrategyKind::ClaudeLikeJson,
        "~/.qwen/settings.json",
        QWEN_EVENTS,
    ),
    profile(
        "qoder",
        "Qoder",
        HookFormat::QoderClaudeJson,
        HookConfigStrategyKind::ClaudeLikeJson,
        "~/.qoder/settings.json",
        QODER_EVENTS,
    ),
    profile(
        "droid",
        "Factory",
        HookFormat::FactoryClaudeJson,
        HookConfigStrategyKind::ClaudeLikeJson,
        "~/.factory/settings.json",
        FACTORY_EVENTS,
    ),
    profile(
        "codebuddy",
        "CodeBuddy",
        HookFormat::CodeBuddyClaudeJson,
        HookConfigStrategyKind::ClaudeLikeJson,
        "~/.codebuddy/settings.json",
        CODEBUDDY_EVENTS,
    ),
    profile(
        "antigravity",
        "AntiGravity",
        HookFormat::AntiGravityClaudeJson,
        HookConfigStrategyKind::ClaudeLikeJson,
        "~/.antigravity/settings.json",
        ANTIGRAVITY_EVENTS,
    ),
    profile(
        "workbuddy",
        "WorkBuddy",
        HookFormat::WorkBuddyClaudeJson,
        HookConfigStrategyKind::ClaudeLikeJson,
        "~/.workbuddy/settings.json",
        WORKBUDDY_EVENTS,
    ),
    profile(
        "hermes",
        "Hermes",
        HookFormat::HermesClaudeJson,
        HookConfigStrategyKind::ClaudeLikeJson,
        "~/.hermes/settings.json",
        HERMES_EVENTS,
    ),
    profile(
        "pi",
        "pi",
        HookFormat::PiExtension,
        HookConfigStrategyKind::PiExtension,
        "~/.pi/agent/extensions/memorph.ts",
        PI_EVENTS,
    ),
    profile(
        "trae",
        "TraeCli",
        HookFormat::TraeYaml,
        HookConfigStrategyKind::TraeYaml,
        "~/.trae/traecli.yaml",
        TRAE_EVENTS,
    ),
];

const fn event(name: &'static str, blocking: bool) -> HookProviderEventProfile {
    HookProviderEventProfile { name, blocking }
}

const fn profile(
    provider: &'static str,
    display_name: &'static str,
    format: HookFormat,
    strategy_kind: HookConfigStrategyKind,
    config_hint: &'static str,
    events: &'static [HookProviderEventProfile],
) -> HookProviderProfile {
    HookProviderProfile {
        provider,
        display_name,
        format,
        strategy_kind,
        config_hint,
        events,
    }
}

pub(crate) fn all() -> &'static [HookProviderProfile] {
    PROFILES
}

pub(crate) fn provider_ids() -> impl Iterator<Item = &'static str> {
    PROFILES.iter().map(|profile| profile.provider)
}

pub(crate) fn find(provider: &str) -> Option<&'static HookProviderProfile> {
    let provider = normalize_provider_id(provider);
    PROFILES.iter().find(|profile| profile.provider == provider)
}

fn normalize_provider_id(provider: &str) -> String {
    let provider = provider.trim().to_ascii_lowercase();
    match provider.as_str() {
        "traecli" | "trae-cli" | "trae_cli" => "trae".to_string(),
        "claude-code" | "claude_code" => "claude".to_string(),
        "qwen-code" | "qwen_code" => "qwen".to_string(),
        "gemini-cli" | "gemini_cli" => "gemini".to_string(),
        "cline-agent" | "cline_agent" => "cline".to_string(),
        other => crate::providers::canonical_provider_id(other),
    }
}

pub(crate) fn supports_provider(provider: &str) -> bool {
    find(provider).is_some()
}

pub(crate) fn event_names(profile: &HookProviderProfile) -> Vec<&'static str> {
    profile.events.iter().map(|event| event.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_registry_exposes_supported_hook_providers() {
        for provider in [
            "claude",
            "cline",
            "codex",
            "opencode",
            "qwen",
            "trae",
            "qoder",
            "droid",
            "codebuddy",
            "antigravity",
            "workbuddy",
            "hermes",
            "pi",
        ] {
            assert!(supports_provider(provider), "missing profile: {provider}");
        }
    }

    #[test]
    fn profiles_have_events_and_config_hints() {
        for profile in all() {
            assert!(
                !profile.events.is_empty(),
                "{} has no events",
                profile.provider
            );
            assert!(!profile.config_hint.trim().is_empty());
        }
    }

    #[test]
    fn accepts_provider_aliases() {
        assert_eq!(find("claude-code").unwrap().provider, "claude");
        assert_eq!(find("traecli").unwrap().provider, "trae");
        assert_eq!(find("factory").unwrap().provider, "droid");
    }

    #[test]
    fn codeisland_provider_sources_are_registered_in_memorph_profiles() {
        let codeisland_sources = [
            "claude",
            "codex",
            "gemini",
            "cursor",
            "trae",
            "traecli",
            "qoder",
            "droid",
            "codebuddy",
            "antigravity",
            "workbuddy",
            "hermes",
            "qwen",
            "copilot",
            "kimi",
            "kiro",
            "cline",
            "pi",
        ];

        for source in codeisland_sources {
            if source.starts_with("codeisland-") {
                continue;
            }
            let provider = match source {
                "traecli" => "trae",
                other => other,
            };
            assert!(
                supports_provider(provider),
                "CodeIsland provider source is not registered in memorph: {source} -> {provider}"
            );
        }
        assert!(supports_provider("opencode"));
    }
}
