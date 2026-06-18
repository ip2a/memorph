//! Terminal visibility and notification-suppression decisions.
//!
//! CodeIsland performs platform-specific checks with AppKit, AppleScript, and
//! terminal CLIs. memorph keeps the core decision logic side-effect free: the
//! Desktop/Tauri layer or another platform adapter supplies the currently
//! frontmost app/tab context, and this module evaluates runtime sessions against
//! their hook-captured terminal identity.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::hooks::model::{RuntimeSession, RuntimeSessionId};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TerminalVisibilityContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmost_bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmost_app_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_iterm_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tty: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tmux_pane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_kitty_window_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_wezterm_pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_zellij_pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_zellij_session_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_warp_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visible_ghostty_cwds: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityConfidence {
    None,
    App,
    Tab,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeVisibilityDecision {
    pub runtime_id: RuntimeSessionId,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    pub app_frontmost: bool,
    pub tab_visible: bool,
    pub suppress_notification: bool,
    pub confidence: VisibilityConfidence,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_by: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_context: Vec<String>,
}

pub fn evaluate_sessions(
    sessions: &[RuntimeSession],
    context: &TerminalVisibilityContext,
) -> Vec<RuntimeVisibilityDecision> {
    sessions
        .iter()
        .map(|session| evaluate_session(session, context))
        .collect()
}

pub fn evaluate_session(
    session: &RuntimeSession,
    context: &TerminalVisibilityContext,
) -> RuntimeVisibilityDecision {
    let app_frontmost = is_app_frontmost(session, context);
    let mut missing_context = Vec::new();

    if !app_frontmost && !ghostty_cwd_visible(session, context) {
        missing_context.push("frontmost_terminal_app".to_string());
        return decision(
            session,
            false,
            false,
            VisibilityConfidence::None,
            "terminal app is not frontmost or not known".to_string(),
            None,
            missing_context,
        );
    }

    if is_ide_integrated_terminal(session) {
        return decision(
            session,
            true,
            true,
            VisibilityConfidence::App,
            "IDE integrated terminal is frontmost; treating session as visible".to_string(),
            Some("ide_frontmost".to_string()),
            missing_context,
        );
    }

    if let Some(expected) = terminal_var(session, "ZELLIJ_PANE_ID") {
        match normalized_eq(Some(expected), context.active_zellij_pane_id.as_deref()) {
            Some(true) => {
                if let Some(expected_session) = terminal_var(session, "ZELLIJ_SESSION_NAME") {
                    if normalized_eq(
                        Some(expected_session),
                        context.active_zellij_session_name.as_deref(),
                    ) == Some(false)
                    {
                        return decision(
                            session,
                            true,
                            false,
                            VisibilityConfidence::App,
                            "frontmost terminal is active, but Zellij session differs".to_string(),
                            None,
                            missing_context,
                        );
                    }
                }
                return decision(
                    session,
                    true,
                    true,
                    VisibilityConfidence::Tab,
                    "active Zellij pane matches runtime session".to_string(),
                    Some("zellij_pane".to_string()),
                    missing_context,
                );
            }
            Some(false) => {
                return decision(
                    session,
                    true,
                    false,
                    VisibilityConfidence::App,
                    "frontmost terminal is active, but Zellij pane differs".to_string(),
                    None,
                    missing_context,
                );
            }
            None => missing_context.push("active_zellij_pane_id".to_string()),
        }
    }

    if let Some(expected) = terminal_var(session, "TMUX_PANE") {
        match normalized_eq(Some(expected), context.active_tmux_pane.as_deref()) {
            Some(true) => {
                return decision(
                    session,
                    true,
                    true,
                    VisibilityConfidence::Tab,
                    "active tmux pane matches runtime session".to_string(),
                    Some("tmux_pane".to_string()),
                    missing_context,
                );
            }
            Some(false) => {
                return decision(
                    session,
                    true,
                    false,
                    VisibilityConfidence::App,
                    "frontmost terminal is active, but tmux pane differs".to_string(),
                    None,
                    missing_context,
                );
            }
            None => missing_context.push("active_tmux_pane".to_string()),
        }
    }

    if let Some(expected) = terminal_var(session, "ITERM_SESSION_ID") {
        match normalized_eq(Some(expected), context.active_iterm_session_id.as_deref()) {
            Some(true) => {
                return decision(
                    session,
                    true,
                    true,
                    VisibilityConfidence::Tab,
                    "active iTerm session matches runtime session".to_string(),
                    Some("iterm_session_id".to_string()),
                    missing_context,
                );
            }
            Some(false) => {
                return decision(
                    session,
                    true,
                    false,
                    VisibilityConfidence::App,
                    "frontmost terminal is active, but iTerm session differs".to_string(),
                    None,
                    missing_context,
                );
            }
            None => missing_context.push("active_iterm_session_id".to_string()),
        }
    }

    if let Some(expected) = terminal_var(session, "KITTY_WINDOW_ID") {
        match normalized_eq(Some(expected), context.active_kitty_window_id.as_deref()) {
            Some(true) => {
                return decision(
                    session,
                    true,
                    true,
                    VisibilityConfidence::Tab,
                    "active kitty window matches runtime session".to_string(),
                    Some("kitty_window_id".to_string()),
                    missing_context,
                );
            }
            Some(false) => {
                return decision(
                    session,
                    true,
                    false,
                    VisibilityConfidence::App,
                    "frontmost terminal is active, but kitty window differs".to_string(),
                    None,
                    missing_context,
                );
            }
            None => missing_context.push("active_kitty_window_id".to_string()),
        }
    }

    if let Some(expected) = terminal_var(session, "WEZTERM_PANE") {
        match normalized_eq(Some(expected), context.active_wezterm_pane_id.as_deref()) {
            Some(true) => {
                return decision(
                    session,
                    true,
                    true,
                    VisibilityConfidence::Tab,
                    "active WezTerm pane matches runtime session".to_string(),
                    Some("wezterm_pane".to_string()),
                    missing_context,
                );
            }
            Some(false) => {
                return decision(
                    session,
                    true,
                    false,
                    VisibilityConfidence::App,
                    "frontmost terminal is active, but WezTerm pane differs".to_string(),
                    None,
                    missing_context,
                );
            }
            None => missing_context.push("active_wezterm_pane_id".to_string()),
        }
    }

    if let Some(expected) = terminal_var(session, "WARP_SESSION_ID") {
        match normalized_eq(Some(expected), context.active_warp_session_id.as_deref()) {
            Some(true) => {
                return decision(
                    session,
                    true,
                    true,
                    VisibilityConfidence::Tab,
                    "active Warp session matches runtime session".to_string(),
                    Some("warp_session_id".to_string()),
                    missing_context,
                );
            }
            Some(false) => {
                return decision(
                    session,
                    true,
                    false,
                    VisibilityConfidence::App,
                    "frontmost terminal is active, but Warp session differs".to_string(),
                    None,
                    missing_context,
                );
            }
            None => missing_context.push("active_warp_session_id".to_string()),
        }
    }

    if let Some(active_tty) = context.active_tty.as_deref() {
        if session.tty.as_deref() == Some(active_tty) {
            return decision(
                session,
                true,
                true,
                VisibilityConfidence::Tab,
                "active TTY matches runtime session".to_string(),
                Some("tty".to_string()),
                missing_context,
            );
        }
    } else if session.tty.is_some() {
        missing_context.push("active_tty".to_string());
    }

    if cwd_matches(session.cwd.as_deref(), context.active_cwd.as_deref()) {
        return decision(
            session,
            true,
            true,
            VisibilityConfidence::Tab,
            "active terminal cwd matches runtime session".to_string(),
            Some("cwd".to_string()),
            missing_context,
        );
    } else if session.cwd.is_some() {
        missing_context.push("active_cwd".to_string());
    }

    if ghostty_cwd_visible(session, context) {
        return decision(
            session,
            true,
            true,
            VisibilityConfidence::Tab,
            "visible Ghostty window cwd matches runtime session".to_string(),
            Some("ghostty_cwd".to_string()),
            missing_context,
        );
    }

    decision(
        session,
        true,
        false,
        VisibilityConfidence::App,
        "terminal app is frontmost, but no tab-level identity matched".to_string(),
        None,
        missing_context,
    )
}

fn decision(
    session: &RuntimeSession,
    app_frontmost: bool,
    tab_visible: bool,
    confidence: VisibilityConfidence,
    reason: String,
    matched_by: Option<String>,
    missing_context: Vec<String>,
) -> RuntimeVisibilityDecision {
    RuntimeVisibilityDecision {
        runtime_id: session.runtime_id.clone(),
        provider: session.provider.clone(),
        provider_session_id: session.provider_session_id.clone(),
        app_frontmost,
        tab_visible,
        suppress_notification: tab_visible,
        confidence,
        reason,
        matched_by,
        missing_context,
    }
}

fn is_app_frontmost(session: &RuntimeSession, context: &TerminalVisibilityContext) -> bool {
    let front_bundle = normalize_token(context.frontmost_bundle_id.as_deref());
    let front_name = normalize_token(context.frontmost_app_name.as_deref());
    let term_program = normalize_token(terminal_var(session, "TERM_PROGRAM"));

    if let Some(bundle) = front_bundle.as_deref() {
        if is_warp_session(session) && bundle == "dev.warp.warp-stable" {
            return true;
        }
        if is_ghostty_session(session) && bundle.contains("ghostty") {
            return true;
        }
        if is_iterm_session(session) && bundle.contains("iterm") {
            return true;
        }
        if is_kitty_session(session) && bundle.contains("kitty") {
            return true;
        }
        if is_wezterm_session(session) && bundle.contains("wezterm") {
            return true;
        }
        if bundle == "com.apple.terminal" && term_program.as_deref() == Some("apple_terminal") {
            return true;
        }
        if is_ide_integrated_terminal(session)
            && (bundle.contains("cursor")
                || bundle.contains("vscode")
                || bundle.contains("visualstudiocode")
                || bundle == "com.microsoft.vscode")
        {
            return true;
        }
    }

    if let Some(name) = front_name.as_deref() {
        if let Some(term) = term_program.as_deref() {
            if name.contains(term) || term.contains(name) {
                return true;
            }
        }
        if is_ghostty_session(session) && name.contains("ghostty") {
            return true;
        }
        if is_warp_session(session) && name.contains("warp") {
            return true;
        }
        if is_wezterm_session(session) && name.contains("wezterm") {
            return true;
        }
        if is_kitty_session(session) && name.contains("kitty") {
            return true;
        }
    }

    false
}

fn is_ide_integrated_terminal(session: &RuntimeSession) -> bool {
    session.terminal_vars.contains_key("VSCODE_INJECTION")
        || session.terminal_vars.contains_key("VSCODE_IPC_HOOK_CLI")
        || session.terminal_vars.contains_key("VSCODE_GIT_IPC_HANDLE")
}

fn is_iterm_session(session: &RuntimeSession) -> bool {
    session.terminal_vars.contains_key("ITERM_SESSION_ID")
        || terminal_var(session, "TERM_PROGRAM")
            .map(|value| value.to_ascii_lowercase().contains("iterm"))
            .unwrap_or(false)
}

fn is_ghostty_session(session: &RuntimeSession) -> bool {
    session.terminal_vars.contains_key("GHOSTTY_RESOURCES_DIR")
        || session.terminal_vars.contains_key("GHOSTTY_BIN_DIR")
        || terminal_var(session, "TERM_PROGRAM")
            .map(|value| value.to_ascii_lowercase().contains("ghostty"))
            .unwrap_or(false)
}

fn is_kitty_session(session: &RuntimeSession) -> bool {
    session.terminal_vars.contains_key("KITTY_WINDOW_ID")
        || session.terminal_vars.contains_key("KITTY_LISTEN_ON")
}

fn is_wezterm_session(session: &RuntimeSession) -> bool {
    session.terminal_vars.contains_key("WEZTERM_PANE")
        || terminal_var(session, "TERM_PROGRAM")
            .map(|value| value.to_ascii_lowercase().contains("wezterm"))
            .unwrap_or(false)
}

fn is_warp_session(session: &RuntimeSession) -> bool {
    session.terminal_vars.contains_key("WARP_SESSION_ID")
        || terminal_var(session, "TERM_PROGRAM")
            .map(|value| value.to_ascii_lowercase().contains("warp"))
            .unwrap_or(false)
}

fn ghostty_cwd_visible(session: &RuntimeSession, context: &TerminalVisibilityContext) -> bool {
    if !is_ghostty_session(session) {
        return false;
    }
    context
        .visible_ghostty_cwds
        .iter()
        .any(|cwd| cwd_matches(session.cwd.as_deref(), Some(cwd.as_path())))
}

fn terminal_var<'a>(session: &'a RuntimeSession, key: &str) -> Option<&'a str> {
    session.terminal_vars.get(key).map(String::as_str)
}

fn normalized_eq(expected: Option<&str>, actual: Option<&str>) -> Option<bool> {
    let expected = normalize_token(expected)?;
    let actual = normalize_token(actual)?;
    Some(expected == actual)
}

fn normalize_token(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    Some(
        value
            .trim_end_matches(".app")
            .replace(' ', "")
            .to_ascii_lowercase(),
    )
}

fn cwd_matches(expected: Option<&Path>, actual: Option<&Path>) -> bool {
    let Some(expected) = expected else {
        return false;
    };
    let Some(actual) = actual else {
        return false;
    };
    normalize_path(expected) == normalize_path(actual)
}

fn normalize_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    raw.trim_end_matches('/')
        .trim_start_matches("file://")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::BTreeMap;

    use crate::hooks::model::{RuntimeSession, RuntimeSessionStatus};

    fn session(vars: &[(&str, &str)]) -> RuntimeSession {
        RuntimeSession {
            runtime_id: RuntimeSessionId::new("runtime-1"),
            provider: "claude".to_string(),
            provider_session_id: Some("session-1".to_string()),
            run_id: None,
            cwd: Some(PathBuf::from("/tmp/project")),
            pid: None,
            parent_pid: None,
            pid_start_time: None,
            tty: Some("/dev/ttys001".to_string()),
            terminal_vars: vars
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect::<BTreeMap<_, _>>(),
            process_ancestry: Vec::new(),
            correlation: None,
            model: None,
            session_title: None,
            transcript_path: None,
            workspace_roots: Vec::new(),
            last_user_prompt: None,
            last_assistant_message: None,
            last_tool_result: None,
            last_error: None,
            stop_reason: None,
            compact_count: 0,
            tool_call_count: 0,
            failed_tool_count: 0,
            permission_request_count: 0,
            question_count: 0,
            status: RuntimeSessionStatus::Running,
            current_tool: None,
            pending_permission: None,
            pending_question: None,
            recent_activity: Vec::new(),
            subagents: BTreeMap::new(),
            last_event_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn suppresses_when_tmux_pane_matches() {
        let session = session(&[("TERM_PROGRAM", "iTerm.app"), ("TMUX_PANE", "%12")]);
        let decision = evaluate_session(
            &session,
            &TerminalVisibilityContext {
                frontmost_bundle_id: Some("com.googlecode.iterm2".to_string()),
                active_tmux_pane: Some("%12".to_string()),
                ..TerminalVisibilityContext::default()
            },
        );

        assert!(decision.app_frontmost);
        assert!(decision.tab_visible);
        assert!(decision.suppress_notification);
        assert_eq!(decision.matched_by.as_deref(), Some("tmux_pane"));
    }

    #[test]
    fn does_not_suppress_when_frontmost_tab_differs() {
        let session = session(&[
            ("TERM_PROGRAM", "iTerm.app"),
            ("ITERM_SESSION_ID", "w0t1p0"),
        ]);
        let decision = evaluate_session(
            &session,
            &TerminalVisibilityContext {
                frontmost_bundle_id: Some("com.googlecode.iterm2".to_string()),
                active_iterm_session_id: Some("w0t2p0".to_string()),
                ..TerminalVisibilityContext::default()
            },
        );

        assert!(decision.app_frontmost);
        assert!(!decision.tab_visible);
        assert!(!decision.suppress_notification);
    }

    #[test]
    fn treats_ide_integrated_terminal_as_visible_when_app_frontmost() {
        let session = session(&[("TERM_PROGRAM", "vscode"), ("VSCODE_INJECTION", "1")]);
        let decision = evaluate_session(
            &session,
            &TerminalVisibilityContext {
                frontmost_bundle_id: Some("com.microsoft.VSCode".to_string()),
                ..TerminalVisibilityContext::default()
            },
        );

        assert!(decision.suppress_notification);
        assert_eq!(decision.confidence, VisibilityConfidence::App);
        assert_eq!(decision.matched_by.as_deref(), Some("ide_frontmost"));
    }

    #[test]
    fn records_missing_precise_context_when_app_is_frontmost() {
        let session = session(&[
            ("TERM_PROGRAM", "iTerm.app"),
            ("ITERM_SESSION_ID", "w0t1p0"),
        ]);
        let decision = evaluate_session(
            &session,
            &TerminalVisibilityContext {
                frontmost_bundle_id: Some("com.googlecode.iterm2".to_string()),
                ..TerminalVisibilityContext::default()
            },
        );

        assert!(decision.app_frontmost);
        assert!(!decision.suppress_notification);
        assert!(decision
            .missing_context
            .contains(&"active_iterm_session_id".to_string()));
    }
}
