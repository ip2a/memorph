//! Hook provider profile registry.
//!
//! Profiles are the provider-neutral inventory of hook capabilities. They keep
//! provider coverage, hook storage format, and required events in one place so
//! doctor/diagnostics/UI can reason about supported providers without scraping
//! installer implementation details.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookFormat {
    ClaudeNestedJson,
    CodexJson,
    CursorFlatJson,
    GeminiNestedJson,
    CopilotJson,
    KimiToml,
    KiroAgentJson,
    OpenCodePlugin,
    QwenNestedJson,
    TraeFlatJson,
    TraeCnFlatJson,
    TraeYaml,
    QoderClaudeJson,
    FactoryClaudeJson,
    CodeBuddyClaudeJson,
    CodyBuddyCnClaudeJson,
    StepFunClaudeJson,
    AntiGravityClaudeJson,
    WorkBuddyClaudeJson,
    HermesClaudeJson,
    PiExtension,
    OmpExtension,
    ClineFiles,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct HookProviderEventProfile {
    pub name: &'static str,
    pub blocking: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct HookProviderProfile {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub format: HookFormat,
    pub config_hint: &'static str,
    pub events: &'static [HookProviderEventProfile],
}

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
const CODYBUDDYCN_EVENTS: &[HookProviderEventProfile] = &[
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
const STEPFUN_EVENTS: &[HookProviderEventProfile] = &[
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

const TRAE_GUI_EVENTS: &[HookProviderEventProfile] = &[
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

const TRAECN_EVENTS: &[HookProviderEventProfile] = &[
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

const PI_EVENTS: &[HookProviderEventProfile] = &[
    event("SessionStart", false),
    event("SessionEnd", true),
    event("UserPromptSubmit", true),
    event("PreToolUse", false),
    event("PostToolUse", true),
    event("PermissionRequest", false),
    event("Stop", true),
];

const OMP_EVENTS: &[HookProviderEventProfile] = &[
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
        "~/.claude/settings.json",
        CLAUDE_EVENTS,
    ),
    profile(
        "cline",
        "Cline",
        HookFormat::ClineFiles,
        "~/Documents/Cline/Rules/Hooks or ~/Documents/Cline/Hooks",
        CLINE_EVENTS,
    ),
    profile(
        "codex",
        "Codex",
        HookFormat::CodexJson,
        "$CODEX_HOME/hooks.json",
        CODEX_EVENTS,
    ),
    profile(
        "copilot",
        "GitHub Copilot",
        HookFormat::CopilotJson,
        "~/.copilot/hooks/memorph.json",
        COPILOT_EVENTS,
    ),
    profile(
        "cursor",
        "Cursor",
        HookFormat::CursorFlatJson,
        "~/.cursor/hooks.json",
        CURSOR_EVENTS,
    ),
    profile(
        "gemini",
        "Gemini CLI",
        HookFormat::GeminiNestedJson,
        "~/.gemini/settings.json",
        GEMINI_EVENTS,
    ),
    profile(
        "kimi",
        "Kimi Code CLI",
        HookFormat::KimiToml,
        "~/.kimi/config.toml",
        KIMI_EVENTS,
    ),
    profile(
        "kiro",
        "Kiro",
        HookFormat::KiroAgentJson,
        "~/.kiro/agents/memorph.json",
        KIRO_EVENTS,
    ),
    profile(
        "opencode",
        "OpenCode",
        HookFormat::OpenCodePlugin,
        "~/.config/opencode/plugins/memorph.js",
        OPENCODE_EVENTS,
    ),
    profile(
        "qwen",
        "Qwen Code",
        HookFormat::QwenNestedJson,
        "~/.qwen/settings.json",
        QWEN_EVENTS,
    ),
    profile(
        "trae_gui",
        "Trae",
        HookFormat::TraeFlatJson,
        "~/.trae/hooks.json",
        TRAE_GUI_EVENTS,
    ),
    profile(
        "traecn",
        "Trae CN",
        HookFormat::TraeCnFlatJson,
        "~/.trae-cn/hooks.json",
        TRAECN_EVENTS,
    ),
    profile(
        "qoder",
        "Qoder",
        HookFormat::QoderClaudeJson,
        "~/.qoder/settings.json",
        QODER_EVENTS,
    ),
    profile(
        "droid",
        "Factory",
        HookFormat::FactoryClaudeJson,
        "~/.factory/settings.json",
        FACTORY_EVENTS,
    ),
    profile(
        "codebuddy",
        "CodeBuddy",
        HookFormat::CodeBuddyClaudeJson,
        "~/.codebuddy/settings.json",
        CODEBUDDY_EVENTS,
    ),
    profile(
        "codybuddycn",
        "CodyBuddyCN",
        HookFormat::CodyBuddyCnClaudeJson,
        "~/.codybuddycn/settings.json",
        CODYBUDDYCN_EVENTS,
    ),
    profile(
        "stepfun",
        "StepFun",
        HookFormat::StepFunClaudeJson,
        "~/.stepfun/settings.json",
        STEPFUN_EVENTS,
    ),
    profile(
        "antigravity",
        "AntiGravity",
        HookFormat::AntiGravityClaudeJson,
        "~/.antigravity/settings.json",
        ANTIGRAVITY_EVENTS,
    ),
    profile(
        "workbuddy",
        "WorkBuddy",
        HookFormat::WorkBuddyClaudeJson,
        "~/.workbuddy/settings.json",
        WORKBUDDY_EVENTS,
    ),
    profile(
        "hermes",
        "Hermes",
        HookFormat::HermesClaudeJson,
        "~/.hermes/settings.json",
        HERMES_EVENTS,
    ),
    profile(
        "pi",
        "pi",
        HookFormat::PiExtension,
        "~/.pi/agent/extensions/memorph.ts",
        PI_EVENTS,
    ),
    profile(
        "omp",
        "Oh My Pi",
        HookFormat::OmpExtension,
        "~/.omp/agent/extensions/memorph.ts",
        OMP_EVENTS,
    ),
    profile(
        "trae",
        "TraeCli",
        HookFormat::TraeYaml,
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
    config_hint: &'static str,
    events: &'static [HookProviderEventProfile],
) -> HookProviderProfile {
    HookProviderProfile {
        provider,
        display_name,
        format,
        config_hint,
        events,
    }
}

pub fn all() -> &'static [HookProviderProfile] {
    PROFILES
}

pub fn provider_ids() -> impl Iterator<Item = &'static str> {
    PROFILES.iter().map(|profile| profile.provider)
}

pub fn find(provider: &str) -> Option<&'static HookProviderProfile> {
    let provider = normalize_provider_id(provider);
    PROFILES.iter().find(|profile| profile.provider == provider)
}

pub fn supports_provider(provider: &str) -> bool {
    find(provider).is_some()
}

pub fn event_names(profile: &HookProviderProfile) -> Vec<&'static str> {
    profile.events.iter().map(|event| event.name).collect()
}

fn normalize_provider_id(provider: &str) -> String {
    match provider.trim().to_ascii_lowercase().as_str() {
        "traecli" | "trae-cli" | "trae_cli" => "trae".to_string(),
        "trae-gui" | "trae_gui" => "trae_gui".to_string(),
        "claude-code" | "claude_code" => "claude".to_string(),
        "qwen-code" | "qwen_code" => "qwen".to_string(),
        "factory" => "droid".to_string(),
        "oh-my-pi" | "oh_my_pi" => "omp".to_string(),
        "gemini-cli" | "gemini_cli" => "gemini".to_string(),
        "cline-agent" | "cline_agent" => "cline".to_string(),
        other => other.to_string(),
    }
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
            "trae_gui",
            "traecn",
            "qoder",
            "droid",
            "codebuddy",
            "codybuddycn",
            "stepfun",
            "antigravity",
            "workbuddy",
            "hermes",
            "pi",
            "omp",
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
}
