//! Read and manage hook entries already present in provider configuration files.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookSource {
    Memorph,
    ThirdParty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledHook {
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
pub struct InstalledHooks {
    pub provider: String,
    pub config_path: Option<String>,
    pub hooks: Vec<InstalledHook>,
}

pub fn list(provider: &str) -> Result<InstalledHooks> {
    let status = crate::hooks::operations::status(provider)?;
    let Some(path) = status
        .config_path
        .clone()
        .filter(|path| Path::new(path).is_file())
    else {
        return Ok(InstalledHooks {
            provider: provider.to_string(),
            config_path: status.config_path,
            hooks: Vec::new(),
        });
    };
    let root =
        crate::hooks::config_formats::json_hooks::read_json_object_or_empty(Path::new(&path))?;
    Ok(InstalledHooks {
        provider: provider.to_string(),
        config_path: Some(path),
        hooks: parse_json_hooks(&root),
    })
}

pub fn remove(
    provider: &str,
    event: &str,
    index: usize,
    expected_fingerprint: &str,
) -> Result<InstalledHooks> {
    let current = list(provider)?;
    let path = current
        .config_path
        .as_deref()
        .context("Hook configuration file is not available")?;
    let mut root =
        crate::hooks::config_formats::json_hooks::read_json_object_or_empty(Path::new(path))?;
    let entries = root
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .and_then(|hooks| hooks.get_mut(event))
        .and_then(Value::as_array_mut)
        .context("Hook event is not present")?;
    let entry = entries.get(index).context("Hook index is out of range")?;
    if !crate::hooks::config_formats::json_hooks::entry_contains_memorph_hook(entry) {
        anyhow::bail!("Only memorph-managed hooks can be removed");
    }
    if fingerprint(entry) != expected_fingerprint {
        anyhow::bail!("Hook changed; refresh the installed hooks list and try again");
    }
    entries.remove(index);
    crate::hooks::config_formats::json_hooks::write_json_object(Path::new(path), &root)?;
    list(provider)
}

fn parse_json_hooks(root: &Map<String, Value>) -> Vec<InstalledHook> {
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

fn parse_entry(event: &str, index: usize, entry: &Value) -> InstalledHook {
    let command = entry
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| entry.pointer("/hooks/0/command").and_then(Value::as_str))
        .map(str::to_string);
    let managed = command
        .as_deref()
        .map(crate::hooks::shared::command_contains_memorph_hook)
        .unwrap_or(false)
        || crate::hooks::config_formats::json_hooks::entry_contains_memorph_hook(entry);
    InstalledHook {
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
}
