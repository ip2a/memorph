//! Provider hook installation and removal.
//!
//! The first implemented provider is Claude Code. The installer uses the same
//! Claude hook shape as CodeIsland: `hooks.{Event}[]` entries containing a
//! command hook with matcher `*`.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::sync::{OnceLock, RwLock};

use crate::hooks::health;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus};
use crate::storage::atomic_write;

const HOOK_COMMAND_MARKER: &str = "__hook-bridge";
const HOOK_MANAGED_VERSION: &str = "hook-v1";
const SETTINGS_BACKUP_SUFFIX: &str = "memorph-hook-backup";
const OPENCODE_PLUGIN_FILE: &str = "memorph.js";
const OPENCODE_PLUGIN_MARKER: &str = "memorph-opencode-hook-plugin";
const OPENCODE_PLUGIN_VERSION: &str = "v1";
const CLINE_HOOK_MARKER: &str = "memorph-cline-hook";
const CLINE_HOOK_VERSION: &str = "v1";
const PI_EXTENSION_MARKER: &str = "memorph pi extension";
const PI_EXTENSION_VERSION: &str = HOOK_MANAGED_VERSION;
const OMP_EXTENSION_MARKER: &str = "memorph omp extension";
const OMP_EXTENSION_VERSION: &str = HOOK_MANAGED_VERSION;

#[cfg(test)]
static TEST_HOME_DIR: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

fn hook_home_dir() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(path) = TEST_HOME_DIR
            .get_or_init(|| RwLock::new(None))
            .read()
            .unwrap()
            .clone()
        {
            return path;
        }
    }

    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
fn set_test_home_dir(path: Option<PathBuf>) {
    *TEST_HOME_DIR
        .get_or_init(|| RwLock::new(None))
        .write()
        .unwrap() = path;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClineHookEvent {
    name: &'static str,
    blocking: bool,
}

const CLINE_EVENTS: &[ClineHookEvent] = &[
    ClineHookEvent {
        name: "UserPromptSubmit",
        blocking: false,
    },
    ClineHookEvent {
        name: "PreToolUse",
        blocking: true,
    },
    ClineHookEvent {
        name: "PostToolUse",
        blocking: false,
    },
    ClineHookEvent {
        name: "TaskStart",
        blocking: false,
    },
    ClineHookEvent {
        name: "TaskResume",
        blocking: false,
    },
    ClineHookEvent {
        name: "TaskCancel",
        blocking: false,
    },
    ClineHookEvent {
        name: "TaskComplete",
        blocking: false,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookOperationReport {
    pub provider: String,
    pub operation: String,
    pub changed: bool,
    pub status: HookInstallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClaudeHookEvent {
    name: &'static str,
    timeout: u64,
    blocking: bool,
}

const CLAUDE_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUseFailure",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PermissionRequest",
        timeout: 86400,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: false,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodexHookEvent {
    name: &'static str,
    timeout: u64,
    blocking: bool,
}

const CODEX_EVENTS: &[CodexHookEvent] = &[
    CodexHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    CodexHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: false,
    },
    CodexHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: false,
    },
    CodexHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    CodexHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: false,
    },
    CodexHookEvent {
        name: "PermissionRequest",
        timeout: 86400,
        blocking: true,
    },
    CodexHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: false,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CursorHookEvent {
    name: &'static str,
    blocking: bool,
}

const CURSOR_EVENTS: &[CursorHookEvent] = &[
    CursorHookEvent {
        name: "beforeSubmitPrompt",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeShellExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterShellExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeReadFile",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterFileEdit",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeMCPExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterMCPExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterAgentThought",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterAgentResponse",
        blocking: false,
    },
    CursorHookEvent {
        name: "stop",
        blocking: false,
    },
];

const TRAE_GUI_EVENTS: &[CursorHookEvent] = &[
    CursorHookEvent {
        name: "beforeSubmitPrompt",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeShellExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterShellExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeReadFile",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterFileEdit",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeMCPExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterMCPExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterAgentThought",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterAgentResponse",
        blocking: false,
    },
    CursorHookEvent {
        name: "stop",
        blocking: false,
    },
];

const TRAECN_EVENTS: &[CursorHookEvent] = &[
    CursorHookEvent {
        name: "beforeSubmitPrompt",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeShellExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterShellExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeReadFile",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterFileEdit",
        blocking: false,
    },
    CursorHookEvent {
        name: "beforeMCPExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterMCPExecution",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterAgentThought",
        blocking: false,
    },
    CursorHookEvent {
        name: "afterAgentResponse",
        blocking: false,
    },
    CursorHookEvent {
        name: "stop",
        blocking: false,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GeminiHookEvent {
    name: &'static str,
    timeout: u64,
    blocking: bool,
}

const GEMINI_EVENTS: &[GeminiHookEvent] = &[
    GeminiHookEvent {
        name: "SessionStart",
        timeout: 10000,
        blocking: false,
    },
    GeminiHookEvent {
        name: "SessionEnd",
        timeout: 10000,
        blocking: false,
    },
    GeminiHookEvent {
        name: "BeforeTool",
        timeout: 10000,
        blocking: false,
    },
    GeminiHookEvent {
        name: "AfterTool",
        timeout: 10000,
        blocking: false,
    },
    GeminiHookEvent {
        name: "BeforeAgent",
        timeout: 10000,
        blocking: false,
    },
    GeminiHookEvent {
        name: "AfterAgent",
        timeout: 10000,
        blocking: false,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KimiHookEvent {
    name: &'static str,
    timeout: u64,
    matcher: Option<&'static str>,
    blocking: bool,
}

const KIMI_EVENTS: &[KimiHookEvent] = &[
    KimiHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        matcher: None,
        blocking: true,
    },
    KimiHookEvent {
        name: "PreToolUse",
        timeout: 5,
        matcher: Some(".*"),
        blocking: false,
    },
    KimiHookEvent {
        name: "PostToolUse",
        timeout: 5,
        matcher: Some(".*"),
        blocking: true,
    },
    KimiHookEvent {
        name: "PostToolUseFailure",
        timeout: 5,
        matcher: Some(".*"),
        blocking: true,
    },
    KimiHookEvent {
        name: "Stop",
        timeout: 5,
        matcher: None,
        blocking: true,
    },
    KimiHookEvent {
        name: "SubagentStart",
        timeout: 5,
        matcher: None,
        blocking: true,
    },
    KimiHookEvent {
        name: "SubagentStop",
        timeout: 5,
        matcher: None,
        blocking: true,
    },
    KimiHookEvent {
        name: "SessionStart",
        timeout: 5,
        matcher: None,
        blocking: false,
    },
    KimiHookEvent {
        name: "SessionEnd",
        timeout: 5,
        matcher: None,
        blocking: true,
    },
    KimiHookEvent {
        name: "Notification",
        timeout: 600,
        matcher: None,
        blocking: false,
    },
    KimiHookEvent {
        name: "PreCompact",
        timeout: 5,
        matcher: None,
        blocking: true,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KiroHookEvent {
    name: &'static str,
    timeout_ms: u64,
    blocking: bool,
}

const KIRO_EVENTS: &[KiroHookEvent] = &[
    KiroHookEvent {
        name: "agentSpawn",
        timeout_ms: 5000,
        blocking: false,
    },
    KiroHookEvent {
        name: "userPromptSubmit",
        timeout_ms: 5000,
        blocking: true,
    },
    KiroHookEvent {
        name: "preToolUse",
        timeout_ms: 5000,
        blocking: false,
    },
    KiroHookEvent {
        name: "postToolUse",
        timeout_ms: 5000,
        blocking: true,
    },
    KiroHookEvent {
        name: "stop",
        timeout_ms: 5000,
        blocking: true,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CopilotHookEvent {
    name: &'static str,
    timeout_sec: u64,
    blocking: bool,
}

const COPILOT_EVENTS: &[CopilotHookEvent] = &[
    CopilotHookEvent {
        name: "sessionStart",
        timeout_sec: 5,
        blocking: false,
    },
    CopilotHookEvent {
        name: "sessionEnd",
        timeout_sec: 5,
        blocking: true,
    },
    CopilotHookEvent {
        name: "userPromptSubmitted",
        timeout_sec: 5,
        blocking: false,
    },
    CopilotHookEvent {
        name: "preToolUse",
        timeout_sec: 5,
        blocking: false,
    },
    CopilotHookEvent {
        name: "postToolUse",
        timeout_sec: 5,
        blocking: true,
    },
    CopilotHookEvent {
        name: "errorOccurred",
        timeout_sec: 5,
        blocking: true,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QwenHookEvent {
    name: &'static str,
    timeout: u64,
    blocking: bool,
}

const QWEN_EVENTS: &[QwenHookEvent] = &[
    QwenHookEvent {
        name: "UserPromptSubmit",
        timeout: 5000,
        blocking: true,
    },
    QwenHookEvent {
        name: "PreToolUse",
        timeout: 5000,
        blocking: false,
    },
    QwenHookEvent {
        name: "PostToolUse",
        timeout: 5000,
        blocking: true,
    },
    QwenHookEvent {
        name: "PostToolUseFailure",
        timeout: 5000,
        blocking: true,
    },
    QwenHookEvent {
        name: "PermissionRequest",
        timeout: 86400000,
        blocking: false,
    },
    QwenHookEvent {
        name: "Stop",
        timeout: 5000,
        blocking: true,
    },
    QwenHookEvent {
        name: "SubagentStart",
        timeout: 5000,
        blocking: true,
    },
    QwenHookEvent {
        name: "SubagentStop",
        timeout: 5000,
        blocking: true,
    },
    QwenHookEvent {
        name: "SessionStart",
        timeout: 5000,
        blocking: false,
    },
    QwenHookEvent {
        name: "SessionEnd",
        timeout: 5000,
        blocking: true,
    },
    QwenHookEvent {
        name: "Notification",
        timeout: 86400000,
        blocking: false,
    },
    QwenHookEvent {
        name: "PreCompact",
        timeout: 5000,
        blocking: true,
    },
];

const QODER_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: true,
    },
];

const DROID_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: true,
    },
];

const CODEBUDDY_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: true,
    },
];

const CODYBUDDYCN_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: true,
    },
];

const STEPFUN_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: true,
    },
];

const ANTIGRAVITY_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: true,
    },
];

const WORKBUDDY_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: true,
    },
];

const HERMES_EVENTS: &[ClaudeHookEvent] = &[
    ClaudeHookEvent {
        name: "UserPromptSubmit",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "PreToolUse",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PostToolUse",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SessionStart",
        timeout: 5,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "SessionEnd",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Stop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStart",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "SubagentStop",
        timeout: 5,
        blocking: true,
    },
    ClaudeHookEvent {
        name: "Notification",
        timeout: 86400,
        blocking: false,
    },
    ClaudeHookEvent {
        name: "PreCompact",
        timeout: 5,
        blocking: true,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TraeHookEvent {
    name: &'static str,
    timeout_sec: u64,
    blocking: bool,
}

const TRAE_EVENTS: &[TraeHookEvent] = &[
    TraeHookEvent {
        name: "session_start",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "session_end",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "user_prompt_submit",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "pre_tool_use",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "post_tool_use",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "post_tool_use_failure",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "permission_request",
        timeout_sec: 86400,
        blocking: true,
    },
    TraeHookEvent {
        name: "notification",
        timeout_sec: 86400,
        blocking: false,
    },
    TraeHookEvent {
        name: "subagent_start",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "subagent_stop",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "stop",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "pre_compact",
        timeout_sec: 5,
        blocking: false,
    },
    TraeHookEvent {
        name: "post_compact",
        timeout_sec: 5,
        blocking: false,
    },
];

pub fn supports_provider(provider: &str) -> bool {
    crate::hooks::profiles::supports_provider(provider)
}

pub fn install(provider: &str) -> Result<HookOperationReport> {
    let provider = canonical_provider_id(provider)?;
    match provider {
        "claude" => install_claude(),
        "cline" => install_cline(),
        "codex" => install_codex(),
        "copilot" => install_copilot(),
        "cursor" => install_cursor(),
        "gemini" => install_gemini(),
        "kimi" => install_kimi(),
        "kiro" => install_kiro(),
        "opencode" => install_opencode(),
        "qwen" => install_qwen(),
        "trae_gui" => install_trae_gui(),
        "traecn" => install_traecn(),
        "qoder" => install_qoder(),
        "droid" => install_droid(),
        "codebuddy" => install_codebuddy(),
        "codybuddycn" => install_codybuddycn(),
        "stepfun" => install_stepfun(),
        "antigravity" => install_antigravity(),
        "workbuddy" => install_workbuddy(),
        "hermes" => install_hermes(),
        "pi" => install_pi(),
        "omp" => install_omp(),
        "trae" => install_trae(),
        _ => anyhow::bail!(
            "Hook installer is registered but not implemented for provider: {provider}"
        ),
    }
}

pub fn uninstall(provider: &str) -> Result<HookOperationReport> {
    let provider = canonical_provider_id(provider)?;
    match provider {
        "claude" => uninstall_claude(),
        "cline" => uninstall_cline(),
        "codex" => uninstall_codex(),
        "copilot" => uninstall_copilot(),
        "cursor" => uninstall_cursor(),
        "gemini" => uninstall_gemini(),
        "kimi" => uninstall_kimi(),
        "kiro" => uninstall_kiro(),
        "opencode" => uninstall_opencode(),
        "qwen" => uninstall_qwen(),
        "trae_gui" => uninstall_trae_gui(),
        "traecn" => uninstall_traecn(),
        "qoder" => uninstall_qoder(),
        "droid" => uninstall_droid(),
        "codebuddy" => uninstall_codebuddy(),
        "codybuddycn" => uninstall_codybuddycn(),
        "stepfun" => uninstall_stepfun(),
        "antigravity" => uninstall_antigravity(),
        "workbuddy" => uninstall_workbuddy(),
        "hermes" => uninstall_hermes(),
        "pi" => uninstall_pi(),
        "omp" => uninstall_omp(),
        "trae" => uninstall_trae(),
        _ => anyhow::bail!(
            "Hook uninstaller is registered but not implemented for provider: {provider}"
        ),
    }
}

pub fn repair(provider: &str) -> Result<HookOperationReport> {
    let provider = canonical_provider_id(provider)?;
    let before = health::status(provider)?;
    let mut report = install(provider)?;
    report.operation = "repair".to_string();
    report.changed = before.status != HookHealthStatus::InstalledOk;
    Ok(report)
}

pub fn verify(provider: &str) -> Result<HookOperationReport> {
    let provider = canonical_provider_id(provider)?;
    let status = health::status(provider)?;
    Ok(HookOperationReport {
        provider: provider.to_string(),
        operation: "verify".to_string(),
        changed: false,
        backup_path: None,
        message: status.message.clone(),
        status,
    })
}

pub(crate) fn claude_settings_path() -> PathBuf {
    hook_home_dir().join(".claude").join("settings.json")
}

pub(crate) fn cline_hooks_dir() -> PathBuf {
    hook_home_dir()
        .join("Documents")
        .join("Cline")
        .join("Rules")
        .join("Hooks")
}

pub(crate) fn cline_legacy_hooks_dir() -> PathBuf {
    hook_home_dir()
        .join("Documents")
        .join("Cline")
        .join("Hooks")
}

pub(crate) fn cline_hooks_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![cline_hooks_dir(), cline_legacy_hooks_dir()];
    dirs.dedup();
    dirs
}

pub(crate) fn codex_home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(hook_home_dir().join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

pub(crate) fn codex_hooks_path() -> PathBuf {
    codex_home().join("hooks.json")
}

pub(crate) fn codex_config_path() -> PathBuf {
    codex_home().join("config.toml")
}

pub(crate) fn cursor_hooks_path() -> PathBuf {
    hook_home_dir().join(".cursor").join("hooks.json")
}

pub(crate) fn trae_gui_hooks_path() -> PathBuf {
    hook_home_dir().join(".trae").join("hooks.json")
}

pub(crate) fn traecn_hooks_path() -> PathBuf {
    hook_home_dir().join(".trae-cn").join("hooks.json")
}

pub(crate) fn copilot_hooks_path() -> PathBuf {
    hook_home_dir()
        .join(".copilot")
        .join("hooks")
        .join("memorph.json")
}

pub(crate) fn gemini_settings_path() -> PathBuf {
    hook_home_dir().join(".gemini").join("settings.json")
}

pub(crate) fn kimi_config_path() -> PathBuf {
    hook_home_dir().join(".kimi").join("config.toml")
}

pub(crate) fn kiro_agent_path() -> PathBuf {
    hook_home_dir()
        .join(".kiro")
        .join("agents")
        .join("memorph.json")
}

pub(crate) fn qwen_settings_path() -> PathBuf {
    hook_home_dir().join(".qwen").join("settings.json")
}

pub(crate) fn qoder_settings_path() -> PathBuf {
    hook_home_dir().join(".qoder").join("settings.json")
}

pub(crate) fn droid_settings_path() -> PathBuf {
    hook_home_dir().join(".factory").join("settings.json")
}

pub(crate) fn codebuddy_settings_path() -> PathBuf {
    hook_home_dir().join(".codebuddy").join("settings.json")
}

pub(crate) fn codybuddycn_settings_path() -> PathBuf {
    hook_home_dir().join(".codybuddycn").join("settings.json")
}

pub(crate) fn stepfun_settings_path() -> PathBuf {
    hook_home_dir().join(".stepfun").join("settings.json")
}

pub(crate) fn antigravity_settings_path() -> PathBuf {
    hook_home_dir().join(".antigravity").join("settings.json")
}

pub(crate) fn workbuddy_settings_path() -> PathBuf {
    hook_home_dir().join(".workbuddy").join("settings.json")
}

pub(crate) fn hermes_settings_path() -> PathBuf {
    hook_home_dir().join(".hermes").join("settings.json")
}

pub(crate) fn traecli_config_path() -> PathBuf {
    hook_home_dir().join(".trae").join("traecli.yaml")
}

pub(crate) fn pi_agent_dir() -> PathBuf {
    hook_home_dir().join(".pi").join("agent")
}

pub(crate) fn pi_extension_dir() -> PathBuf {
    pi_agent_dir().join("extensions")
}

pub(crate) fn pi_extension_path() -> PathBuf {
    pi_extension_dir().join("memorph.ts")
}

pub(crate) fn omp_agent_dir() -> PathBuf {
    hook_home_dir().join(".omp").join("agent")
}

pub(crate) fn omp_extension_dir() -> PathBuf {
    omp_agent_dir().join("extensions")
}

pub(crate) fn omp_extension_path() -> PathBuf {
    omp_extension_dir().join("memorph.ts")
}

pub(crate) fn opencode_config_dir() -> PathBuf {
    hook_home_dir().join(".config").join("opencode")
}

pub(crate) fn opencode_plugin_dir() -> PathBuf {
    opencode_config_dir().join("plugins")
}

pub(crate) fn opencode_plugin_path() -> PathBuf {
    opencode_plugin_dir().join(OPENCODE_PLUGIN_FILE)
}

pub(crate) fn opencode_config_candidates() -> Vec<PathBuf> {
    let dir = opencode_config_dir();
    vec![
        dir.join("opencode.jsonc"),
        dir.join("opencode.json"),
        dir.join("config.json"),
    ]
}

pub(crate) fn claude_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PermissionRequest",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "SessionStart",
        "SessionEnd",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn cline_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "TaskStart",
        "TaskResume",
        "TaskCancel",
        "TaskComplete",
    ];
    EVENTS
}

pub(crate) fn codex_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "SessionStart",
        "SessionEnd",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
        "Stop",
    ];
    EVENTS
}

pub(crate) fn cursor_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "beforeSubmitPrompt",
        "beforeShellExecution",
        "afterShellExecution",
        "beforeReadFile",
        "afterFileEdit",
        "beforeMCPExecution",
        "afterMCPExecution",
        "afterAgentThought",
        "afterAgentResponse",
        "stop",
    ];
    EVENTS
}

pub(crate) fn trae_gui_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "beforeSubmitPrompt",
        "beforeShellExecution",
        "afterShellExecution",
        "beforeReadFile",
        "afterFileEdit",
        "beforeMCPExecution",
        "afterMCPExecution",
        "afterAgentThought",
        "afterAgentResponse",
        "stop",
    ];
    EVENTS
}

pub(crate) fn traecn_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "beforeSubmitPrompt",
        "beforeShellExecution",
        "afterShellExecution",
        "beforeReadFile",
        "afterFileEdit",
        "beforeMCPExecution",
        "afterMCPExecution",
        "afterAgentThought",
        "afterAgentResponse",
        "stop",
    ];
    EVENTS
}

pub(crate) fn gemini_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "SessionStart",
        "SessionEnd",
        "BeforeTool",
        "AfterTool",
        "BeforeAgent",
        "AfterAgent",
    ];
    EVENTS
}

pub(crate) fn kimi_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "SessionStart",
        "SessionEnd",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn kiro_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "agentSpawn",
        "userPromptSubmit",
        "preToolUse",
        "postToolUse",
        "stop",
    ];
    EVENTS
}

pub(crate) fn qwen_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PermissionRequest",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "SessionStart",
        "SessionEnd",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn qoder_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SessionStart",
        "SessionEnd",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn droid_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SessionStart",
        "SessionEnd",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn codebuddy_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SessionStart",
        "SessionEnd",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn codybuddycn_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SessionStart",
        "SessionEnd",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn stepfun_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SessionStart",
        "SessionEnd",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn antigravity_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SessionStart",
        "SessionEnd",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn workbuddy_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SessionStart",
        "SessionEnd",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn hermes_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SessionStart",
        "SessionEnd",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "Notification",
        "PreCompact",
    ];
    EVENTS
}

pub(crate) fn copilot_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "sessionStart",
        "sessionEnd",
        "userPromptSubmitted",
        "preToolUse",
        "postToolUse",
        "errorOccurred",
    ];
    EVENTS
}

pub(crate) fn trae_required_events() -> &'static [&'static str] {
    const EVENTS: &[&str] = &[
        "session_start",
        "session_end",
        "user_prompt_submit",
        "pre_tool_use",
        "post_tool_use",
        "post_tool_use_failure",
        "permission_request",
        "notification",
        "subagent_start",
        "subagent_stop",
        "stop",
        "pre_compact",
        "post_compact",
    ];
    EVENTS
}

pub(crate) fn command_contains_memorph_hook(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("memorph") && lower.contains(HOOK_COMMAND_MARKER)
}

pub(crate) fn current_hook_managed_version() -> &'static str {
    HOOK_MANAGED_VERSION
}

fn canonical_provider_id(provider: &str) -> Result<&'static str> {
    crate::hooks::profiles::find(provider)
        .map(|profile| profile.provider)
        .ok_or_else(|| {
            anyhow::anyhow!("Hook installer is not implemented for provider: {provider}")
        })
}

fn install_claude() -> Result<HookOperationReport> {
    let path = claude_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Claude config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in CLAUDE_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider claude --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("claude")?;
    Ok(HookOperationReport {
        provider: "claude".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Claude hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_claude() -> Result<HookOperationReport> {
    let path = claude_settings_path();
    if !path.exists() {
        let status = health::status("claude")?;
        return Ok(HookOperationReport {
            provider: "claude".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Claude settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("claude")?;
    Ok(HookOperationReport {
        provider: "claude".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Claude memorph hook entries removed.".to_string()),
    })
}

fn install_cline() -> Result<HookOperationReport> {
    let command_base = bridge_command_base()?;
    let mut changed = false;
    let mut backup_path = None;
    for dir in cline_hooks_dirs() {
        fs::create_dir_all(&dir).with_context(|| {
            format!("Failed to create Cline hooks directory: {}", dir.display())
        })?;
        for event in CLINE_EVENTS {
            let path = dir.join(event.name);
            let original = fs::read_to_string(&path).ok();
            let preserved_path = cline_preserved_hook_path(&path);
            if let Some(contents) = original.as_deref() {
                if !contents.contains(CLINE_HOOK_MARKER)
                    && !command_contains_memorph_hook(contents)
                    && !preserved_path.exists()
                {
                    if backup_path.is_none() && path.exists() {
                        backup_path = backup_if_exists(&path)?;
                    }
                    atomic_write::write_string_atomic(&preserved_path, contents)?;
                    make_executable(&preserved_path)?;
                }
            }
            let rendered = cline_hook_script(
                &command_base,
                event,
                preserved_path.exists().then_some(preserved_path.as_path()),
            )?;
            if original.as_deref() != Some(rendered.as_str()) {
                if backup_path.is_none() && path.exists() {
                    backup_path = backup_if_exists(&path)?;
                }
                atomic_write::write_string_atomic(&path, &rendered)?;
                make_executable(&path)?;
                changed = true;
            } else {
                make_executable(&path)?;
            }
        }
    }

    let status = health::status("cline")?;
    Ok(HookOperationReport {
        provider: "cline".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Cline file-based hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_cline() -> Result<HookOperationReport> {
    let dirs = cline_hooks_dirs();
    if !dirs.iter().any(|dir| dir.exists()) {
        let status = health::status("cline")?;
        return Ok(HookOperationReport {
            provider: "cline".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Cline hooks directory does not exist.".to_string()),
        });
    }

    let mut changed = false;
    let mut backup_path = None;
    for dir in dirs {
        for event in CLINE_EVENTS {
            let path = dir.join(event.name);
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            if !contents.contains(CLINE_HOOK_MARKER) || !command_contains_memorph_hook(&contents) {
                continue;
            }
            if backup_path.is_none() {
                backup_path = backup_if_exists(&path)?;
            }
            let preserved_path = cline_preserved_hook_path(&path);
            if preserved_path.exists() {
                fs::copy(&preserved_path, &path).with_context(|| {
                    format!(
                        "Failed to restore preserved Cline hook file: {}",
                        path.display()
                    )
                })?;
                make_executable(&path)?;
                fs::remove_file(&preserved_path).with_context(|| {
                    format!(
                        "Failed to remove preserved Cline hook file: {}",
                        preserved_path.display()
                    )
                })?;
            } else {
                fs::remove_file(&path).with_context(|| {
                    format!("Failed to remove Cline hook file: {}", path.display())
                })?;
            }
            changed = true;
        }
    }

    let status = health::status("cline")?;
    Ok(HookOperationReport {
        provider: "cline".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Cline memorph hook entries removed.".to_string()),
    })
}

fn install_codex() -> Result<HookOperationReport> {
    let path = codex_hooks_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Codex hook directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in CODEX_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider codex --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let hooks_changed = root != original;
    write_json_object(&path, &root)?;
    let flag_changed = enable_codex_hooks_config()?;
    let status = health::status("codex")?;
    Ok(HookOperationReport {
        provider: "codex".to_string(),
        operation: "install".to_string(),
        changed: hooks_changed || flag_changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Codex hook entries installed and hooks feature enabled.".to_string()),
        status,
    })
}

fn uninstall_codex() -> Result<HookOperationReport> {
    let path = codex_hooks_path();
    if !path.exists() {
        let status = health::status("codex")?;
        return Ok(HookOperationReport {
            provider: "codex".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Codex hooks.json file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("codex")?;
    Ok(HookOperationReport {
        provider: "codex".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Codex memorph hook entries removed.".to_string()),
    })
}

fn install_copilot() -> Result<HookOperationReport> {
    let path = copilot_hooks_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Copilot hook directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if !root.contains_key("version") {
        root.insert("version".to_string(), json!(1));
    }
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in COPILOT_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider copilot --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "type": "command",
            "bash": command,
            "timeoutSec": event.timeout_sec
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("copilot")?;
    Ok(HookOperationReport {
        provider: "copilot".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Copilot hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_copilot() -> Result<HookOperationReport> {
    let path = copilot_hooks_path();
    if !path.exists() {
        let status = health::status("copilot")?;
        return Ok(HookOperationReport {
            provider: "copilot".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Copilot hook file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("copilot")?;
    Ok(HookOperationReport {
        provider: "copilot".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Copilot memorph hook entries removed.".to_string()),
    })
}

fn install_cursor() -> Result<HookOperationReport> {
    let path = cursor_hooks_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Cursor hook directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in CURSOR_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider cursor --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({ "command": command }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("cursor")?;
    Ok(HookOperationReport {
        provider: "cursor".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Cursor hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_cursor() -> Result<HookOperationReport> {
    let path = cursor_hooks_path();
    if !path.exists() {
        let status = health::status("cursor")?;
        return Ok(HookOperationReport {
            provider: "cursor".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Cursor hooks.json file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("cursor")?;
    Ok(HookOperationReport {
        provider: "cursor".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Cursor memorph hook entries removed.".to_string()),
    })
}

fn install_trae_gui() -> Result<HookOperationReport> {
    let path = trae_gui_hooks_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create Trae hook directory: {}", parent.display())
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in TRAE_GUI_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider trae_gui --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({ "command": command }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("trae_gui")?;
    Ok(HookOperationReport {
        provider: "trae_gui".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Trae hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_trae_gui() -> Result<HookOperationReport> {
    let path = trae_gui_hooks_path();
    if !path.exists() {
        let status = health::status("trae_gui")?;
        return Ok(HookOperationReport {
            provider: "trae_gui".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Trae hooks.json file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("trae_gui")?;
    Ok(HookOperationReport {
        provider: "trae_gui".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Trae memorph hook entries removed.".to_string()),
    })
}

fn install_traecn() -> Result<HookOperationReport> {
    let path = traecn_hooks_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Trae CN hook directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in TRAECN_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider traecn --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({ "command": command }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("traecn")?;
    Ok(HookOperationReport {
        provider: "traecn".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Trae CN hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_traecn() -> Result<HookOperationReport> {
    let path = traecn_hooks_path();
    if !path.exists() {
        let status = health::status("traecn")?;
        return Ok(HookOperationReport {
            provider: "traecn".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Trae CN hooks.json file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("traecn")?;
    Ok(HookOperationReport {
        provider: "traecn".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Trae CN memorph hook entries removed.".to_string()),
    })
}

fn install_gemini() -> Result<HookOperationReport> {
    let path = gemini_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Gemini config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in GEMINI_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider gemini --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("gemini")?;
    Ok(HookOperationReport {
        provider: "gemini".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Gemini hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_gemini() -> Result<HookOperationReport> {
    let path = gemini_settings_path();
    if !path.exists() {
        let status = health::status("gemini")?;
        return Ok(HookOperationReport {
            provider: "gemini".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Gemini settings.json file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("gemini")?;
    Ok(HookOperationReport {
        provider: "gemini".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Gemini memorph hook entries removed.".to_string()),
    })
}

fn install_kimi() -> Result<HookOperationReport> {
    let path = kimi_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Kimi config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = fs::read_to_string(&path).unwrap_or_default();
    let backup_path = backup_if_exists(&path)?;
    let mut updated = remove_kimi_hooks(&original);
    let command_base = bridge_command_base()?;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    if !updated.trim().is_empty() {
        updated.push('\n');
    }
    updated.push_str(&kimi_hook_blocks(&command_base)?);

    let changed = updated != original;
    atomic_write::write_string_atomic(&path, &updated)?;
    let status = health::status("kimi")?;
    Ok(HookOperationReport {
        provider: "kimi".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Kimi hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_kimi() -> Result<HookOperationReport> {
    let path = kimi_config_path();
    if !path.exists() {
        let status = health::status("kimi")?;
        return Ok(HookOperationReport {
            provider: "kimi".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Kimi config.toml file does not exist.".to_string()),
        });
    }

    let original = fs::read_to_string(&path).unwrap_or_default();
    let backup_path = backup_if_exists(&path)?;
    let updated = remove_kimi_hooks(&original);
    let changed = updated != original;
    if changed {
        atomic_write::write_string_atomic(&path, &updated)?;
    }
    let status = health::status("kimi")?;
    Ok(HookOperationReport {
        provider: "kimi".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Kimi memorph hook entries removed.".to_string()),
    })
}

fn install_kiro() -> Result<HookOperationReport> {
    let path = kiro_agent_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Kiro agent directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if !root.contains_key("name") {
        root.insert("name".to_string(), Value::String("memorph".to_string()));
    }
    if !root.contains_key("description") {
        root.insert(
            "description".to_string(),
            Value::String(
                "Auto-generated by memorph. Launch with `kiro --agent memorph` to relay hook events."
                    .to_string(),
            ),
        );
    }
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in KIRO_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider kiro --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "command": command,
            "matcher": "*",
            "timeout_ms": event.timeout_ms
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("kiro")?;
    Ok(HookOperationReport {
        provider: "kiro".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some(
            "Kiro memorph agent hooks installed. Launch with `kiro --agent memorph`.".to_string(),
        ),
        status,
    })
}

fn uninstall_kiro() -> Result<HookOperationReport> {
    let path = kiro_agent_path();
    if !path.exists() {
        let status = health::status("kiro")?;
        return Ok(HookOperationReport {
            provider: "kiro".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Kiro memorph agent file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("kiro")?;
    Ok(HookOperationReport {
        provider: "kiro".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Kiro memorph hook entries removed.".to_string()),
    })
}

fn install_qwen() -> Result<HookOperationReport> {
    let path = qwen_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Qwen config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in QWEN_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider qwen --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("qwen")?;
    Ok(HookOperationReport {
        provider: "qwen".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Qwen hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_qwen() -> Result<HookOperationReport> {
    let path = qwen_settings_path();
    if !path.exists() {
        let status = health::status("qwen")?;
        return Ok(HookOperationReport {
            provider: "qwen".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Qwen settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("qwen")?;
    Ok(HookOperationReport {
        provider: "qwen".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Qwen memorph hook entries removed.".to_string()),
    })
}

fn install_qoder() -> Result<HookOperationReport> {
    let path = qoder_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Qoder config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in QODER_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider qoder --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("qoder")?;
    Ok(HookOperationReport {
        provider: "qoder".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Qoder hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_qoder() -> Result<HookOperationReport> {
    let path = qoder_settings_path();
    if !path.exists() {
        let status = health::status("qoder")?;
        return Ok(HookOperationReport {
            provider: "qoder".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Qoder settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("qoder")?;
    Ok(HookOperationReport {
        provider: "qoder".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Qoder memorph hook entries removed.".to_string()),
    })
}

fn install_droid() -> Result<HookOperationReport> {
    let path = droid_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Factory config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in DROID_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider droid --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("droid")?;
    Ok(HookOperationReport {
        provider: "droid".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Factory hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_droid() -> Result<HookOperationReport> {
    let path = droid_settings_path();
    if !path.exists() {
        let status = health::status("droid")?;
        return Ok(HookOperationReport {
            provider: "droid".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Factory settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("droid")?;
    Ok(HookOperationReport {
        provider: "droid".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Factory memorph hook entries removed.".to_string()),
    })
}

fn install_codebuddy() -> Result<HookOperationReport> {
    let path = codebuddy_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create CodeBuddy config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in CODEBUDDY_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider codebuddy --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("codebuddy")?;
    Ok(HookOperationReport {
        provider: "codebuddy".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("CodeBuddy hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_codebuddy() -> Result<HookOperationReport> {
    let path = codebuddy_settings_path();
    if !path.exists() {
        let status = health::status("codebuddy")?;
        return Ok(HookOperationReport {
            provider: "codebuddy".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("CodeBuddy settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("codebuddy")?;
    Ok(HookOperationReport {
        provider: "codebuddy".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("CodeBuddy memorph hook entries removed.".to_string()),
    })
}

fn install_codybuddycn() -> Result<HookOperationReport> {
    let path = codybuddycn_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create CodyBuddyCN config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in CODYBUDDYCN_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider codybuddycn --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("codybuddycn")?;
    Ok(HookOperationReport {
        provider: "codybuddycn".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("CodyBuddyCN hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_codybuddycn() -> Result<HookOperationReport> {
    let path = codybuddycn_settings_path();
    if !path.exists() {
        let status = health::status("codybuddycn")?;
        return Ok(HookOperationReport {
            provider: "codybuddycn".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("CodyBuddyCN settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("codybuddycn")?;
    Ok(HookOperationReport {
        provider: "codybuddycn".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("CodyBuddyCN memorph hook entries removed.".to_string()),
    })
}

fn install_stepfun() -> Result<HookOperationReport> {
    let path = stepfun_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create StepFun config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in STEPFUN_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider stepfun --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("stepfun")?;
    Ok(HookOperationReport {
        provider: "stepfun".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("StepFun hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_stepfun() -> Result<HookOperationReport> {
    let path = stepfun_settings_path();
    if !path.exists() {
        let status = health::status("stepfun")?;
        return Ok(HookOperationReport {
            provider: "stepfun".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("StepFun settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("stepfun")?;
    Ok(HookOperationReport {
        provider: "stepfun".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("StepFun memorph hook entries removed.".to_string()),
    })
}

fn install_antigravity() -> Result<HookOperationReport> {
    let path = antigravity_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create AntiGravity config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in ANTIGRAVITY_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider antigravity --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("antigravity")?;
    Ok(HookOperationReport {
        provider: "antigravity".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("AntiGravity hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_antigravity() -> Result<HookOperationReport> {
    let path = antigravity_settings_path();
    if !path.exists() {
        let status = health::status("antigravity")?;
        return Ok(HookOperationReport {
            provider: "antigravity".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("AntiGravity settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("antigravity")?;
    Ok(HookOperationReport {
        provider: "antigravity".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("AntiGravity memorph hook entries removed.".to_string()),
    })
}

fn install_workbuddy() -> Result<HookOperationReport> {
    let path = workbuddy_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create WorkBuddy config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in WORKBUDDY_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider workbuddy --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("workbuddy")?;
    Ok(HookOperationReport {
        provider: "workbuddy".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("WorkBuddy hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_workbuddy() -> Result<HookOperationReport> {
    let path = workbuddy_settings_path();
    if !path.exists() {
        let status = health::status("workbuddy")?;
        return Ok(HookOperationReport {
            provider: "workbuddy".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("WorkBuddy settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("workbuddy")?;
    Ok(HookOperationReport {
        provider: "workbuddy".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("WorkBuddy memorph hook entries removed.".to_string()),
    })
}

fn install_hermes() -> Result<HookOperationReport> {
    let path = hermes_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Hermes config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = bridge_command_base()?;

    let hooks = ensure_object_field(&mut root, "hooks");
    for event in HERMES_EVENTS {
        let entries = ensure_array_field(hooks, event.name);
        entries.retain(|entry| !entry_contains_memorph_hook(entry));
        let command = format!(
            "{} --managed-version {} --provider hermes --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    write_json_object(&path, &root)?;
    let status = health::status("hermes")?;
    Ok(HookOperationReport {
        provider: "hermes".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Hermes hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_hermes() -> Result<HookOperationReport> {
    let path = hermes_settings_path();
    if !path.exists() {
        let status = health::status("hermes")?;
        return Ok(HookOperationReport {
            provider: "hermes".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Hermes settings file does not exist.".to_string()),
        });
    }

    let original = read_json_object_or_empty(&path)?;
    let backup_path = backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| !entry_contains_memorph_hook(entry));
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        write_json_object(&path, &root)?;
    }
    let status = health::status("hermes")?;
    Ok(HookOperationReport {
        provider: "hermes".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Hermes memorph hook entries removed.".to_string()),
    })
}

fn install_trae() -> Result<HookOperationReport> {
    let path = traecli_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create TraeCli config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = fs::read_to_string(&path).unwrap_or_default();
    let backup_path = backup_if_exists(&path)?;
    let command_base = bridge_command_base()?;
    let updated = merge_traecli_hooks(&original, &command_base)?;
    let changed = updated != original;
    if changed {
        atomic_write::write_string_atomic(&path, &updated)?;
    }
    let status = health::status("trae")?;
    Ok(HookOperationReport {
        provider: "trae".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("TraeCli hook entries installed.".to_string()),
        status,
    })
}

fn uninstall_trae() -> Result<HookOperationReport> {
    let path = traecli_config_path();
    if !path.exists() {
        let status = health::status("trae")?;
        return Ok(HookOperationReport {
            provider: "trae".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("TraeCli config file does not exist.".to_string()),
        });
    }

    let original = fs::read_to_string(&path).unwrap_or_default();
    let backup_path = backup_if_exists(&path)?;
    let updated = remove_traecli_hooks(&original);
    let changed = updated != original;
    if changed {
        atomic_write::write_string_atomic(&path, &updated)?;
    }
    let status = health::status("trae")?;
    Ok(HookOperationReport {
        provider: "trae".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("TraeCli memorph hook entries removed.".to_string()),
    })
}

fn install_pi() -> Result<HookOperationReport> {
    let path = pi_extension_path();
    fs::create_dir_all(pi_extension_dir()).with_context(|| {
        format!(
            "Failed to create pi extension directory: {}",
            pi_extension_dir().display()
        )
    })?;
    let original = fs::read_to_string(&path).ok();
    let backup_path = backup_if_exists(&path)?;
    let source = pi_extension_source()?;
    let changed = original.as_deref() != Some(source.as_str());
    if changed {
        atomic_write::write_string_atomic(&path, &source)?;
    }
    let status = health::status("pi")?;
    Ok(HookOperationReport {
        provider: "pi".to_string(),
        operation: "install".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("pi memorph extension installed.".to_string()),
    })
}

fn uninstall_pi() -> Result<HookOperationReport> {
    let path = pi_extension_path();
    if !path.exists() {
        let status = health::status("pi")?;
        return Ok(HookOperationReport {
            provider: "pi".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("pi memorph extension file does not exist.".to_string()),
        });
    }
    let contents = fs::read_to_string(&path).unwrap_or_default();
    if !contents.contains(PI_EXTENSION_MARKER) {
        let status = health::status("pi")?;
        return Ok(HookOperationReport {
            provider: "pi".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("pi extension file is not managed by memorph.".to_string()),
        });
    }
    let backup_path = backup_if_exists(&path)?;
    fs::remove_file(&path)
        .with_context(|| format!("Failed to remove pi extension file: {}", path.display()))?;
    let status = health::status("pi")?;
    Ok(HookOperationReport {
        provider: "pi".to_string(),
        operation: "uninstall".to_string(),
        changed: true,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("pi memorph extension removed.".to_string()),
    })
}

fn install_omp() -> Result<HookOperationReport> {
    let path = omp_extension_path();
    fs::create_dir_all(omp_extension_dir()).with_context(|| {
        format!(
            "Failed to create OMP extension directory: {}",
            omp_extension_dir().display()
        )
    })?;
    let original = fs::read_to_string(&path).ok();
    let backup_path = backup_if_exists(&path)?;
    let source = omp_extension_source()?;
    let changed = original.as_deref() != Some(source.as_str());
    if changed {
        atomic_write::write_string_atomic(&path, &source)?;
    }
    let status = health::status("omp")?;
    Ok(HookOperationReport {
        provider: "omp".to_string(),
        operation: "install".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Oh My Pi memorph extension installed.".to_string()),
    })
}

fn uninstall_omp() -> Result<HookOperationReport> {
    let path = omp_extension_path();
    if !path.exists() {
        let status = health::status("omp")?;
        return Ok(HookOperationReport {
            provider: "omp".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Oh My Pi memorph extension file does not exist.".to_string()),
        });
    }
    let contents = fs::read_to_string(&path).unwrap_or_default();
    if !contents.contains(OMP_EXTENSION_MARKER) {
        let status = health::status("omp")?;
        return Ok(HookOperationReport {
            provider: "omp".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Oh My Pi extension file is not managed by memorph.".to_string()),
        });
    }
    let backup_path = backup_if_exists(&path)?;
    fs::remove_file(&path).with_context(|| {
        format!(
            "Failed to remove Oh My Pi extension file: {}",
            path.display()
        )
    })?;
    let status = health::status("omp")?;
    Ok(HookOperationReport {
        provider: "omp".to_string(),
        operation: "uninstall".to_string(),
        changed: true,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Oh My Pi memorph extension removed.".to_string()),
    })
}

fn install_opencode() -> Result<HookOperationReport> {
    let config_dir = opencode_config_dir();
    if !config_dir.exists() {
        let status = health::status("opencode")?;
        return Ok(HookOperationReport {
            provider: "opencode".to_string(),
            operation: "install".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("OpenCode config directory does not exist.".to_string()),
        });
    }

    fs::create_dir_all(opencode_plugin_dir()).with_context(|| {
        format!(
            "Failed to create OpenCode plugin directory: {}",
            opencode_plugin_dir().display()
        )
    })?;
    let plugin_path = opencode_plugin_path();
    let plugin_source = opencode_plugin_source()?;
    let plugin_changed = fs::read_to_string(&plugin_path)
        .map(|existing| existing != plugin_source)
        .unwrap_or(true);
    if plugin_changed {
        atomic_write::write_string_atomic(&plugin_path, &plugin_source)?;
    }

    let target_path = opencode_registration_target();
    let original = fs::read_to_string(&target_path).ok();
    let backup_path = if original.as_deref().unwrap_or_default().is_empty() {
        None
    } else {
        backup_if_exists(&target_path)?
    };
    let plugin_ref = format!("file://{}", plugin_path.display());
    let merged = merge_opencode_plugin_ref(original.as_deref(), &plugin_ref)?;
    let config_changed = original.as_deref() != Some(merged.as_str());
    if config_changed {
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create OpenCode config directory: {}",
                    parent.display()
                )
            })?;
        }
        atomic_write::write_string_atomic(&target_path, &merged)?;
    }

    let status = health::status("opencode")?;
    Ok(HookOperationReport {
        provider: "opencode".to_string(),
        operation: "install".to_string(),
        changed: plugin_changed || config_changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("OpenCode memorph plugin installed.".to_string()),
    })
}

fn uninstall_opencode() -> Result<HookOperationReport> {
    let plugin_path = opencode_plugin_path();
    let mut changed = false;
    if plugin_path.exists() {
        let owned = fs::read_to_string(&plugin_path)
            .map(|contents| contents.contains(OPENCODE_PLUGIN_MARKER))
            .unwrap_or(false);
        if owned {
            fs::remove_file(&plugin_path).with_context(|| {
                format!(
                    "Failed to remove OpenCode plugin: {}",
                    plugin_path.display()
                )
            })?;
            changed = true;
        }
    }

    let mut backup_path = None;
    for config_path in opencode_config_candidates() {
        let Some(contents) = fs::read_to_string(&config_path).ok() else {
            continue;
        };
        let Some(cleaned) = remove_opencode_plugin_ref(&contents)? else {
            continue;
        };
        if backup_path.is_none() {
            backup_path = backup_if_exists(&config_path)?;
        } else {
            let _ = backup_if_exists(&config_path);
        }
        atomic_write::write_string_atomic(&config_path, &cleaned)?;
        changed = true;
    }

    let status = health::status("opencode")?;
    Ok(HookOperationReport {
        provider: "opencode".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("OpenCode memorph plugin removed.".to_string()),
    })
}

pub(crate) fn claude_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn opencode_plugin_installed() -> bool {
    let plugin_path = opencode_plugin_path();
    if !plugin_path.exists() {
        return false;
    }
    let plugin_current = fs::read_to_string(&plugin_path)
        .map(|contents| {
            contents.contains(OPENCODE_PLUGIN_MARKER)
                && contents.contains(&format!("version: {OPENCODE_PLUGIN_VERSION}"))
        })
        .unwrap_or(false);
    if !plugin_current {
        return false;
    }
    opencode_config_candidates()
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .any(|contents| opencode_config_contains_memorph_plugin(&contents))
}

pub(crate) fn current_opencode_plugin_version() -> &'static str {
    OPENCODE_PLUGIN_VERSION
}

pub(crate) fn opencode_installed_plugin_version() -> Option<String> {
    let plugin_path = opencode_plugin_path();
    let contents = fs::read_to_string(plugin_path).ok()?;
    if !contents.contains(OPENCODE_PLUGIN_MARKER) {
        return None;
    }
    contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("// version:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

pub(crate) fn opencode_config_contains_memorph_plugin(contents: &str) -> bool {
    parse_jsonc_object(contents)
        .ok()
        .and_then(|root| root.get("plugin").cloned())
        .and_then(|value| match value {
            Value::Array(values) => Some(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|entry| entry.contains(OPENCODE_PLUGIN_FILE)),
            ),
            Value::String(value) => Some(value.contains(OPENCODE_PLUGIN_FILE)),
            _ => None,
        })
        .unwrap_or(false)
}

pub(crate) fn codex_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn cursor_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn trae_gui_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn traecn_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn copilot_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn gemini_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn kimi_contents_contains_memorph_hook(contents: &str, event: &str) -> bool {
    kimi_hook_blocks_from_contents(contents)
        .into_iter()
        .any(|block| {
            kimi_block_event(&block).as_deref() == Some(event)
                && block_contains_memorph_hook(&block)
        })
}

pub(crate) fn kiro_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn qwen_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn qoder_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn droid_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn codebuddy_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn codybuddycn_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn stepfun_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn antigravity_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn workbuddy_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn hermes_event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub(crate) fn trae_contents_contains_memorph_hook(contents: &str, event: &str) -> bool {
    traecli_hook_blocks_from_contents(contents)
        .into_iter()
        .any(|block| {
            traecli_block_event(&block).as_deref() == Some(event)
                && block_contains_memorph_command(&block)
        })
}

pub(crate) fn claude_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn codex_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn cursor_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn trae_gui_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn traecn_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn copilot_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn gemini_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn kimi_event_memorph_hook_version(
    contents: &str,
    event: &str,
) -> Option<Option<String>> {
    kimi_hook_blocks_from_contents(contents)
        .into_iter()
        .find(|block| {
            kimi_block_event(block).as_deref() == Some(event) && block_contains_memorph_hook(block)
        })
        .map(|block| kimi_block_managed_version(&block))
}

pub(crate) fn kiro_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn qwen_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn qoder_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn droid_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn codebuddy_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn codybuddycn_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn stepfun_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn antigravity_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn workbuddy_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn hermes_event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
    event_memorph_hook_version(root, event)
}

pub(crate) fn trae_event_memorph_hook_version(
    contents: &str,
    event: &str,
) -> Option<Option<String>> {
    traecli_hook_blocks_from_contents(contents)
        .into_iter()
        .find(|block| {
            traecli_block_event(block).as_deref() == Some(event)
                && block_contains_memorph_command(block)
        })
        .map(|block| traecli_block_managed_version(&block))
}

fn block_contains_memorph_hook(block: &[String]) -> bool {
    block
        .iter()
        .filter_map(|line| toml_string_assignment_value(line.trim(), "command"))
        .any(|command| command_contains_memorph_hook(&command))
}

fn kimi_block_managed_version(block: &[String]) -> Option<String> {
    block
        .iter()
        .filter_map(|line| toml_string_assignment_value(line.trim(), "command"))
        .find_map(|command| {
            command_contains_memorph_hook(&command).then(|| command_managed_version(&command))
        })
        .flatten()
}

fn event_memorph_hook_version(root: &Map<String, Value>, event: &str) -> Option<Option<String>> {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries
                .iter()
                .filter(|entry| entry_contains_memorph_hook(entry))
                .find_map(entry_memorph_hook_version)
                .or_else(|| {
                    entries
                        .iter()
                        .any(entry_contains_memorph_hook)
                        .then_some(None)
                })
        })
}

fn entry_contains_memorph_hook(entry: &Value) -> bool {
    if let Some(command) = entry.get("command").and_then(Value::as_str) {
        if command_contains_memorph_hook(command) {
            return true;
        }
    }
    if let Some(command) = entry.get("bash").and_then(Value::as_str) {
        if command_contains_memorph_hook(command) {
            return true;
        }
    }
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .map(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .map(command_contains_memorph_hook)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn entry_memorph_hook_version(entry: &Value) -> Option<Option<String>> {
    if let Some(command) = entry.get("command").and_then(Value::as_str) {
        if command_contains_memorph_hook(command) {
            return Some(command_managed_version(command));
        }
    }
    if let Some(command) = entry.get("bash").and_then(Value::as_str) {
        if command_contains_memorph_hook(command) {
            return Some(command_managed_version(command));
        }
    }
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .and_then(|hooks| {
            hooks.iter().find_map(|hook| {
                let command = hook.get("command").and_then(Value::as_str)?;
                command_contains_memorph_hook(command).then(|| command_managed_version(command))
            })
        })
}

fn command_managed_version(command: &str) -> Option<String> {
    let mut parts = command.split_whitespace();
    while let Some(part) = parts.next() {
        if part == "--managed-version" {
            return parts.next().map(ToString::to_string);
        }
        if let Some(value) = part.strip_prefix("--managed-version=") {
            return Some(value.to_string());
        }
    }
    None
}

pub(crate) fn pi_extension_installed_version(contents: &str) -> Option<Option<String>> {
    extension_installed_version(contents, PI_EXTENSION_MARKER)
}

pub(crate) fn omp_extension_installed_version(contents: &str) -> Option<Option<String>> {
    extension_installed_version(contents, OMP_EXTENSION_MARKER)
}

fn extension_installed_version(contents: &str, marker: &str) -> Option<Option<String>> {
    if !contents.contains(marker) || !contents.contains(HOOK_COMMAND_MARKER) {
        return None;
    }
    let version = contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("// version:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    });
    Some(version)
}

fn pi_extension_source() -> Result<String> {
    memorph_pi_extension_source(
        "pi",
        PI_EXTENSION_MARKER,
        PI_EXTENSION_VERSION,
        r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";"#,
        "pi",
    )
}

fn omp_extension_source() -> Result<String> {
    memorph_pi_extension_source(
        "omp",
        OMP_EXTENSION_MARKER,
        OMP_EXTENSION_VERSION,
        r#"import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent/extensibility/extensions/types";"#,
        "omp",
    )
}

fn memorph_pi_extension_source(
    provider: &str,
    marker: &str,
    version: &str,
    extension_api_import: &str,
    session_prefix: &str,
) -> Result<String> {
    let exe = serde_json::to_string(&bridge_executable_path()?)?;
    let provider_json = serde_json::to_string(provider)?;
    let marker_json = serde_json::to_string(marker)?;
    let version_json = serde_json::to_string(version)?;
    let session_prefix_json = serde_json::to_string(session_prefix)?;
    Ok(format!(
        r#"// {marker}
// version: {version}
// Generated by memorph. Relays pi/OMP agent lifecycle events to memorph hooks.

import {{ execFile, execFileSync }} from "node:child_process";
{extension_api_import}

const MEMORPH_EXE = {exe};
const PROVIDER = {provider_json};
const SESSION_PREFIX = {session_prefix_json};
const MARKER = {marker_json};
const VERSION = {version_json};
const MANAGED_VERSION = "{managed_version}";

const ENV_KEYS = [
  "TERM_PROGRAM",
  "ITERM_SESSION_ID",
  "TERM_SESSION_ID",
  "TMUX",
  "TMUX_PANE",
  "KITTY_WINDOW_ID",
  "__CFBundleIdentifier",
] as const;

const DANGEROUS_PATTERNS: RegExp[] = [
  /\brm\s+(-rf?|--recursive)/i,
  /\bsudo\b/i,
  /\b(chmod|chown)\b.*777/i,
];

function isDangerous(command: string): boolean {{
  return DANGEROUS_PATTERNS.some((pattern) => pattern.test(command));
}}

function collectEnv(): Record<string, string> {{
  const env: Record<string, string> = {{}};
  for (const key of ENV_KEYS) {{
    if (process.env[key]) env[key] = process.env[key]!;
  }}
  return env;
}}

function detectTty(): string | null {{
  try {{
    let pid = process.pid;
    for (let i = 0; i < 8; i++) {{
      const out = execFileSync("ps", ["-o", "tty=,ppid=", "-p", String(pid)], {{
        timeout: 1000,
      }})
        .toString()
        .trim();
      const [tty, ppidStr] = out.split(/\s+/);
      if (tty && tty !== "??" && tty !== "?") {{
        return tty.startsWith("/dev/") ? tty : `/dev/${{tty}}`;
      }}
      const ppid = parseInt(ppidStr ?? "0", 10);
      if (!ppid || ppid <= 1) break;
      pid = ppid;
    }}
  }} catch {{}}
  return null;
}}

function sendToMemorph(
  eventName: string,
  payload: Record<string, unknown>,
  blocking = false,
  timeoutMs = 30_000,
): Promise<Record<string, unknown> | null> {{
  return new Promise((resolve) => {{
    const args = [
      "__hook-bridge",
      "--managed-version",
      MANAGED_VERSION,
      "--provider",
      PROVIDER,
      "--event",
      eventName,
    ];
    if (blocking) args.push("--blocking");
    try {{
      const child = execFile(
        MEMORPH_EXE,
        args,
        {{ timeout: timeoutMs, maxBuffer: 1_048_576 }},
        (error, stdout) => {{
          if (error || !stdout.trim()) {{
            resolve(null);
            return;
          }}
          try {{
            resolve(JSON.parse(stdout));
          }} catch {{
            resolve(null);
          }}
        }},
      );
      child.stdin?.write(JSON.stringify(payload));
      child.stdin?.end();
    }} catch {{
      resolve(null);
    }}
  }});
}}

function base(
  sessionId: string,
  cwd: string,
  extra: Record<string, unknown>,
  tty: string | null,
): Record<string, unknown> {{
  return {{
    session_id: `${{SESSION_PREFIX}}-${{sessionId}}`,
    provider: PROVIDER,
    _source: PROVIDER,
    _ppid: process.pid,
    _env: collectEnv(),
    _tty: tty,
    cwd,
    ...extra,
  }};
}}

function displayToolName(name: string): string {{
  return name.charAt(0).toUpperCase() + name.slice(1);
}}

function extractLastAssistantText(messages: readonly unknown[]): string {{
  const assistants = messages.filter(
    (message): message is {{ role: "assistant"; content: unknown }} =>
      !!message &&
      typeof message === "object" &&
      (message as {{ role?: string }}).role === "assistant",
  );
  const last = assistants.at(-1);
  if (!last || !Array.isArray(last.content)) return "";
  return last.content
    .filter((part): part is {{ type: "text"; text: string }} =>
      !!part &&
      typeof part === "object" &&
      (part as {{ type?: string }}).type === "text" &&
      typeof (part as {{ text?: unknown }}).text === "string",
    )
    .map((part) => part.text)
    .join("")
    .trim();
}}

export default function memorphExtension(pi: ExtensionAPI) {{
  void MARKER;
  void VERSION;
  const tty = detectTty();
  const pendingPermissionSessions = new Set<string>();

  pi.on("session_start", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    const sessionName = typeof pi.getSessionName === "function" ? pi.getSessionName() : undefined;
    await sendToMemorph(
      "SessionStart",
      base(sessionId, ctx.cwd, {{
        hook_event_name: "SessionStart",
        ...(sessionName ? {{ session_title: sessionName }} : {{}}),
      }}, tty),
    );
  }});

  pi.on("session_shutdown", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    await sendToMemorph("SessionEnd", base(sessionId, ctx.cwd, {{ hook_event_name: "SessionEnd" }}, tty));
  }});

  pi.on("before_agent_start", async (event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    const sid = `${{SESSION_PREFIX}}-${{sessionId}}`;
    if (pendingPermissionSessions.has(sid)) return;
    await sendToMemorph(
      "UserPromptSubmit",
      base(sessionId, ctx.cwd, {{
        hook_event_name: "UserPromptSubmit",
        prompt: event.prompt ?? "",
      }}, tty),
    );
  }});

  pi.on("agent_end", async (event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    const sid = `${{SESSION_PREFIX}}-${{sessionId}}`;
    if (pendingPermissionSessions.has(sid)) return;
    const sessionName = typeof pi.getSessionName === "function" ? pi.getSessionName() : undefined;
    await sendToMemorph(
      "Stop",
      base(sessionId, ctx.cwd, {{
        hook_event_name: "Stop",
        last_assistant_message: extractLastAssistantText(event.messages) || undefined,
        ...(sessionName ? {{ session_title: sessionName }} : {{}}),
      }}, tty),
    );
  }});

  pi.on("tool_call", async (event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    const sid = `${{SESSION_PREFIX}}-${{sessionId}}`;
    const toolName = displayToolName(event.toolName);
    const toolInput: Record<string, unknown> = {{ ...event.input }};
    if (event.toolName === "bash") {{
      const command = event.input.command as string | undefined;
      if (command) toolInput.patterns = [command];
    }}
    if (event.toolName === "edit" || event.toolName === "write") {{
      const path = event.input.path as string | undefined;
      if (path) toolInput.file_path = path;
    }}

    if (
      event.toolName === "bash" &&
      typeof event.input.command === "string" &&
      isDangerous(event.input.command)
    ) {{
      pendingPermissionSessions.add(sid);
      let response: Record<string, unknown> | null = null;
      try {{
        response = await sendToMemorph(
          "PermissionRequest",
          base(sessionId, ctx.cwd, {{
            hook_event_name: "PermissionRequest",
            tool_name: toolName,
            tool_input: toolInput,
            tool_use_id: event.toolCallId,
          }}, tty),
          true,
        );
      }} finally {{
        pendingPermissionSessions.delete(sid);
      }}
      if (response?.decision === "deny") {{
        return {{ block: true, reason: "Blocked by memorph" }};
      }}
    }}

    if (!pendingPermissionSessions.has(sid)) {{
      await sendToMemorph(
        "PreToolUse",
        base(sessionId, ctx.cwd, {{
          hook_event_name: "PreToolUse",
          tool_name: toolName,
          tool_input: toolInput,
        }}, tty),
      );
    }}
    return undefined;
  }});

  pi.on("tool_result", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    const sid = `${{SESSION_PREFIX}}-${{sessionId}}`;
    if (pendingPermissionSessions.has(sid)) return;
    await sendToMemorph("PostToolUse", base(sessionId, ctx.cwd, {{ hook_event_name: "PostToolUse" }}, tty));
  }});

  pi.on("session_before_compact", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    await sendToMemorph("PreCompact", base(sessionId, ctx.cwd, {{ hook_event_name: "PreCompact" }}, tty));
  }});

  pi.on("session_compact", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    await sendToMemorph("PostCompact", base(sessionId, ctx.cwd, {{ hook_event_name: "PostCompact" }}, tty));
  }});
}}
"#,
        marker = marker,
        version = version,
        extension_api_import = extension_api_import,
        exe = exe,
        provider_json = provider_json,
        session_prefix_json = session_prefix_json,
        marker_json = marker_json,
        version_json = version_json,
        managed_version = HOOK_MANAGED_VERSION,
    ))
}

fn bridge_command_base() -> Result<String> {
    let exe = std::env::current_exe().context("Failed to resolve current memorph executable")?;
    let exe = shell_quote(&exe.to_string_lossy());
    Ok(format!("{exe} {HOOK_COMMAND_MARKER}"))
}

fn bridge_executable_path() -> Result<String> {
    Ok(std::env::current_exe()
        .context("Failed to resolve current memorph executable")?
        .to_string_lossy()
        .to_string())
}

fn shell_quote(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("'{}'", value.replace('\'', "'\\''"))
    } else {
        value.to_string()
    }
}

pub(crate) fn read_json_object_or_empty(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read JSON file: {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&raw)
        .with_context(|| format!("Failed to parse JSON file: {}", path.display()))?
    {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("Expected JSON object in {}", path.display()),
    }
}

fn write_json_object(path: &Path, root: &Map<String, Value>) -> Result<()> {
    let raw = serde_json::to_string_pretty(root)?;
    atomic_write::write_string_atomic(path, &(raw + "\n"))
}

fn opencode_registration_target() -> PathBuf {
    let candidates = opencode_config_candidates();
    if candidates[0].exists() {
        candidates[0].clone()
    } else {
        candidates[1].clone()
    }
}

fn opencode_plugin_source() -> Result<String> {
    let exe = serde_json::to_string(&bridge_executable_path()?)?;
    let template = r#"// __OPENCODE_PLUGIN_MARKER__
// version: __OPENCODE_PLUGIN_VERSION__
// Auto-generated by memorph. Forwards OpenCode runtime events into memorph hooks.
import {{ execFile }} from "child_process";

const MEMORPH = __MEMORPH_EXE_JSON__;
const pending = new Set();

function send(mapped, blocking = false) {{
  return new Promise((resolve) => {{
    const args = ["__hook-bridge", "--provider", "opencode", "--event", mapped.hook_event_name || "Event"];
    if (blocking) args.push("--blocking");
    const child = execFile(MEMORPH, args, {{ timeout: blocking ? 300000 : 5000, maxBuffer: 1024 * 1024 }}, (error, stdout) => {{
      if (error || !stdout) {{ resolve(null); return; }}
      try {{ resolve(JSON.parse(stdout)); }} catch {{ resolve(null); }}
    }});
    child.stdin.write(JSON.stringify(mapped));
    child.stdin.end();
  }});
}}

function base(sessionId, pid, extra) {{
  return {{ session_id: sessionId, _source: "opencode", _ppid: pid, ...extra }};
}}

function cap(value) {{
  value = value || "";
  return value.charAt(0).toUpperCase() + value.slice(1);
}}

export default {{
  id: "memorph",
  server: async ({{ client, serverUrl }}) => {{
    const pid = process.pid;
    const sessionCwd = new Map();
    const msgRoles = new Map();
    const lastAssistant = new Map();
    const api = client?._client;
    const port = serverUrl ? parseInt(serverUrl.port) || 4096 : 4096;

    async function replyPermission(id, decision) {{
      const behavior = decision?.hookSpecificOutput?.decision?.behavior || decision?.decision;
      if (!behavior) return;
      const reply = behavior === "allow" || behavior === "always" ? "once" : "reject";
      try {{
        if (typeof api?.request === "function") {{
          await api.request({{ method: "POST", url: "/permission/{{requestID}}/reply", path: {{ requestID: id }}, body: {{ reply }} }});
          return;
        }}
      }} catch {{}}
      try {{
        await fetch(`http://localhost:${{port}}/permission/${{id}}/reply`, {{
          method: "POST", headers: {{ "Content-Type": "application/json" }}, body: JSON.stringify({{ reply }})
        }});
      }} catch {{}}
    }}

    function mapEvent(event) {{
      const t = event.type;
      const p = event.properties || {{}};
      if (t === "session.created" && p.info) {{
        const cwd = p.info.directory || "";
        sessionCwd.set(p.info.id, cwd);
        return base(`opencode-${{p.info.id}}`, pid, {{ hook_event_name: "SessionStart", cwd }});
      }}
      if (t === "session.deleted" && p.info) {{
        sessionCwd.delete(p.info.id);
        return base(`opencode-${{p.info.id}}`, pid, {{ hook_event_name: "SessionEnd" }});
      }}
      if (t === "session.updated" && p.info) {{
        if (p.info.directory) sessionCwd.set(p.info.id, p.info.directory);
        if (p.info.time?.archived) return base(`opencode-${{p.info.id}}`, pid, {{ hook_event_name: "SessionEnd" }});
        return null;
      }}
      if (t === "session.status" && p.sessionID && p.status?.type === "idle") {{
        return base(`opencode-${{p.sessionID}}`, pid, {{
          hook_event_name: "Stop", cwd: sessionCwd.get(p.sessionID), last_assistant_message: lastAssistant.get(p.sessionID)
        }});
      }}
      if (t === "message.updated" && p.info?.id && p.info?.sessionID) {{
        msgRoles.set(p.info.id, {{ role: p.info.role, sessionID: p.info.sessionID }});
        if (msgRoles.size > 200) msgRoles.delete(msgRoles.keys().next().value);
        return null;
      }}
      if (t === "message.part.updated" && p.part?.type === "text" && p.part?.messageID) {{
        const meta = msgRoles.get(p.part.messageID);
        if (!meta) return null;
        const text = p.part.text || "";
        if (meta.role === "user" && text) return base(`opencode-${{meta.sessionID}}`, pid, {{
          hook_event_name: "UserPromptSubmit", cwd: sessionCwd.get(meta.sessionID), prompt: text
        }});
        if (meta.role === "assistant" && text) lastAssistant.set(meta.sessionID, text);
        return null;
      }}
      if (t === "message.part.updated" && p.part?.type === "tool" && p.part?.sessionID) {{
        const status = p.part.state?.status;
        const sid = `opencode-${{p.part.sessionID}}`;
        const tool_name = cap(p.part.tool);
        if (status === "running" || status === "pending") return base(sid, pid, {{
          hook_event_name: "PreToolUse", cwd: sessionCwd.get(p.part.sessionID), tool_name, tool_input: p.part.state?.input || {{}}
        }});
        if (status === "completed" || status === "error") return base(sid, pid, {{
          hook_event_name: "PostToolUse", cwd: sessionCwd.get(p.part.sessionID), tool_name
        }});
      }}
      if (t === "permission.asked" && p.id && p.sessionID) {{
        const patterns = p.patterns || [];
        const tool_input = {{ patterns, metadata: p.metadata }};
        if (p.permission === "bash" && patterns.length) tool_input.command = patterns.join(" && ");
        if ((p.permission === "edit" || p.permission === "write") && patterns.length) tool_input.file_path = patterns[0];
        return base(`opencode-${{p.sessionID}}`, pid, {{
          hook_event_name: "PermissionRequest", cwd: sessionCwd.get(p.sessionID), tool_name: cap(p.permission),
          tool_input, _opencode_request_id: p.id
        }});
      }}
      if (t === "permission.replied" && p.sessionID) return base(`opencode-${{p.sessionID}}`, pid, {{
        hook_event_name: "PostToolUse", cwd: sessionCwd.get(p.sessionID)
      }});
      if (t === "question.asked" && p.id && p.sessionID) {{
        return base(`opencode-${{p.sessionID}}`, pid, {{
          hook_event_name: "PermissionRequest", cwd: sessionCwd.get(p.sessionID), tool_name: "AskUserQuestion",
          tool_input: {{ questions: p.questions || [] }}, _opencode_request_id: p.id
        }});
      }}
      if ((t === "question.replied" || t === "question.rejected") && p.sessionID) return base(`opencode-${{p.sessionID}}`, pid, {{
        hook_event_name: "PostToolUse", cwd: sessionCwd.get(p.sessionID)
      }});
      return null;
    }}

    return {{
      event: async ({{ event }}) => {{
        const mapped = mapEvent(event);
        if (!mapped) return;
        // memorph currently records OpenCode permissions/questions without
        // taking over OpenCode's native permission UI. Do not block here unless
        // a future policy layer returns explicit decisions.
        const blocking = false;
        if (blocking) {{
          pending.add(mapped.session_id);
          try {{
            const response = await send(mapped, true);
            if (mapped.tool_name !== "AskUserQuestion" && mapped._opencode_request_id) {{
              await replyPermission(mapped._opencode_request_id, response);
            }}
          }} finally {{
            pending.delete(mapped.session_id);
          }}
          return;
        }}
        if (pending.has(mapped.session_id) && mapped.hook_event_name !== "SessionEnd") return;
        await send(mapped, false);
      }}
    }};
  }}
}};
"#
    .replace("__OPENCODE_PLUGIN_MARKER__", OPENCODE_PLUGIN_MARKER)
    .replace("__OPENCODE_PLUGIN_VERSION__", OPENCODE_PLUGIN_VERSION)
    .replace("__MEMORPH_EXE_JSON__", &exe)
    .replace("{{", "{")
    .replace("}}", "}");
    Ok(template)
}

fn merge_opencode_plugin_ref(original: Option<&str>, plugin_ref: &str) -> Result<String> {
    let Some(contents) = original.filter(|contents| !contents.trim().is_empty()) else {
        let mut root = Map::new();
        root.insert(
            "$schema".to_string(),
            Value::String("https://opencode.ai/config.json".to_string()),
        );
        root.insert(
            "plugin".to_string(),
            Value::Array(vec![Value::String(plugin_ref.to_string())]),
        );
        return Ok(serde_json::to_string_pretty(&Value::Object(root))? + "\n");
    };

    let parsed = parse_jsonc_object(contents)?;
    let mut plugins = parsed
        .get("plugin")
        .cloned()
        .and_then(|value| match value {
            Value::Array(values) => Some(values),
            Value::String(value) => Some(vec![Value::String(value)]),
            _ => None,
        })
        .unwrap_or_default();
    plugins.retain(|value| {
        value
            .as_str()
            .map(|entry| !entry.contains(OPENCODE_PLUGIN_FILE) && !entry.contains("vibe-island"))
            .unwrap_or(true)
    });
    plugins.push(Value::String(plugin_ref.to_string()));

    let mut merged = set_jsonc_top_level_value(contents, "plugin", &Value::Array(plugins))?;
    if !parsed.contains_key("$schema") {
        merged = set_jsonc_top_level_value(
            &merged,
            "$schema",
            &Value::String("https://opencode.ai/config.json".to_string()),
        )?;
    }
    Ok(ensure_trailing_newline(merged))
}

fn remove_opencode_plugin_ref(original: &str) -> Result<Option<String>> {
    let mut root = parse_jsonc_object(original)?;
    let Some(plugin_value) = root.remove("plugin") else {
        return Ok(None);
    };
    let mut plugins = match plugin_value {
        Value::Array(values) => values,
        Value::String(value) => vec![Value::String(value)],
        other => vec![other],
    };
    let original_len = plugins.len();
    plugins.retain(|value| {
        value
            .as_str()
            .map(|entry| !entry.contains(OPENCODE_PLUGIN_FILE))
            .unwrap_or(true)
    });
    if plugins.len() == original_len {
        return Ok(None);
    }
    if plugins.is_empty() {
        return Ok(Some(ensure_trailing_newline(delete_jsonc_top_level_key(
            original, "plugin",
        )?)));
    }
    Ok(Some(ensure_trailing_newline(set_jsonc_top_level_value(
        original,
        "plugin",
        &Value::Array(plugins),
    )?)))
}

#[derive(Debug, Clone, Copy)]
struct JsonTopLevelProperty {
    key_start: usize,
    value_start: usize,
    value_end: usize,
}

fn set_jsonc_top_level_value(contents: &str, key: &str, value: &Value) -> Result<String> {
    if let Some(property) = find_jsonc_top_level_property(contents, key) {
        let mut output = String::new();
        output.push_str(&contents[..property.value_start]);
        output.push_str(&serde_json::to_string_pretty(value)?);
        output.push_str(&contents[property.value_end..]);
        return Ok(output);
    }

    let close_index = find_jsonc_top_level_object_close(contents)
        .with_context(|| "Failed to find top-level JSON object close")?;
    let parsed = parse_jsonc_object(contents)?;
    let mut output = String::new();
    output.push_str(&contents[..close_index]);
    if parsed.is_empty() {
        output.push_str(&format!(
            "\n  \"{}\": {}\n",
            escape_json_key(key),
            serde_json::to_string_pretty(value)?
        ));
    } else {
        output.push_str(&format!(
            ",\n  \"{}\": {}\n",
            escape_json_key(key),
            serde_json::to_string_pretty(value)?
        ));
    }
    output.push_str(&contents[close_index..]);
    Ok(output)
}

fn delete_jsonc_top_level_key(contents: &str, key: &str) -> Result<String> {
    let Some(property) = find_jsonc_top_level_property(contents, key) else {
        return Ok(contents.to_string());
    };
    let mut start = property.key_start;
    while start > 0 {
        let previous = contents.as_bytes()[start - 1];
        if previous == b' ' || previous == b'\t' {
            start -= 1;
            continue;
        }
        break;
    }
    let mut end = skip_jsonc_ws_comments(contents, property.value_end);
    if contents.as_bytes().get(end) == Some(&b',') {
        end += 1;
        if contents.as_bytes().get(end) == Some(&b'\n') {
            end += 1;
        }
    } else {
        let mut previous = start;
        while previous > 0 && contents.as_bytes()[previous - 1].is_ascii_whitespace() {
            previous -= 1;
        }
        if previous > 0 && contents.as_bytes()[previous - 1] == b',' {
            start = previous - 1;
        }
    }
    let mut output = String::new();
    output.push_str(&contents[..start]);
    output.push_str(&contents[end..]);
    Ok(output)
}

fn find_jsonc_top_level_property(contents: &str, key: &str) -> Option<JsonTopLevelProperty> {
    let mut idx = 0;
    let mut depth = 0i32;
    while idx < contents.len() {
        idx = skip_jsonc_ws_comments(contents, idx);
        let ch = contents[idx..].chars().next()?;
        if ch == '"' && depth == 1 {
            let key_end = find_json_string_end(contents, idx)?;
            let parsed_key: String = serde_json::from_str(&contents[idx..key_end]).ok()?;
            let colon = skip_jsonc_ws_comments(contents, key_end);
            if contents.as_bytes().get(colon) == Some(&b':') {
                let value_start = skip_jsonc_ws_comments(contents, colon + 1);
                let value_end = find_jsonc_value_end(contents, value_start)?;
                if parsed_key == key {
                    return Some(JsonTopLevelProperty {
                        key_start: idx,
                        value_start,
                        value_end: trim_ascii_ws_end(contents, value_start, value_end),
                    });
                }
                idx = value_end;
                continue;
            }
        }
        match ch {
            '"' => idx = find_json_string_end(contents, idx)?,
            '{' | '[' => {
                depth += 1;
                idx += ch.len_utf8();
            }
            '}' | ']' => {
                depth -= 1;
                idx += ch.len_utf8();
            }
            _ => idx += ch.len_utf8(),
        }
    }
    None
}

fn find_jsonc_top_level_object_close(contents: &str) -> Option<usize> {
    let mut idx = 0;
    let mut depth = 0i32;
    while idx < contents.len() {
        idx = skip_jsonc_ws_comments(contents, idx);
        let ch = contents[idx..].chars().next()?;
        match ch {
            '"' => idx = find_json_string_end(contents, idx)?,
            '{' => {
                depth += 1;
                idx += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
                idx += 1;
            }
            '[' => {
                depth += 1;
                idx += 1;
            }
            ']' => {
                depth -= 1;
                idx += 1;
            }
            _ => idx += ch.len_utf8(),
        }
    }
    None
}

fn find_jsonc_value_end(contents: &str, start: usize) -> Option<usize> {
    let mut idx = start;
    let mut nested = 0i32;
    while idx < contents.len() {
        idx = skip_jsonc_ws_comments(contents, idx);
        let ch = contents[idx..].chars().next()?;
        match ch {
            '"' => idx = find_json_string_end(contents, idx)?,
            '{' | '[' => {
                nested += 1;
                idx += 1;
            }
            '}' => {
                if nested == 0 {
                    return Some(idx);
                }
                nested -= 1;
                idx += 1;
            }
            ']' => {
                nested -= 1;
                idx += 1;
            }
            ',' if nested == 0 => return Some(idx),
            _ => idx += ch.len_utf8(),
        }
    }
    Some(contents.len())
}

fn find_json_string_end(contents: &str, start: usize) -> Option<usize> {
    let mut escaped = false;
    let mut iter = contents[start + 1..].char_indices();
    while let Some((offset, ch)) = iter.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some(start + 1 + offset + ch.len_utf8());
        }
    }
    None
}

fn skip_jsonc_ws_comments(contents: &str, mut idx: usize) -> usize {
    loop {
        while idx < contents.len() && contents.as_bytes()[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if contents[idx..].starts_with("//") {
            idx = contents[idx..]
                .find('\n')
                .map(|offset| idx + offset + 1)
                .unwrap_or(contents.len());
            continue;
        }
        if contents[idx..].starts_with("/*") {
            idx = contents[idx + 2..]
                .find("*/")
                .map(|offset| idx + 2 + offset + 2)
                .unwrap_or(contents.len());
            continue;
        }
        return idx;
    }
}

fn trim_ascii_ws_end(contents: &str, start: usize, mut end: usize) -> usize {
    while end > start && contents.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end
}

fn ensure_trailing_newline(mut contents: String) -> String {
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents
}

fn escape_json_key(key: &str) -> String {
    serde_json::to_string(key)
        .unwrap_or_else(|_| format!("\"{key}\""))
        .trim_matches('"')
        .to_string()
}

fn parse_jsonc_object(contents: &str) -> Result<Map<String, Value>> {
    match serde_json::from_str::<Value>(&strip_json_comments(contents))
        .context("Failed to parse JSON/JSONC object")?
    {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("Expected JSON object"),
    }
}

fn strip_json_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            result.push(ch);
            continue;
        }
        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut prev = '\0';
                    for next in chars.by_ref() {
                        if prev == '*' && next == '/' {
                            break;
                        }
                        prev = next;
                    }
                    continue;
                }
                _ => {}
            }
        }
        result.push(ch);
    }
    result
}

fn backup_if_exists(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = path.with_extension(format!(
        "json.{SETTINGS_BACKUP_SUFFIX}.{}",
        Utc::now().format("%Y%m%d%H%M%S")
    ));
    fs::copy(path, &backup).with_context(|| {
        format!(
            "Failed to write Claude settings backup: {}",
            backup.display()
        )
    })?;
    Ok(Some(backup))
}

fn enable_codex_hooks_config() -> Result<bool> {
    let path = codex_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Codex config directory: {}",
                parent.display()
            )
        })?;
    }
    let original = fs::read_to_string(&path).unwrap_or_default();
    let updated = ensure_codex_hooks_feature_enabled(&original);
    if updated == original {
        return Ok(false);
    }
    atomic_write::write_string_atomic(&path, &updated)?;
    Ok(true)
}

pub(crate) fn codex_hooks_feature_enabled(contents: &str) -> bool {
    let mut in_features = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_features = trimmed == "[features]";
            continue;
        }
        if in_features && is_toml_bool_assignment(trimmed, "hooks", true) {
            return true;
        }
    }
    false
}

fn ensure_codex_hooks_feature_enabled(contents: &str) -> String {
    let mut lines: Vec<String> = contents
        .replace("\r\n", "\n")
        .lines()
        .filter(|line| !line.trim_start().starts_with("codex_hooks"))
        .map(ToString::to_string)
        .collect();
    let had_trailing_newline = contents.ends_with('\n') || contents.is_empty();

    let mut features_start = None;
    let mut features_end = lines.len();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if trimmed == "[features]" {
                features_start = Some(idx);
                features_end = lines.len();
            } else if features_start.is_some() {
                features_end = idx;
                break;
            }
        }
    }

    if let Some(start) = features_start {
        for line in lines.iter_mut().take(features_end).skip(start + 1) {
            if toml_assignment_key(line.trim()) == Some("hooks") {
                if is_toml_bool_assignment(line.trim(), "hooks", true) {
                    return join_lines(lines, had_trailing_newline);
                }
                *line = "hooks = true".to_string();
                return join_lines(lines, true);
            }
        }
        lines.insert(start + 1, "hooks = true".to_string());
        return join_lines(lines, true);
    }

    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push("[features]".to_string());
    lines.push("hooks = true".to_string());
    join_lines(lines, true)
}

fn is_toml_bool_assignment(line: &str, key: &str, expected: bool) -> bool {
    if toml_assignment_key(line) != Some(key) {
        return false;
    }
    line.split_once('=')
        .map(|(_, value)| {
            value
                .split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case(if expected { "true" } else { "false" })
        })
        .unwrap_or(false)
}

fn toml_assignment_key(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    trimmed.split_once('=').map(|(key, _)| key.trim())
}

fn toml_string_assignment_value(line: &str, key: &str) -> Option<String> {
    if toml_assignment_key(line) != Some(key) {
        return None;
    }
    let value = line.split_once('=')?.1.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        serde_json::from_str(value).ok()
    } else if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        Some(value[1..value.len() - 1].to_string())
    } else {
        Some(
            value
                .split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string(),
        )
    }
}

pub(crate) fn cline_event_has_memorph_hook(event: &str) -> bool {
    cline_event_memorph_hook_version(event).is_some()
}

pub(crate) fn cline_event_memorph_hook_version(event: &str) -> Option<Option<String>> {
    cline_hooks_dirs().into_iter().find_map(|dir| {
        let path = dir.join(event);
        let contents = fs::read_to_string(path).ok()?;
        if !contents.contains(CLINE_HOOK_MARKER) || !command_contains_memorph_hook(&contents) {
            return None;
        }
        Some(command_managed_version(&contents))
    })
}

fn cline_preserved_hook_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("hook");
    path.with_file_name(format!("{file_name}.memorph-original"))
}

fn cline_hook_script(
    command_base: &str,
    event: &ClineHookEvent,
    preserved_hook_path: Option<&Path>,
) -> Result<String> {
    let command = format!(
        "{} --managed-version {} --provider cline --event {}{}",
        command_base,
        HOOK_MANAGED_VERSION,
        event.name,
        if event.blocking { " --blocking" } else { "" }
    );
    let preserved_hook = preserved_hook_path
        .map(|path| shell_quote(&path.to_string_lossy()))
        .unwrap_or_default();
    Ok(format!(
        "#!/bin/bash\n# {marker}\n# version: {version}\nINPUT=$(cat)\nMEMORPH_OUTPUT=$(printf '%s' \"$INPUT\" | {command} 2>/dev/null)\nORIGINAL_HOOK={preserved_hook}\nORIGINAL_OUTPUT=\"\"\nif [ -n \"$ORIGINAL_HOOK\" ] && [ -x \"$ORIGINAL_HOOK\" ]; then\n  ORIGINAL_OUTPUT=$(printf '%s' \"$INPUT\" | \"$ORIGINAL_HOOK\" 2>/dev/null)\nfi\nif printf '%s' \"$MEMORPH_OUTPUT\" | grep -q '\"cancel\"[[:space:]]*:[[:space:]]*true'; then\n  printf '%s' \"$MEMORPH_OUTPUT\"\nelif [ -n \"$ORIGINAL_OUTPUT\" ]; then\n  printf '%s' \"$ORIGINAL_OUTPUT\"\nelif [ -n \"$MEMORPH_OUTPUT\" ]; then\n  printf '%s' \"$MEMORPH_OUTPUT\"\nelse\n  printf '{{\"cancel\":false}}'\nfi\n",
        marker = CLINE_HOOK_MARKER,
        version = CLINE_HOOK_VERSION,
        command = command,
        preserved_hook = preserved_hook,
    ))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("Failed to chmod Cline hook file: {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn merge_traecli_hooks(contents: &str, command_base: &str) -> Result<String> {
    let cleaned = remove_traecli_hooks(contents);
    let mut lines: Vec<String> = cleaned
        .replace("\r\n", "\n")
        .split('\n')
        .map(ToString::to_string)
        .collect();

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let rendered = render_traecli_hooks(command_base)?;
    if let Some(hooks_index) = lines.iter().position(|line| {
        let trimmed = line.trim();
        line == trimmed
            && (trimmed == "hooks:"
                || trimmed == "hooks: []"
                || trimmed == "hooks: null"
                || trimmed == "hooks: ~")
    }) {
        lines[hooks_index] = "hooks:".to_string();
        let mut rendered_lines: Vec<String> = rendered.lines().map(ToString::to_string).collect();
        lines.splice(hooks_index + 1..hooks_index + 1, rendered_lines.drain(..));
    } else {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("hooks:".to_string());
        lines.extend(rendered.lines().map(ToString::to_string));
    }

    let mut output = lines.join("\n");
    output.push('\n');
    Ok(output)
}

fn render_traecli_hooks(command_base: &str) -> Result<String> {
    let mut blocks = Vec::new();
    for event in TRAE_EVENTS {
        let command = format!(
            "{} --managed-version {} --provider trae --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        blocks.push(format!(
            "  - type: command\n    command: '{}'\n    timeout: '{}s'\n    matchers:\n      - event: {}",
            yaml_single_quote(&command),
            event.timeout_sec,
            event.name
        ));
    }
    Ok(blocks.join("\n"))
}

fn remove_traecli_hooks(contents: &str) -> String {
    let normalized = contents.replace("\r\n", "\n");
    let had_trailing_newline = normalized.ends_with('\n');
    let lines: Vec<String> = normalized.lines().map(ToString::to_string).collect();
    let mut result = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        let line = &lines[idx];
        let trimmed = line.trim_start();
        if is_traecli_hook_item_start(trimmed) {
            let indent = line.len() - trimmed.len();
            let mut next = idx + 1;
            while next < lines.len() {
                let candidate = &lines[next];
                let candidate_trimmed = candidate.trim_start();
                let candidate_indent = candidate.len() - candidate_trimmed.len();
                if candidate_indent == indent && candidate_trimmed.starts_with("- ") {
                    break;
                }
                if candidate_indent < indent && !candidate_trimmed.trim().is_empty() {
                    break;
                }
                next += 1;
            }
            if block_contains_memorph_command(&lines[idx..next]) {
                idx = next;
                continue;
            }
            result.extend(lines[idx..next].iter().cloned());
            idx = next;
            continue;
        }
        result.push(line.clone());
        idx += 1;
    }
    while result.last().is_some_and(|line| line.trim().is_empty()) {
        result.pop();
    }
    join_lines(result, had_trailing_newline)
}

fn traecli_hook_blocks_from_contents(contents: &str) -> Vec<Vec<String>> {
    let lines: Vec<String> = contents
        .replace("\r\n", "\n")
        .lines()
        .map(ToString::to_string)
        .collect();
    let mut blocks = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        let line = &lines[idx];
        let trimmed = line.trim_start();
        if is_traecli_hook_item_start(trimmed) {
            let indent = line.len() - trimmed.len();
            let mut next = idx + 1;
            while next < lines.len() {
                let candidate = &lines[next];
                let candidate_trimmed = candidate.trim_start();
                let candidate_indent = candidate.len() - candidate_trimmed.len();
                if candidate_indent == indent && candidate_trimmed.starts_with("- ") {
                    break;
                }
                if candidate_indent < indent && !candidate_trimmed.trim().is_empty() {
                    break;
                }
                next += 1;
            }
            blocks.push(lines[idx..next].to_vec());
            idx = next;
        } else {
            idx += 1;
        }
    }
    blocks
}

fn is_traecli_hook_item_start(trimmed: &str) -> bool {
    trimmed == "- type: command"
        || trimmed.starts_with("- type: command ")
        || trimmed.starts_with("- type: command #")
}

fn traecli_block_event(block: &[String]) -> Option<String> {
    for (idx, line) in block.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(raw) = trimmed.strip_prefix("- event:") {
            return Some(parse_yaml_scalar(raw));
        }
        if trimmed == "matchers:" {
            for matcher_line in block.iter().skip(idx + 1) {
                let matcher_trimmed = matcher_line.trim();
                if let Some(raw) = matcher_trimmed.strip_prefix("- event:") {
                    return Some(parse_yaml_scalar(raw));
                }
                if matcher_trimmed.starts_with("- ") && !matcher_trimmed.starts_with("- event:") {
                    break;
                }
            }
        }
    }
    None
}

fn block_contains_memorph_command(block: &[String]) -> bool {
    block
        .iter()
        .filter_map(|line| yaml_assignment_value(line.trim(), "command"))
        .any(|command| command_contains_memorph_hook(&command))
}

fn traecli_block_managed_version(block: &[String]) -> Option<String> {
    block
        .iter()
        .filter_map(|line| yaml_assignment_value(line.trim(), "command"))
        .find_map(|command| {
            command_contains_memorph_hook(&command).then(|| command_managed_version(&command))
        })
        .flatten()
}

fn yaml_assignment_value(line: &str, key: &str) -> Option<String> {
    let raw = line.strip_prefix(key)?.strip_prefix(':')?;
    Some(parse_yaml_scalar(raw))
}

fn parse_yaml_scalar(raw: &str) -> String {
    let value = raw.split('#').next().unwrap_or_default().trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return serde_json::from_str(value)
            .unwrap_or_else(|_| value[1..value.len() - 1].to_string());
    }
    value.to_string()
}

fn yaml_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn kimi_hook_blocks(command_base: &str) -> Result<String> {
    let mut blocks = Vec::new();
    for event in KIMI_EVENTS {
        let command = format!(
            "{} --managed-version {} --provider kimi --event {}{}",
            command_base,
            HOOK_MANAGED_VERSION,
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        let mut block = format!(
            "[[hooks]]\nevent = {}\ncommand = {}\ntimeout = {}",
            serde_json::to_string(event.name)?,
            serde_json::to_string(&command)?,
            event.timeout
        );
        if let Some(matcher) = event.matcher {
            block.push_str(&format!("\nmatcher = {}", serde_json::to_string(matcher)?));
        }
        blocks.push(block);
    }
    Ok(blocks.join("\n\n") + "\n")
}

fn remove_kimi_hooks(contents: &str) -> String {
    let lines: Vec<String> = contents
        .replace("\r\n", "\n")
        .lines()
        .map(ToString::to_string)
        .collect();
    let had_trailing_newline = contents.ends_with('\n');
    let mut result = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if lines[idx].trim() == "[[hooks]]" {
            let mut block = vec![lines[idx].clone()];
            let mut next = idx + 1;
            while next < lines.len() {
                let trimmed = lines[next].trim();
                if trimmed.starts_with('[') {
                    break;
                }
                block.push(lines[next].clone());
                next += 1;
            }
            if !block_contains_memorph_hook(&block) {
                result.extend(block);
            }
            idx = next;
        } else {
            result.push(lines[idx].clone());
            idx += 1;
        }
    }
    while result.last().is_some_and(|line| line.trim().is_empty()) {
        result.pop();
    }
    join_lines(result, had_trailing_newline)
}

fn kimi_hook_blocks_from_contents(contents: &str) -> Vec<Vec<String>> {
    let lines: Vec<String> = contents
        .replace("\r\n", "\n")
        .lines()
        .map(ToString::to_string)
        .collect();
    let mut blocks = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if lines[idx].trim() == "[[hooks]]" {
            let mut block = vec![lines[idx].clone()];
            idx += 1;
            while idx < lines.len() {
                let trimmed = lines[idx].trim();
                if trimmed.starts_with('[') {
                    break;
                }
                block.push(lines[idx].clone());
                idx += 1;
            }
            blocks.push(block);
        } else {
            idx += 1;
        }
    }
    blocks
}

fn kimi_block_event(block: &[String]) -> Option<String> {
    block
        .iter()
        .find_map(|line| toml_string_assignment_value(line.trim(), "event"))
}

fn join_lines(lines: Vec<String>, trailing_newline: bool) -> String {
    let mut output = lines.join("\n");
    if trailing_newline && !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

fn ensure_object_field<'a>(
    root: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    if !root.get(key).map(Value::is_object).unwrap_or(false) {
        root.insert(key.to_string(), Value::Object(Map::new()));
    }
    root.get_mut(key).and_then(Value::as_object_mut).unwrap()
}

fn ensure_array_field<'a>(root: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    if !root.get(key).map(Value::is_array).unwrap_or(false) {
        root.insert(key.to_string(), Value::Array(Vec::new()));
    }
    root.get_mut(key).and_then(Value::as_array_mut).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_memorph_hook_inside_claude_entry() {
        let entry = json!({
            "matcher": "*",
            "hooks": [{"type": "command", "command": "memorph __hook-bridge --managed-version hook-v1 --provider claude"}]
        });
        assert!(entry_contains_memorph_hook(&entry));
        assert_eq!(
            entry_memorph_hook_version(&entry).flatten().as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn legacy_memorph_hook_has_no_managed_version() {
        let entry = json!({
            "matcher": "*",
            "hooks": [{"type": "command", "command": "memorph __hook-bridge --provider claude"}]
        });
        assert!(entry_contains_memorph_hook(&entry));
        assert_eq!(entry_memorph_hook_version(&entry), Some(None));
    }

    #[test]
    fn canonical_provider_id_accepts_profile_aliases() {
        assert_eq!(canonical_provider_id("traecli").unwrap(), "trae");
        assert_eq!(canonical_provider_id("claude-code").unwrap(), "claude");
    }

    #[test]
    fn renders_pi_and_omp_extensions_with_memorph_bridge() {
        let pi = pi_extension_source().unwrap();
        assert!(pi.contains("memorph pi extension"));
        assert!(pi.contains("__hook-bridge"));
        assert!(pi.contains("--provider"));
        assert!(pi.contains("const PROVIDER = \"pi\""));
        assert!(!pi.contains("codeisland-bridge"));
        assert!(!pi.contains("codeisland-"));

        let omp = omp_extension_source().unwrap();
        assert!(omp.contains("memorph omp extension"));
        assert!(omp.contains("__hook-bridge"));
        assert!(omp.contains("const PROVIDER = \"omp\""));
        assert!(!omp.contains("codeisland-bridge"));
        assert!(!omp.contains("codeisland-"));
    }

    #[test]
    fn detects_memorph_extension_versions() {
        let current = format!(
            "// memorph pi extension\n// version: {}\nmemorph __hook-bridge\n",
            HOOK_MANAGED_VERSION
        );
        assert_eq!(
            pi_extension_installed_version(&current)
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
        assert!(pi_extension_installed_version("// unrelated extension").is_none());
    }

    #[test]
    fn renders_cline_hook_script_with_valid_fallback_response() {
        let script = cline_hook_script(
            "memorph __hook-bridge",
            &ClineHookEvent {
                name: "PreToolUse",
                blocking: true,
            },
            None,
        )
        .unwrap();
        assert!(script.contains(CLINE_HOOK_MARKER));
        assert!(script.contains("--provider cline --event PreToolUse --blocking"));
        assert!(script.contains("printf '{\"cancel\":false}'"));
    }

    #[test]
    fn detects_memorph_hook_inside_cursor_flat_entry() {
        let mut root = Map::new();
        root.insert(
            "hooks".to_string(),
            json!({
                "beforeShellExecution": [{
                    "command": "memorph __hook-bridge --managed-version hook-v1 --provider cursor --event beforeShellExecution"
                }]
            }),
        );
        assert!(cursor_event_has_memorph_hook(&root, "beforeShellExecution"));
        assert_eq!(
            cursor_event_memorph_hook_version(&root, "beforeShellExecution")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn detects_memorph_hook_inside_copilot_entry() {
        let mut root = Map::new();
        root.insert("version".to_string(), json!(1));
        root.insert(
            "hooks".to_string(),
            json!({
                "preToolUse": [{
                    "type": "command",
                    "bash": "memorph __hook-bridge --managed-version hook-v1 --provider copilot --event preToolUse",
                    "timeoutSec": 5
                }]
            }),
        );
        assert!(copilot_event_has_memorph_hook(&root, "preToolUse"));
        assert_eq!(
            copilot_event_memorph_hook_version(&root, "preToolUse")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn detects_memorph_hook_inside_gemini_nested_entry() {
        let mut root = Map::new();
        root.insert(
            "hooks".to_string(),
            json!({
                "BeforeTool": [{
                    "hooks": [{
                        "type": "command",
                        "command": "memorph __hook-bridge --managed-version hook-v1 --provider gemini --event BeforeTool",
                        "timeout": 10000
                    }]
                }]
            }),
        );
        assert!(gemini_event_has_memorph_hook(&root, "BeforeTool"));
        assert_eq!(
            gemini_event_memorph_hook_version(&root, "BeforeTool")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn detects_memorph_hook_inside_qwen_entry() {
        let mut root = Map::new();
        root.insert(
            "hooks".to_string(),
            json!({
                "PreToolUse": [{
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "memorph __hook-bridge --managed-version hook-v1 --provider qwen --event PreToolUse",
                        "timeout": 5000
                    }]
                }]
            }),
        );
        assert!(qwen_event_has_memorph_hook(&root, "PreToolUse"));
        assert_eq!(
            qwen_event_memorph_hook_version(&root, "PreToolUse")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn detects_and_removes_traecli_yaml_hook_blocks() {
        let contents = r#"model: trae
hooks:
  - type: command
    command: 'memorph __hook-bridge --managed-version hook-v1 --provider trae --event pre_tool_use'
    timeout: '5s'
    matchers:
      - event: pre_tool_use
  - type: command
    command: 'echo keep'
    timeout: '5s'
    matchers:
      - event: session_start
"#;
        assert!(trae_contents_contains_memorph_hook(
            contents,
            "pre_tool_use"
        ));
        assert_eq!(
            trae_event_memorph_hook_version(contents, "pre_tool_use")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
        let cleaned = remove_traecli_hooks(contents);
        assert!(!cleaned.contains("__hook-bridge"));
        assert!(cleaned.contains("echo keep"));
    }

    #[test]
    fn merges_traecli_yaml_hooks_under_existing_hooks_key() {
        let merged =
            merge_traecli_hooks("model: trae\nhooks: []\n", "memorph __hook-bridge").unwrap();
        assert!(merged.contains("hooks:\n  - type: command"));
        assert!(merged.contains("--provider trae --event permission_request --blocking"));
        assert!(trae_contents_contains_memorph_hook(
            &merged,
            "session_start"
        ));
        assert!(trae_contents_contains_memorph_hook(
            &merged,
            "permission_request"
        ));
    }

    #[test]
    fn detects_and_removes_kimi_toml_hook_blocks() {
        let contents = r#"
model = "kimi"

[[hooks]]
event = "PreToolUse"
command = "memorph __hook-bridge --managed-version hook-v1 --provider kimi --event PreToolUse"
timeout = 5
matcher = ".*"

[[hooks]]
event = "UserPromptSubmit"
command = "echo keep"
timeout = 5
"#;
        assert!(kimi_contents_contains_memorph_hook(contents, "PreToolUse"));
        assert_eq!(
            kimi_event_memorph_hook_version(contents, "PreToolUse")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
        let cleaned = remove_kimi_hooks(contents);
        assert!(!cleaned.contains("__hook-bridge"));
        assert!(cleaned.contains("echo keep"));
    }

    #[test]
    fn detects_memorph_hook_inside_kiro_agent_entry() {
        let mut root = Map::new();
        root.insert(
            "hooks".to_string(),
            json!({
                "preToolUse": [{
                    "command": "memorph __hook-bridge --managed-version hook-v1 --provider kiro --event preToolUse",
                    "matcher": "*",
                    "timeout_ms": 5000
                }]
            }),
        );
        assert!(kiro_event_has_memorph_hook(&root, "preToolUse"));
        assert_eq!(
            kiro_event_memorph_hook_version(&root, "preToolUse")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    use std::sync::Mutex;

    static INSTALLER_E2E_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct TestHomeGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        _dir: tempfile::TempDir,
    }

    impl TestHomeGuard {
        fn new() -> Self {
            let lock = INSTALLER_E2E_LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let dir = tempfile::tempdir().unwrap();
            set_test_home_dir(Some(dir.path().to_path_buf()));
            Self {
                _lock: lock,
                _dir: dir,
            }
        }
    }

    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            set_test_home_dir(None);
        }
    }

    #[test]
    fn codeisland_provider_sources_are_registered_in_memorph_profiles() {
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/hooks/reference_codeisland/Sources/CodeIsland/ConfigInstaller.swift"
        ))
        .unwrap();

        let mut codeisland_sources = Vec::new();
        let mut rest = source.as_str();
        while let Some(index) = rest.find("source: \"") {
            rest = &rest[index + "source: \"".len()..];
            if let Some(end) = rest.find('"') {
                codeisland_sources.push(&rest[..end]);
                rest = &rest[end + 1..];
            } else {
                break;
            }
        }

        for source in codeisland_sources {
            if source.starts_with("codeisland-") {
                continue;
            }
            let provider = match source {
                "trae" => "trae_gui",
                "traecli" => "trae",
                other => other,
            };
            assert!(
                crate::hooks::profiles::supports_provider(provider),
                "CodeIsland provider source is not registered in memorph: {source} -> {provider}"
            );
        }
        assert!(crate::hooks::profiles::supports_provider("opencode"));
    }

    #[test]
    fn installs_repairs_and_uninstalls_codeisland_claude_fork_providers() {
        let _home = TestHomeGuard::new();
        for provider in [
            "qoder",
            "droid",
            "codebuddy",
            "codybuddycn",
            "stepfun",
            "antigravity",
            "workbuddy",
            "hermes",
        ] {
            assert_eq!(
                verify(provider).unwrap().status.status,
                HookHealthStatus::NotInstalled
            );

            let installed = install(provider).unwrap();
            assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
            assert!(installed.changed, "{provider} install should write hooks");

            let verified = verify(provider).unwrap();
            assert_eq!(verified.status.status, HookHealthStatus::InstalledOk);
            assert_eq!(
                verified.status.installed_version.as_deref(),
                Some(HOOK_MANAGED_VERSION)
            );

            let removed = uninstall(provider).unwrap();
            assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
            assert!(removed.changed, "{provider} uninstall should remove hooks");
        }
    }

    #[test]
    fn repairs_stale_codeisland_claude_fork_provider_hook_version() {
        let _home = TestHomeGuard::new();
        install("qoder").unwrap();
        let path = qoder_settings_path();
        let stale = std::fs::read_to_string(&path)
            .unwrap()
            .replace(HOOK_MANAGED_VERSION, "hook-old");
        atomic_write::write_string_atomic(&path, &stale).unwrap();

        assert_eq!(
            verify("qoder").unwrap().status.status,
            HookHealthStatus::InstalledStaleBinary
        );
        let repaired = repair("qoder").unwrap();
        assert_eq!(repaired.status.status, HookHealthStatus::InstalledOk);
        assert_eq!(
            verify("qoder").unwrap().status.installed_version.as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn installs_and_uninstalls_trae_gui_and_traecn_flat_hooks() {
        let _home = TestHomeGuard::new();
        for provider in ["trae_gui", "traecn"] {
            assert_eq!(
                verify(provider).unwrap().status.status,
                HookHealthStatus::NotInstalled
            );
            let installed = install(provider).unwrap();
            assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
            assert!(installed.changed);

            let path = if provider == "trae_gui" {
                trae_gui_hooks_path()
            } else {
                traecn_hooks_path()
            };
            let contents = std::fs::read_to_string(path).unwrap();
            assert!(contents.contains("__hook-bridge"));
            assert!(contents.contains(&format!("--provider {provider}")));

            let removed = uninstall(provider).unwrap();
            assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
        }
    }

    #[test]
    fn installs_and_uninstalls_pi_and_omp_extensions() {
        let _home = TestHomeGuard::new();
        for provider in ["pi", "omp"] {
            assert_eq!(
                verify(provider).unwrap().status.status,
                HookHealthStatus::NotInstalled
            );
            let installed = install(provider).unwrap();
            assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
            assert!(installed.changed);

            let path = if provider == "pi" {
                pi_extension_path()
            } else {
                omp_extension_path()
            };
            let contents = std::fs::read_to_string(&path).unwrap();
            assert!(contents.contains("__hook-bridge"));
            assert!(contents.contains(&format!("const PROVIDER = \"{provider}\"")));
            assert!(!contents.contains("codeisland-bridge"));

            let removed = uninstall(provider).unwrap();
            assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
            assert!(!path.exists());
        }
    }

    #[test]
    fn cline_install_preserves_and_restores_existing_user_hook_file() {
        let _home = TestHomeGuard::new();
        let dir = cline_hooks_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let hook_path = dir.join("PreToolUse");
        let original = "#!/bin/bash\nprintf '{\"cancel\":true,\"reason\":\"user hook\"}'\n";
        atomic_write::write_string_atomic(&hook_path, original).unwrap();

        let installed = install("cline").unwrap();
        assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
        let preserved_path = cline_preserved_hook_path(&hook_path);
        assert_eq!(std::fs::read_to_string(&preserved_path).unwrap(), original);
        let installed_script = std::fs::read_to_string(&hook_path).unwrap();
        assert!(installed_script.contains(CLINE_HOOK_MARKER));
        assert!(installed_script.contains("ORIGINAL_HOOK="));
        assert!(installed_script.contains("PreToolUse.memorph-original"));

        let removed = uninstall("cline").unwrap();
        assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
        assert_eq!(std::fs::read_to_string(&hook_path).unwrap(), original);
        assert!(!preserved_path.exists());
    }

    #[test]
    fn claude_fork_install_uninstall_preserves_foreign_json_hooks() {
        let _home = TestHomeGuard::new();
        let path = qoder_settings_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        atomic_write::write_string_atomic(
            &path,
            &serde_json::to_string_pretty(&json!({
                "theme": "dark",
                "hooks": {
                    "PreToolUse": [{
                        "matcher": "Bash",
                        "hooks": [{"type": "command", "command": "echo keep", "timeout": 1}]
                    }],
                    "CustomEvent": [{"command": "echo custom"}]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        install("qoder").unwrap();
        let installed = read_json_object_or_empty(&path).unwrap();
        assert_eq!(installed.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(
            installed["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert!(qoder_event_has_memorph_hook(&installed, "PreToolUse"));

        uninstall("qoder").unwrap();
        let removed = read_json_object_or_empty(&path).unwrap();
        assert_eq!(removed.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(
            removed["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert_eq!(
            removed["hooks"]["CustomEvent"][0]["command"].as_str(),
            Some("echo custom")
        );
        assert!(!qoder_event_has_memorph_hook(&removed, "PreToolUse"));
    }

    #[test]
    fn flat_json_install_uninstall_preserves_foreign_hooks() {
        let _home = TestHomeGuard::new();
        let path = trae_gui_hooks_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        atomic_write::write_string_atomic(
            &path,
            &serde_json::to_string_pretty(&json!({
                "version": 1,
                "hooks": {
                    "beforeShellExecution": [{"command": "echo keep"}],
                    "custom": [{"command": "echo custom"}]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        install("trae_gui").unwrap();
        let installed = read_json_object_or_empty(&path).unwrap();
        assert_eq!(installed.get("version").and_then(Value::as_i64), Some(1));
        assert_eq!(
            installed["hooks"]["beforeShellExecution"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert!(trae_gui_event_has_memorph_hook(
            &installed,
            "beforeShellExecution"
        ));

        uninstall("trae_gui").unwrap();
        let removed = read_json_object_or_empty(&path).unwrap();
        assert_eq!(removed.get("version").and_then(Value::as_i64), Some(1));
        assert_eq!(
            removed["hooks"]["beforeShellExecution"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert_eq!(
            removed["hooks"]["custom"][0]["command"].as_str(),
            Some("echo custom")
        );
        assert!(!trae_gui_event_has_memorph_hook(
            &removed,
            "beforeShellExecution"
        ));
    }

    #[test]
    fn kimi_install_uninstall_preserves_foreign_toml_hook_blocks() {
        let _home = TestHomeGuard::new();
        let path = kimi_config_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = r#"model = "kimi"

[[hooks]]
event = "PreToolUse"
command = "echo keep"
timeout = 5

[ui]
theme = "dark"
"#;
        atomic_write::write_string_atomic(&path, original).unwrap();

        install("kimi").unwrap();
        let installed = std::fs::read_to_string(&path).unwrap();
        assert!(installed.contains("command = \"echo keep\""));
        assert!(installed.contains("theme = \"dark\""));
        assert!(kimi_contents_contains_memorph_hook(
            &installed,
            "PreToolUse"
        ));

        uninstall("kimi").unwrap();
        let removed = std::fs::read_to_string(&path).unwrap();
        assert!(removed.contains("command = \"echo keep\""));
        assert!(removed.contains("theme = \"dark\""));
        assert!(!kimi_contents_contains_memorph_hook(&removed, "PreToolUse"));
    }

    #[test]
    fn traecli_install_uninstall_preserves_foreign_yaml_hook_blocks() {
        let _home = TestHomeGuard::new();
        let path = traecli_config_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = r#"model: trae
hooks:
  - type: command
    command: 'echo keep'
    timeout: '5s'
    matchers:
      - event: session_start
workspace: keep
"#;
        atomic_write::write_string_atomic(&path, original).unwrap();

        install("trae").unwrap();
        let installed = std::fs::read_to_string(&path).unwrap();
        assert!(installed.contains("command: 'echo keep'"));
        assert!(installed.contains("workspace: keep"));
        assert!(trae_contents_contains_memorph_hook(
            &installed,
            "session_start"
        ));

        uninstall("trae").unwrap();
        let removed = std::fs::read_to_string(&path).unwrap();
        assert!(removed.contains("command: 'echo keep'"));
        assert!(removed.contains("workspace: keep"));
        assert!(!trae_contents_contains_memorph_hook(
            &removed,
            "session_start"
        ));
    }

    #[test]
    fn codex_install_uninstall_preserves_foreign_hooks_and_config_toml() {
        let _home = TestHomeGuard::new();
        std::fs::create_dir_all(codex_home()).unwrap();
        atomic_write::write_string_atomic(
            &codex_hooks_path(),
            &serde_json::to_string_pretty(&json!({
                "hooks": {
                    "PreToolUse": [{"hooks": [{"type": "command", "command": "echo keep", "timeout": 1}]}],
                    "CustomEvent": [{"command": "echo custom"}]
                },
                "metadata": {"owner": "user"}
            }))
            .unwrap(),
        )
        .unwrap();
        atomic_write::write_string_atomic(
            &codex_config_path(),
            "model = \"gpt\"\n\n[features]\nexperimental = true\nhooks = false\n\n[workspace]\ntrust = true\n",
        )
        .unwrap();

        install("codex").unwrap();
        let installed_hooks = read_json_object_or_empty(&codex_hooks_path()).unwrap();
        assert_eq!(installed_hooks["metadata"]["owner"].as_str(), Some("user"));
        assert_eq!(
            installed_hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert!(codex_event_has_memorph_hook(&installed_hooks, "PreToolUse"));
        let installed_config = std::fs::read_to_string(codex_config_path()).unwrap();
        assert!(installed_config.contains("model = \"gpt\""));
        assert!(installed_config.contains("experimental = true"));
        assert!(installed_config.contains("hooks = true"));
        assert!(installed_config.contains("[workspace]"));

        uninstall("codex").unwrap();
        let removed_hooks = read_json_object_or_empty(&codex_hooks_path()).unwrap();
        assert_eq!(
            removed_hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert_eq!(
            removed_hooks["hooks"]["CustomEvent"][0]["command"].as_str(),
            Some("echo custom")
        );
        assert!(!codex_event_has_memorph_hook(&removed_hooks, "PreToolUse"));
        let removed_config = std::fs::read_to_string(codex_config_path()).unwrap();
        assert!(removed_config.contains("hooks = true"));
        assert!(removed_config.contains("[workspace]"));
    }

    #[test]
    fn opencode_install_uninstall_preserves_jsonc_comments_and_foreign_plugins() {
        let _home = TestHomeGuard::new();
        std::fs::create_dir_all(opencode_config_dir()).unwrap();
        let config_path = opencode_config_dir().join("opencode.jsonc");
        atomic_write::write_string_atomic(
            &config_path,
            "{\n  // keep this comment\n  \"plugin\": [\n    \"file:///tmp/keep.js\"\n  ],\n  \"theme\": \"dark\"\n}\n",
        )
        .unwrap();

        install("opencode").unwrap();
        let installed = std::fs::read_to_string(&config_path).unwrap();
        assert!(installed.contains("// keep this comment"));
        assert!(installed.contains("file:///tmp/keep.js"));
        assert!(installed.contains("memorph.js"));
        assert!(installed.contains("\"theme\": \"dark\""));

        uninstall("opencode").unwrap();
        let removed = std::fs::read_to_string(&config_path).unwrap();
        assert!(removed.contains("// keep this comment"));
        assert!(removed.contains("file:///tmp/keep.js"));
        assert!(!removed.contains("memorph.js"));
        assert!(removed.contains("\"theme\": \"dark\""));
    }

    #[test]
    fn command_base_contains_hidden_bridge_marker() {
        let command = bridge_command_base().unwrap();
        assert!(command.contains(HOOK_COMMAND_MARKER));
    }
}
