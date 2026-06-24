//! Hook bridge entrypoint support.
//!
//! This is an internal execution path invoked by provider hook configuration.
//! It reads provider stdin JSON, enriches it with local context, forwards it to
//! the currently running memorph API endpoint, and emits no provider decision
//! output. Hook failures are fail-open by design.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::PathBuf;
use std::time::Duration;

use crate::hooks::protocol::{HookBridgeEnvironment, HookIngestRequest, HookIngestResponse};
use crate::hooks::store;

const HOOK_BRIDGE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRunOptions {
    pub provider: String,
    pub event: String,
    pub blocking: bool,
}

pub fn run_blocking(options: BridgeRunOptions) -> Result<()> {
    let raw = read_stdin_json()?;
    let request = build_request(options, raw);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("Failed to build hook bridge runtime")?;
    let response = runtime.block_on(send_request(&request));
    match response {
        Ok(response) => print_response(&request.provider, &request.event_name, &response),
        Err(error) => {
            let _ = store::append_error("hook_bridge", error.to_string());
            print_response(
                &request.provider,
                &request.event_name,
                &HookIngestResponse {
                    accepted: false,
                    event_ids: Vec::new(),
                    message: Some(error.to_string()),
                },
            );
        }
    }
    Ok(())
}

pub fn build_request(options: BridgeRunOptions, raw: Value) -> HookIngestRequest {
    let mut request = HookIngestRequest::new(options.provider, options.event, raw);
    request.blocking = options.blocking;
    request.environment = capture_environment();
    request
}

fn read_stdin_json() -> Result<Value> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("Failed to read hook stdin")?;
    if input.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&input).context("Failed to parse hook stdin as JSON")
}

fn capture_environment() -> HookBridgeEnvironment {
    let vars = capture_terminal_environment_vars(|key| std::env::var(key).ok());

    HookBridgeEnvironment {
        cwd: std::env::current_dir().ok(),
        pid: Some(std::process::id()),
        parent_pid: parent_pid(),
        pid_start_time: crate::hooks::lifecycle::process_start_time(std::process::id()),
        tty: std::env::var("TTY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| crate::hooks::lifecycle::process_tty(std::process::id())),
        shell: std::env::var("SHELL").ok(),
        process_ancestry: crate::hooks::lifecycle::process_ancestry(std::process::id(), 6),
        vars,
    }
}

fn capture_terminal_environment_vars(
    mut get: impl FnMut(&str) -> Option<String>,
) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    for key in TERMINAL_ENV_KEYS {
        if let Some(value) = get(key).map(|value| value.trim().to_string()) {
            if !value.is_empty() {
                vars.insert((*key).to_string(), value);
            }
        }
    }
    vars
}

const TERMINAL_ENV_KEYS: &[&str] = &[
    // Generic terminal identity.
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "TERM_SESSION_ID",
    "COLORTERM",
    // iTerm2.
    "ITERM_SESSION_ID",
    "ITERM_PROFILE",
    // tmux.
    "TMUX",
    "TMUX_PANE",
    // Kitty.
    "KITTY_WINDOW_ID",
    "KITTY_LISTEN_ON",
    // Ghostty.
    "GHOSTTY_RESOURCES_DIR",
    "GHOSTTY_BIN_DIR",
    "GHOSTTY_SHELL_INTEGRATION_NO_SUDO",
    // Zellij.
    "ZELLIJ",
    "ZELLIJ_PANE_ID",
    "ZELLIJ_SESSION_NAME",
    // WezTerm and Kaku.
    "WEZTERM_PANE",
    "WEZTERM_UNIX_SOCKET",
    // Warp.
    "WARP_IS_LOCAL_SHELL_SESSION",
    "WARP_SESSION_ID",
    // VS Code / Cursor integrated terminals.
    "VSCODE_INJECTION",
    "VSCODE_IPC_HOOK_CLI",
    "VSCODE_GIT_IPC_HANDLE",
    // cmux.
    "CMUX_SURFACE_ID",
    "CMUX_WORKSPACE_ID",
];

#[cfg(unix)]
fn parent_pid() -> Option<u32> {
    std::env::var("PPID")
        .ok()
        .and_then(|value| value.parse().ok())
}

#[cfg(not(unix))]
fn parent_pid() -> Option<u32> {
    None
}

async fn send_request(request: &HookIngestRequest) -> Result<HookIngestResponse> {
    let endpoint =
        store::load_server_runtime()?.context("No running memorph hook endpoint found")?;
    let url = format!(
        "{}/api/v1/hooks/ingest",
        endpoint.endpoint.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(HOOK_BRIDGE_TIMEOUT)
        .build()
        .context("Failed to build hook bridge HTTP client")?;
    let response = client
        .post(url)
        .header("x-memorph-hook-token", &endpoint.token)
        .json(request)
        .send()
        .await
        .context("Failed to send hook event to memorph")?;
    let status = response.status();
    let value: Value = response
        .json()
        .await
        .context("Failed to parse memorph hook response")?;
    if !status.is_success() {
        anyhow::bail!("memorph hook ingest failed: {} {}", status, value);
    }
    let data = value
        .get("data")
        .cloned()
        .context("memorph hook response did not include data")?;
    serde_json::from_value(data).context("Failed to decode memorph hook ingest response")
}

fn print_response(provider: &str, event_name: &str, response: &HookIngestResponse) {
    if let Some(value) = provider_response_json(provider, event_name, response) {
        println!("{value}");
    }
}

fn provider_response_json(
    provider: &str,
    event_name: &str,
    response: &HookIngestResponse,
) -> Option<Value> {
    crate::hooks::normalizer::adapter_for(provider)
        .map(|adapter| adapter.blocking_response_json(event_name, response))
        .unwrap_or_else(|| crate::hooks::contract::generic_blocking_response_json(response))
}

#[allow(dead_code)]
fn _path_for_tests(path: &str) -> PathBuf {
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_request_captures_required_fields() {
        let request = build_request(
            BridgeRunOptions {
                provider: "sample".to_string(),
                event: "PreToolUse".to_string(),
                blocking: true,
            },
            json!({"tool_name": "Bash"}),
        );
        assert_eq!(request.provider, "sample");
        assert_eq!(request.event_name, "PreToolUse");
        assert!(request.blocking);
        assert!(request.environment.pid.is_some());
    }

    #[test]
    fn captures_codeisland_style_terminal_environment_keys() {
        let vars = capture_terminal_environment_vars(|key| match key {
            "TERM" => Some("xterm-256color".to_string()),
            "ITERM_SESSION_ID" => Some("w0t0p0".to_string()),
            "TMUX_PANE" => Some("%12".to_string()),
            "KITTY_WINDOW_ID" => Some("7".to_string()),
            "ZELLIJ_PANE_ID" => Some("3".to_string()),
            "WEZTERM_PANE" => Some("42".to_string()),
            "CMUX_SURFACE_ID" => Some("surface-1".to_string()),
            "WARP_SESSION_ID" => Some("warp-1".to_string()),
            "GHOSTTY_RESOURCES_DIR" => Some("/Applications/Ghostty.app/Resources".to_string()),
            "TERM_PROGRAM_VERSION" => Some(" ".to_string()),
            _ => None,
        });

        assert_eq!(vars.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(
            vars.get("ITERM_SESSION_ID").map(String::as_str),
            Some("w0t0p0")
        );
        assert_eq!(vars.get("TMUX_PANE").map(String::as_str), Some("%12"));
        assert_eq!(vars.get("KITTY_WINDOW_ID").map(String::as_str), Some("7"));
        assert_eq!(vars.get("ZELLIJ_PANE_ID").map(String::as_str), Some("3"));
        assert_eq!(vars.get("WEZTERM_PANE").map(String::as_str), Some("42"));
        assert_eq!(
            vars.get("CMUX_SURFACE_ID").map(String::as_str),
            Some("surface-1")
        );
        assert_eq!(
            vars.get("WARP_SESSION_ID").map(String::as_str),
            Some("warp-1")
        );
        assert_eq!(
            vars.get("GHOSTTY_RESOURCES_DIR").map(String::as_str),
            Some("/Applications/Ghostty.app/Resources")
        );
        assert!(!vars.contains_key("TERM_PROGRAM_VERSION"));
    }

    #[test]
    fn provider_response_does_not_emit_decision_output() {
        let response = HookIngestResponse {
            accepted: true,
            event_ids: vec!["e1".to_string()],
            message: None,
        };
        assert!(provider_response_json("generic", "PreToolUse", &response).is_none());
        assert!(provider_response_json("claude", "PermissionRequest", &response).is_none());
    }

}
