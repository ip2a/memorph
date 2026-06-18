//! Provider event normalization.
//!
//! Provider adapters translate raw provider payloads into canonical hook events.
//! They must not update runtime state or write storage; those responsibilities
//! belong to `runtime` and `store`.

use anyhow::{Context, Result};

use crate::hooks::adapters::antigravity::AntiGravityHookAdapter;
use crate::hooks::adapters::claude::ClaudeHookAdapter;
use crate::hooks::adapters::cline::ClineHookAdapter;
use crate::hooks::adapters::codebuddy::CodeBuddyHookAdapter;
use crate::hooks::adapters::codex::CodexHookAdapter;
use crate::hooks::adapters::codybuddycn::CodyBuddyCnHookAdapter;
use crate::hooks::adapters::copilot::CopilotHookAdapter;
use crate::hooks::adapters::cursor::CursorHookAdapter;
use crate::hooks::adapters::droid::DroidHookAdapter;
use crate::hooks::adapters::gemini::GeminiHookAdapter;
use crate::hooks::adapters::generic::GenericHookAdapter;
use crate::hooks::adapters::hermes::HermesHookAdapter;
use crate::hooks::adapters::kimi::KimiHookAdapter;
use crate::hooks::adapters::kiro::KiroHookAdapter;
use crate::hooks::adapters::omp::OmpHookAdapter;
use crate::hooks::adapters::opencode::OpenCodeHookAdapter;
use crate::hooks::adapters::pi::PiHookAdapter;
use crate::hooks::adapters::qoder::QoderHookAdapter;
use crate::hooks::adapters::qwen::QwenHookAdapter;
use crate::hooks::adapters::stepfun::StepFunHookAdapter;
use crate::hooks::adapters::trae::TraeHookAdapter;
use crate::hooks::adapters::trae_gui::TraeGuiHookAdapter;
use crate::hooks::adapters::traecn::TraeCnHookAdapter;
use crate::hooks::adapters::workbuddy::WorkBuddyHookAdapter;
use crate::hooks::model::HookEvent;
use crate::hooks::protocol::HookIngestRequest;

pub trait HookAdapter: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn normalize(&self, request: &HookIngestRequest) -> Result<Vec<HookEvent>>;
}

static ANTIGRAVITY_ADAPTER: AntiGravityHookAdapter = AntiGravityHookAdapter;
static CLAUDE_ADAPTER: ClaudeHookAdapter = ClaudeHookAdapter;
static CODEBUDDY_ADAPTER: CodeBuddyHookAdapter = CodeBuddyHookAdapter;
static CODYBUDDYCN_ADAPTER: CodyBuddyCnHookAdapter = CodyBuddyCnHookAdapter;
static CLINE_ADAPTER: ClineHookAdapter = ClineHookAdapter;
static CODEX_ADAPTER: CodexHookAdapter = CodexHookAdapter;
static COPILOT_ADAPTER: CopilotHookAdapter = CopilotHookAdapter;
static CURSOR_ADAPTER: CursorHookAdapter = CursorHookAdapter;
static DROID_ADAPTER: DroidHookAdapter = DroidHookAdapter;
static GEMINI_ADAPTER: GeminiHookAdapter = GeminiHookAdapter;
static GENERIC_ADAPTER: GenericHookAdapter = GenericHookAdapter;
static HERMES_ADAPTER: HermesHookAdapter = HermesHookAdapter;
static KIMI_ADAPTER: KimiHookAdapter = KimiHookAdapter;
static KIRO_ADAPTER: KiroHookAdapter = KiroHookAdapter;
static OMP_ADAPTER: OmpHookAdapter = OmpHookAdapter;
static OPENCODE_ADAPTER: OpenCodeHookAdapter = OpenCodeHookAdapter;
static PI_ADAPTER: PiHookAdapter = PiHookAdapter;
static QODER_ADAPTER: QoderHookAdapter = QoderHookAdapter;
static QWEN_ADAPTER: QwenHookAdapter = QwenHookAdapter;
static STEPFUN_ADAPTER: StepFunHookAdapter = StepFunHookAdapter;
static TRAE_ADAPTER: TraeHookAdapter = TraeHookAdapter;
static TRAE_GUI_ADAPTER: TraeGuiHookAdapter = TraeGuiHookAdapter;
static TRAECN_ADAPTER: TraeCnHookAdapter = TraeCnHookAdapter;
static WORKBUDDY_ADAPTER: WorkBuddyHookAdapter = WorkBuddyHookAdapter;

pub fn normalize_request(request: &HookIngestRequest) -> Result<Vec<HookEvent>> {
    let mut events = adapter_for(&request.provider)
        .with_context(|| {
            format!(
                "No hook adapter is registered for provider: {}",
                request.provider
            )
        })?
        .normalize(request)?;
    for event in &mut events {
        if event.terminal_vars.is_empty() && !request.environment.vars.is_empty() {
            event.terminal_vars = request.environment.vars.clone();
        }
    }
    Ok(events)
}

pub fn adapter_for(provider: &str) -> Option<&'static dyn HookAdapter> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "claude" | "claude-code" | "claude_code" => Some(&CLAUDE_ADAPTER),
        "cline" | "cline-agent" | "cline_agent" => Some(&CLINE_ADAPTER),
        "codex" | "openai-codex" | "openai_codex" => Some(&CODEX_ADAPTER),
        "copilot" | "github-copilot" | "github_copilot" => Some(&COPILOT_ADAPTER),
        "cursor" | "cursor-agent" | "cursor_agent" => Some(&CURSOR_ADAPTER),
        "gemini" | "gemini-cli" | "gemini_cli" => Some(&GEMINI_ADAPTER),
        "kimi" | "kimi-cli" | "kimi_cli" => Some(&KIMI_ADAPTER),
        "kiro" | "kiro-cli" | "kiro_cli" => Some(&KIRO_ADAPTER),
        "opencode" | "open-code" | "open_code" => Some(&OPENCODE_ADAPTER),
        "qwen" | "qwen-code" | "qwen_code" => Some(&QWEN_ADAPTER),
        "trae" | "trae-cli" | "trae_cli" | "traecli" => Some(&TRAE_ADAPTER),
        "trae-gui" | "trae_gui" => Some(&TRAE_GUI_ADAPTER),
        "traecn" | "trae-cn" | "trae_cn" => Some(&TRAECN_ADAPTER),
        "qoder" => Some(&QODER_ADAPTER),
        "droid" | "factory" => Some(&DROID_ADAPTER),
        "codebuddy" => Some(&CODEBUDDY_ADAPTER),
        "codybuddycn" | "codybuddy-cn" | "codybuddy_cn" => Some(&CODYBUDDYCN_ADAPTER),
        "stepfun" | "step-fun" | "step_fun" => Some(&STEPFUN_ADAPTER),
        "antigravity" | "anti-gravity" | "anti_gravity" => Some(&ANTIGRAVITY_ADAPTER),
        "workbuddy" | "work-buddy" | "work_buddy" => Some(&WORKBUDDY_ADAPTER),
        "hermes" => Some(&HERMES_ADAPTER),
        "pi" => Some(&PI_ADAPTER),
        "omp" | "oh-my-pi" | "oh_my_pi" => Some(&OMP_ADAPTER),
        "generic" | "custom" | "unknown" => Some(&GENERIC_ADAPTER),
        // Provider-specific adapters will be enabled as they are implemented.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_generic_request() {
        let request = HookIngestRequest::new(
            "generic",
            "tool_started",
            json!({"session_id": "sess-1", "tool": {"name": "shell", "input": "cargo check"}}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].provider, "generic");
        assert_eq!(events[0].provider_session_id.as_deref(), Some("sess-1"));
        assert_eq!(events[0].tool.as_ref().unwrap().name, "shell");
    }

    #[test]
    fn routes_claude_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "claude",
            "PreToolUse",
            json!({"session_id": "s1", "tool_name": "Bash"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "claude");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_cline_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "cline",
            "PreToolUse",
            json!({"task_id": "task-1", "tool_name": "execute_command"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "cline");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_codex_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "codex",
            "PreToolUse",
            json!({"session_id": "s1", "tool_name": "shell"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "codex");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_copilot_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "copilot",
            "preToolUse",
            json!({"session_id": "s1", "tool_name": "shell"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "copilot");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_opencode_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "opencode",
            "PreToolUse",
            json!({"session_id": "s1", "tool_name": "Bash"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "opencode");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_cursor_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "cursor",
            "beforeShellExecution",
            json!({"session_id": "s1", "command": "cargo check"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "cursor");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_gemini_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "gemini",
            "BeforeTool",
            json!({"session_id": "s1", "tool_name": "run_shell_command"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "gemini");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_kimi_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "kimi",
            "PreToolUse",
            json!({"session_id": "s1", "tool_name": "Bash"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "kimi");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_kiro_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "kiro",
            "preToolUse",
            json!({"session_id": "s1", "tool_name": "Bash"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "kiro");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_qwen_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "qwen",
            "PreToolUse",
            json!({"session_id": "s1", "tool_name": "Bash"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "qwen");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_trae_provider_to_first_class_adapter() {
        let request = HookIngestRequest::new(
            "trae",
            "pre_tool_use",
            json!({"session_id": "s1", "tool_name": "Bash"}),
        );
        let events = normalize_request(&request).unwrap();
        assert_eq!(events[0].provider, "trae");
        assert_eq!(
            events[0].event_type,
            crate::hooks::model::HookEventType::ToolStarted
        );
    }

    #[test]
    fn routes_codeisland_gap_providers_to_first_class_adapters() {
        for provider in [
            "qoder",
            "droid",
            "codebuddy",
            "codybuddycn",
            "stepfun",
            "antigravity",
            "workbuddy",
            "hermes",
            "traecn",
            "trae_gui",
            "pi",
            "omp",
        ] {
            let request = HookIngestRequest::new(
                provider,
                "PreToolUse",
                json!({"session_id": "s1", "tool_name": "Bash"}),
            );
            let events = normalize_request(&request).unwrap();
            assert_eq!(events[0].provider, provider);
            assert_eq!(
                events[0].event_type,
                crate::hooks::model::HookEventType::ToolStarted
            );
        }
    }

    #[test]
    fn carries_bridge_terminal_environment_into_normalized_events() {
        let mut request = HookIngestRequest::new(
            "claude",
            "PreToolUse",
            json!({"session_id": "s1", "tool_name": "Bash"}),
        );
        request
            .environment
            .vars
            .insert("TMUX_PANE".to_string(), "%5".to_string());
        request
            .environment
            .vars
            .insert("WEZTERM_PANE".to_string(), "12".to_string());

        let events = normalize_request(&request).unwrap();
        assert_eq!(
            events[0].terminal_vars.get("TMUX_PANE").map(String::as_str),
            Some("%5")
        );
        assert_eq!(
            events[0]
                .terminal_vars
                .get("WEZTERM_PANE")
                .map(String::as_str),
            Some("12")
        );
    }

    #[test]
    fn rejects_unregistered_provider_explicitly() {
        let request = HookIngestRequest::new("missing-provider", "tool_started", json!({}));
        let err = normalize_request(&request).unwrap_err().to_string();
        assert!(err.contains("No hook adapter"));
    }
}
