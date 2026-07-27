//! Helpers for JSON hook files that store entries under a top-level `hooks` object.

use std::fs;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::{Map, Value};

use crate::storage::atomic_write;

pub fn read_json_object_or_empty(path: &Path) -> Result<Map<String, Value>> {
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

pub fn write_json_object(path: &Path, root: &Map<String, Value>) -> Result<()> {
    let raw = serde_json::to_string_pretty(root)?;
    atomic_write::write_string_atomic(path, &(raw + "\n"))
}

pub fn ensure_object_field<'a>(
    root: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    if !root.get(key).map(Value::is_object).unwrap_or(false) {
        root.insert(key.to_string(), Value::Object(Map::new()));
    }
    root.get_mut(key).and_then(Value::as_object_mut).unwrap()
}

pub fn ensure_array_field<'a>(root: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    if !root.get(key).map(Value::is_array).unwrap_or(false) {
        root.insert(key.to_string(), Value::Array(Vec::new()));
    }
    root.get_mut(key).and_then(Value::as_array_mut).unwrap()
}

pub fn event_has_memorph_hook(root: &Map<String, Value>, event: &str) -> bool {
    root.get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().any(entry_contains_memorph_hook))
        .unwrap_or(false)
}

pub fn event_memorph_hook_version(
    root: &Map<String, Value>,
    event: &str,
) -> Option<Option<String>> {
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

pub fn entry_contains_memorph_hook(entry: &Value) -> bool {
    if let Some(command) = entry.get("command").and_then(Value::as_str) {
        if crate::hooks::shared::command_contains_memorph_hook(command) {
            return true;
        }
    }
    if let Some(command) = entry.get("bash").and_then(Value::as_str) {
        if crate::hooks::shared::command_contains_memorph_hook(command) {
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
                    .map(crate::hooks::shared::command_contains_memorph_hook)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub fn entry_memorph_hook_version(entry: &Value) -> Option<Option<String>> {
    if let Some(command) = entry.get("command").and_then(Value::as_str) {
        if crate::hooks::shared::command_contains_memorph_hook(command) {
            return Some(command_managed_version(command));
        }
    }
    if let Some(command) = entry.get("bash").and_then(Value::as_str) {
        if crate::hooks::shared::command_contains_memorph_hook(command) {
            return Some(command_managed_version(command));
        }
    }
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .and_then(|hooks| {
            hooks.iter().find_map(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .filter(|command| crate::hooks::shared::command_contains_memorph_hook(command))
                    .map(command_managed_version)
            })
        })
}

fn command_managed_version(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .find_map(|window| (window[0] == "--managed-version").then(|| window[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HOOK_MANAGED_VERSION: &str = crate::hooks::shared::HOOK_MANAGED_VERSION;

    #[test]
    fn detects_memorph_hook_inside_nested_command_entry() {
        let entry = json!({
            "matcher": "*",
            "hooks": [{"type": "command", "command": "memorph __hook-bridge --managed-version hook-v1 --provider sample"}]
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
            "hooks": [{"type": "command", "command": "memorph __hook-bridge --provider sample"}]
        });
        assert!(entry_contains_memorph_hook(&entry));
        assert_eq!(entry_memorph_hook_version(&entry), Some(None));
    }

    #[test]
    fn detects_memorph_hook_inside_flat_command_entry() {
        let mut root = Map::new();
        root.insert(
            "hooks".to_string(),
            json!({
                "beforeTool": [{
                    "command": "memorph __hook-bridge --managed-version hook-v1 --provider sample --event beforeTool"
                }]
            }),
        );
        assert!(event_has_memorph_hook(&root, "beforeTool"));
        assert_eq!(
            event_memorph_hook_version(&root, "beforeTool")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn detects_memorph_hook_inside_bash_command_entry() {
        let mut root = Map::new();
        root.insert(
            "hooks".to_string(),
            json!({
                "preToolUse": [{
                    "type": "command",
                    "bash": "memorph __hook-bridge --managed-version hook-v1 --provider sample --event preToolUse",
                    "timeoutSec": 5
                }]
            }),
        );
        assert!(event_has_memorph_hook(&root, "preToolUse"));
        assert_eq!(
            event_memorph_hook_version(&root, "preToolUse")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn detects_memorph_hook_inside_nested_hooks_array() {
        let mut root = Map::new();
        root.insert(
            "hooks".to_string(),
            json!({
                "BeforeTool": [{
                    "hooks": [{
                        "type": "command",
                        "command": "memorph __hook-bridge --managed-version hook-v1 --provider sample --event BeforeTool",
                        "timeout": 10000
                    }]
                }]
            }),
        );
        assert!(event_has_memorph_hook(&root, "BeforeTool"));
        assert_eq!(
            event_memorph_hook_version(&root, "BeforeTool")
                .flatten()
                .as_deref(),
            Some(HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn ignores_non_memorph_commands() {
        let entry = json!({"command": "echo not-a-memorph-hook"});
        assert!(!entry_contains_memorph_hook(&entry));
        assert_eq!(entry_memorph_hook_version(&entry), None);
    }

    #[test]
    fn empty_config_has_no_managed_hook() {
        let root = Map::new();
        assert!(!event_has_memorph_hook(&root, "beforeTool"));
        assert_eq!(event_memorph_hook_version(&root, "beforeTool"), None);
    }
}
