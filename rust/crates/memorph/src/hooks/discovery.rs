//! Read hook entries already present in provider configuration files.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookSource {
    Memorph,
    ThirdParty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedHook {
    pub event: String,
    pub index: usize,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub source: HookSource,
    pub managed_by_memorph: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedHooks {
    pub provider: String,
    pub scan_supported: bool,
    pub config_path: Option<String>,
    pub hooks: Vec<DetectedHook>,
}

pub fn list(provider: &str) -> Result<DetectedHooks> {
    let Some((provider, path)) = scan_target(provider) else {
        return Ok(DetectedHooks {
            provider: provider.to_string(),
            scan_supported: false,
            config_path: None,
            hooks: Vec::new(),
        });
    };
    let config_path = path.display().to_string();
    if !path.is_file() {
        return Ok(DetectedHooks {
            provider: provider.to_string(),
            scan_supported: true,
            config_path: Some(config_path),
            hooks: Vec::new(),
        });
    }
    let root = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    Ok(DetectedHooks {
        provider: provider.to_string(),
        scan_supported: true,
        config_path: Some(config_path),
        hooks: parse_json_hooks(&root),
    })
}

pub fn delete(
    provider: &str,
    event: &str,
    index: usize,
    fingerprint: &str,
) -> Result<crate::hooks::model::HookOperationReport> {
    let Some((provider, path)) = scan_target(provider) else {
        return Ok(crate::hooks::model::HookOperationReport {
            provider: provider.to_string(),
            operation: "delete".to_string(),
            changed: false,
            backup_path: None,
            message: Some("Hook deletion is not supported for this provider.".to_string()),
            status: crate::hooks::operations::status(provider)?,
        });
    };

    if !path.is_file() {
        return Ok(crate::hooks::model::HookOperationReport {
            provider: provider.to_string(),
            operation: "delete".to_string(),
            changed: false,
            backup_path: None,
            message: Some("Hook config file does not exist.".to_string()),
            status: crate::hooks::operations::status(provider)?,
        });
    }

    let original = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    let Some(target_index) = find_hook_index(&original, event, index, fingerprint) else {
        return Err(anyhow::anyhow!(
            "Hook entry not found for {} {} #{}",
            provider,
            event,
            index + 1
        ));
    };

    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let mut root = original.clone();
    let hooks = root
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("Expected hooks object in {}", path.display()))?;
    let entries = hooks
        .get_mut(event)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("Expected hook entries for {}", event))?;
    entries.remove(target_index);
    if entries.is_empty() {
        hooks.remove(event);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }

    let changed = root != original;
    if changed {
        crate::hooks::config_formats::json_hooks::write_json_object(&path, &root)?;
    }
    let status = crate::hooks::operations::status(provider)?;
    Ok(crate::hooks::model::HookOperationReport {
        provider: provider.to_string(),
        operation: "delete".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some(format!("Removed hook entry {} #{}.", event, index + 1)),
        status,
    })
}

fn find_hook_index(
    root: &Map<String, Value>,
    event: &str,
    index: usize,
    fingerprint: &str,
) -> Option<usize> {
    let entries = root
        .get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)?;

    if index < entries.len()
        && parse_entry(event, index, &entries[index]).fingerprint == fingerprint
    {
        return Some(index);
    }

    entries
        .iter()
        .enumerate()
        .find(|(entry_index, entry)| {
            parse_entry(event, *entry_index, entry).fingerprint == fingerprint
        })
        .map(|(entry_index, _)| entry_index)
}

pub fn supports_provider(provider: &str) -> bool {
    scan_target(provider).is_some()
}

fn scan_target(provider: &str) -> Option<(&'static str, PathBuf)> {
    let profile = crate::hooks::profiles::find(provider)?;
    let home = crate::hooks::shared::hook_home_dir();
    let path = match profile.provider {
        "claude" => home.join(".claude/settings.json"),
        "codex" => crate::providers::codex::hook::hooks_path(),
        "cursor" => home.join(".cursor/hooks.json"),
        "gemini" => home.join(".gemini/settings.json"),
        "qoder" => crate::providers::qoder::hook::settings_path(),
        "droid" => crate::providers::droid::hook::settings_path(),
        "codebuddy" => crate::providers::codebuddy::hook::settings_path(),
        "antigravity" => crate::providers::antigravity::hook::settings_path(),
        "workbuddy" => crate::providers::workbuddy::hook::settings_path(),
        "hermes" => crate::providers::hermes::hook::settings_path(),
        _ => return None,
    };
    Some((profile.provider, path))
}

fn parse_json_hooks(root: &Map<String, Value>) -> Vec<DetectedHook> {
    root.get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|hooks| hooks.iter())
        .flat_map(|(event, entries)| {
            entries.as_array().into_iter().flat_map(move |entries| {
                entries
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| parse_entry(event, index, entry))
            })
        })
        .collect()
}

fn fingerprint(entry: &Value) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(entry).unwrap()))
}

fn parse_entry(event: &str, index: usize, entry: &Value) -> DetectedHook {
    let command = entry
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| entry.get("bash").and_then(Value::as_str))
        .or_else(|| entry.pointer("/hooks/0/command").and_then(Value::as_str))
        .map(str::to_string);
    let managed = command
        .as_deref()
        .map(crate::hooks::shared::command_contains_memorph_hook)
        .unwrap_or(false)
        || crate::hooks::config_formats::json_hooks::entry_contains_memorph_hook(entry);
    DetectedHook {
        event: event.to_string(),
        index,
        fingerprint: fingerprint(entry),
        matcher: entry
            .get("matcher")
            .and_then(Value::as_str)
            .map(str::to_string),
        hook_type: entry
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| entry.pointer("/hooks/0/type").and_then(Value::as_str))
            .map(str::to_string),
        command,
        source: if managed {
            HookSource::Memorph
        } else {
            HookSource::ThirdParty
        },
        managed_by_memorph: managed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_memorph_and_foreign_entries() {
        let root = serde_json::from_value(serde_json::json!({"hooks": {"PreToolUse": [
            {"matcher":"*", "hooks":[{"type":"command", "command":"echo third-party"}]},
            {"matcher":"*", "hooks":[{"type":"command", "command":"memorph __hook-bridge"}]}
        ]}}))
        .unwrap();
        let hooks = parse_json_hooks(&root);
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].source, HookSource::ThirdParty);
        assert!(hooks[1].managed_by_memorph);
        assert_ne!(hooks[0].fingerprint, hooks[1].fingerprint);
    }

    #[test]
    fn fingerprint_changes_when_entry_changes() {
        let first = serde_json::json!({"command": "memorph --hook"});
        let second = serde_json::json!({"command": "memorph --hook --event"});
        assert_ne!(fingerprint(&first), fingerprint(&second));
    }

    #[test]
    fn reports_non_json_providers_as_unsupported() {
        let hooks = list("kimi").unwrap();
        assert!(!hooks.scan_supported);
        assert!(hooks.hooks.is_empty());
    }

    #[test]
    fn discovery_does_not_require_a_memorph_hook_installation() {
        let _home = crate::hooks::test_support::TestHookHomeGuard::new();
        let path = crate::hooks::shared::hook_home_dir().join(".claude/settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        crate::hooks::config_formats::json_hooks::write_json_object(
            &path,
            &serde_json::from_value(serde_json::json!({
                "hooks": {"PreToolUse": [{"command": "echo third-party"}]}
            }))
            .unwrap(),
        )
        .unwrap();

        let hooks = list("claude").unwrap();
        assert!(hooks.scan_supported);
        assert_eq!(hooks.hooks.len(), 1);
        assert_eq!(hooks.hooks[0].source, HookSource::ThirdParty);
    }

    #[test]
    fn delete_removes_only_one_detected_hook_entry() {
        let _home = crate::hooks::test_support::TestHookHomeGuard::new();
        let path = crate::hooks::shared::hook_home_dir().join(".claude/settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        crate::hooks::config_formats::json_hooks::write_json_object(
            &path,
            &serde_json::from_value(serde_json::json!({
                "hooks": {"PreToolUse": [
                    {"command": "echo third-party"},
                    {"command": "memorph __hook-bridge --managed-version hook-v1"}
                ]}
            }))
            .unwrap(),
        )
        .unwrap();

        let hooks = list("claude").unwrap();
        let target = hooks
            .hooks
            .iter()
            .find(|hook| !hook.managed_by_memorph)
            .unwrap();
        let report = delete("claude", &target.event, target.index, &target.fingerprint).unwrap();
        assert!(report.changed);

        let updated = list("claude").unwrap();
        assert_eq!(updated.hooks.len(), 1);
        assert!(updated.hooks[0].managed_by_memorph);
    }
}
