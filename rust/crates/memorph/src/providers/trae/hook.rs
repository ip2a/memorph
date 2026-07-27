use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};
use crate::storage::atomic_write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TraeHookEvent {
    name: &'static str,
    timeout_sec: u64,
    blocking: bool,
}

const EVENTS: &[TraeHookEvent] = &[
    event("session_start", 5, false),
    event("session_end", 5, false),
    event("user_prompt_submit", 5, false),
    event("pre_tool_use", 5, false),
    event("post_tool_use", 5, false),
    event("post_tool_use_failure", 5, false),
    event("permission_request", 86400, true),
    event("notification", 86400, false),
    event("subagent_start", 5, false),
    event("subagent_stop", 5, false),
    event("stop", 5, false),
    event("pre_compact", 5, false),
    event("post_compact", 5, false),
];

const fn event(name: &'static str, timeout_sec: u64, blocking: bool) -> TraeHookEvent {
    TraeHookEvent {
        name,
        timeout_sec,
        blocking,
    }
}

pub struct TraeHook;

pub static TRAE_HOOK: TraeHook = TraeHook;

impl ProviderHook for TraeHook {
    fn provider_id(&self) -> &'static str {
        "trae"
    }

    fn status(&self) -> Result<HookInstallStatus> {
        status()
    }

    fn install(&self) -> Result<HookOperationReport> {
        install()
    }

    fn verify(&self) -> Result<HookOperationReport> {
        let status = self.status()?;
        Ok(HookOperationReport {
            provider: self.provider_id().to_string(),
            operation: "verify".to_string(),
            changed: false,
            backup_path: None,
            message: status.message.clone(),
            status,
        })
    }

    fn repair(&self) -> Result<HookOperationReport> {
        let before = self.status()?;
        let mut report = self.install()?;
        report.operation = "repair".to_string();
        report.changed = before.status != HookHealthStatus::InstalledOk;
        Ok(report)
    }

    fn uninstall(&self) -> Result<HookOperationReport> {
        uninstall()
    }
}

pub(crate) fn config_path() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".trae")
        .join("traecli.yaml")
}

fn required_events() -> impl Iterator<Item = &'static str> {
    EVENTS.iter().map(|event| event.name)
}

fn status() -> Result<HookInstallStatus> {
    let path = config_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "trae".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some("TraeCli traecli.yaml does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("trae"),
        });
    }

    let contents = fs::read_to_string(&path)?;
    let missing: Vec<&str> = required_events()
        .filter(|event| !contents_contains_memorph_hook(&contents, event))
        .collect();
    let installed_version = crate::hooks::health::summarize_versions(
        required_events().filter_map(|event| event_memorph_hook_version(&contents, event)),
    );
    let current_version = Some(crate::hooks::shared::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref()
            != Some(crate::hooks::shared::current_hook_managed_version());
    let health = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == EVENTS.len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "TraeCli memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("TraeCli memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "trae".to_string(),
        status: health,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at("trae"),
    })
}

fn install() -> Result<HookOperationReport> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create TraeCli config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = fs::read_to_string(&path).unwrap_or_default();
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let command_base = crate::hooks::shared::bridge_command_base()?;
    let updated = merge_traecli_hooks(&original, &command_base)?;
    let changed = updated != original;
    if changed {
        atomic_write::write_string_atomic(&path, &updated)?;
    }
    let status = status()?;
    Ok(HookOperationReport {
        provider: "trae".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("TraeCli hook entries installed.".to_string()),
        status,
    })
}

fn uninstall() -> Result<HookOperationReport> {
    let path = config_path();
    if !path.exists() {
        let status = status()?;
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
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let updated = remove_traecli_hooks(&original);
    let changed = updated != original;
    if changed {
        atomic_write::write_string_atomic(&path, &updated)?;
    }
    let status = status()?;
    Ok(HookOperationReport {
        provider: "trae".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("TraeCli memorph hook entries removed.".to_string()),
    })
}

pub(crate) fn contents_contains_memorph_hook(contents: &str, event: &str) -> bool {
    hook_blocks_from_contents(contents)
        .into_iter()
        .any(|block| {
            block_event(&block).as_deref() == Some(event) && block_contains_memorph_command(&block)
        })
}

pub(crate) fn event_memorph_hook_version(contents: &str, event: &str) -> Option<Option<String>> {
    hook_blocks_from_contents(contents)
        .into_iter()
        .find(|block| {
            block_event(block).as_deref() == Some(event) && block_contains_memorph_command(block)
        })
        .map(|block| block_managed_version(&block))
}

pub(crate) fn merge_traecli_hooks(contents: &str, command_base: &str) -> Result<String> {
    let cleaned = remove_traecli_hooks(contents);
    let mut lines: Vec<String> = cleaned
        .replace("\r\n", "\n")
        .split('\n')
        .map(ToString::to_string)
        .collect();

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let rendered = render_hooks(command_base)?;
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

fn render_hooks(command_base: &str) -> Result<String> {
    let mut blocks = Vec::new();
    for event in EVENTS {
        let command = format!(
            "{} --managed-version {} --provider trae --event {}{}",
            command_base,
            crate::hooks::shared::current_hook_managed_version(),
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

pub(crate) fn remove_traecli_hooks(contents: &str) -> String {
    let normalized = contents.replace("\r\n", "\n");
    let had_trailing_newline = normalized.ends_with('\n');
    let lines: Vec<String> = normalized.lines().map(ToString::to_string).collect();
    let mut result = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        let line = &lines[idx];
        let trimmed = line.trim_start();
        if is_hook_item_start(trimmed) {
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

fn hook_blocks_from_contents(contents: &str) -> Vec<Vec<String>> {
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
        if is_hook_item_start(trimmed) {
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

fn is_hook_item_start(trimmed: &str) -> bool {
    trimmed == "- type: command"
        || trimmed.starts_with("- type: command ")
        || trimmed.starts_with("- type: command #")
}

fn block_event(block: &[String]) -> Option<String> {
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
        .any(|command| crate::hooks::shared::command_contains_memorph_hook(&command))
}

fn block_managed_version(block: &[String]) -> Option<String> {
    block
        .iter()
        .filter_map(|line| yaml_assignment_value(line.trim(), "command"))
        .find_map(|command| {
            crate::hooks::shared::command_contains_memorph_hook(&command)
                .then(|| command_managed_version(&command))
        })
        .flatten()
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

fn join_lines(lines: Vec<String>, trailing_newline: bool) -> String {
    let mut output = lines.join("\n");
    if trailing_newline && !output.ends_with('\n') {
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::test_support::TestHookHomeGuard;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = TRAE_HOOK.descriptor().expect("trae descriptor");
        assert_eq!(descriptor.provider(), TRAE_HOOK.provider_id());
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
        assert!(contents_contains_memorph_hook(contents, "pre_tool_use"));
        assert_eq!(
            event_memorph_hook_version(contents, "pre_tool_use")
                .flatten()
                .as_deref(),
            Some(crate::hooks::shared::HOOK_MANAGED_VERSION)
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
        assert!(contents_contains_memorph_hook(&merged, "session_start"));
        assert!(contents_contains_memorph_hook(
            &merged,
            "permission_request"
        ));
    }
    #[test]
    fn traecli_install_uninstall_preserves_foreign_yaml_hook_blocks() {
        let _home = TestHookHomeGuard::new();
        let path = config_path();
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
        crate::storage::atomic_write::write_string_atomic(&path, original).unwrap();

        TRAE_HOOK.install().unwrap();
        let installed = std::fs::read_to_string(&path).unwrap();
        assert!(installed.contains("command: 'echo keep'"));
        assert!(installed.contains("workspace: keep"));
        assert!(contents_contains_memorph_hook(&installed, "session_start"));

        TRAE_HOOK.uninstall().unwrap();
        let removed = std::fs::read_to_string(&path).unwrap();
        assert!(removed.contains("command: 'echo keep'"));
        assert!(removed.contains("workspace: keep"));
        assert!(!contents_contains_memorph_hook(&removed, "session_start"));
    }
}
