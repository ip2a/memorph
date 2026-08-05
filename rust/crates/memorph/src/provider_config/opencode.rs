use crate::provider_config::{entry_metadata, user_visible, ConfigRow, ConfigSource, ConfigTone, ConfigView};
use crate::provider_settings::{SettingDefinition, SettingKind, SettingScope};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub(crate) const VIEW_SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "view_mcp",
        title: "MCP servers",
        description: "MCP servers configured for OpenCode.",
        scope: SettingScope::Global,
        kind: SettingKind::View,
    },
    SettingDefinition {
        id: "view_plugins",
        title: "Plugins",
        description: "OpenCode plugins and TUI plugin settings.",
        scope: SettingScope::Global,
        kind: SettingKind::View,
    },
];

pub fn inspect(view_id: &str) -> anyhow::Result<ConfigView> {
    let home = crate::config::effective_home_dir()?;
    match view_id {
        "view_mcp" => Ok(mcp(&home)),
        "view_plugins" => Ok(plugins(&home)),
        other => anyhow::bail!("Unknown OpenCode config view: {other}"),
    }
}

fn config_paths(home: &Path) -> Vec<PathBuf> {
    ["opencode.json", "opencode.jsonc", "config.json"]
        .into_iter()
        .map(|n| home.join(".config/opencode").join(n))
        .collect()
}
fn tui_paths(home: &Path) -> Vec<PathBuf> {
    ["tui.json", "tui.jsonc"]
        .into_iter()
        .map(|n| home.join(".config/opencode").join(n))
        .collect()
}
fn parse(path: &Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(path).ok()?;
    crate::hooks::config_formats::jsonc::parse_object(&text).ok()
}
fn first(paths: &[PathBuf]) -> Option<(PathBuf, Map<String, Value>)> {
    paths.iter().find_map(|p| parse(p).map(|v| (p.clone(), v)))
}
fn sources(view: &mut ConfigView, paths: &[PathBuf]) {
    for p in paths {
        view.sources.push(ConfigSource {
            path: user_visible(p),
            scope: "user",
            exists: p.is_file(),
        });
    }
}
fn secret_rows(rows: &mut Vec<ConfigRow>, value: &Value, key: &str, label: &str) {
    if let Some(map) = value
        .get(key)
        .and_then(Value::as_object)
        .filter(|m| !m.is_empty())
    {
        rows.push(
            ConfigRow::fact(
                label,
                format!(
                    "{} key(s): {}",
                    map.len(),
                    map.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
            )
            .with_hint("values hidden"),
        );
    }
}

fn mcp(home: &Path) -> ConfigView {
    let paths = config_paths(home);
    let mut view = ConfigView::new("opencode", "view_mcp", "MCP servers");
    sources(&mut view, &paths);
    let Some((source, root)) = first(&paths) else {
        view.push_issue(
            ConfigTone::Muted,
            "No valid OpenCode configuration was found; no MCP servers are configured.",
        );
        return view;
    };
    if let Some(servers) = root.get("mcp").and_then(Value::as_object) {
        for (name, cfg) in servers {
            let mut rows = Vec::new();
            let disabled = cfg
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || cfg.get("type").and_then(Value::as_str) == Some("disabled");
            let kind = if disabled {
                "disabled"
            } else {
                cfg.get("type")
                    .and_then(Value::as_str)
                    .unwrap_or(if cfg.get("command").is_some() {
                        "local"
                    } else {
                        "remote"
                    })
            };
            rows.push(ConfigRow::fact("Mode", kind));
            if let Some(v) = cfg.get("command") {
                rows.push(ConfigRow::fact("Command", v.clone()));
            }
            if let Some(v) = cfg.get("args") {
                rows.push(ConfigRow::fact("Args", v.clone()));
            }
            if let Some(v) = cfg.get("url") {
                rows.push(ConfigRow::fact("URL", v.clone()));
            }
            if let Some(v) = cfg.get("enabled") {
                rows.push(ConfigRow::fact("Enabled", v.clone()));
            }
            secret_rows(&mut rows, cfg, "environment", "Environment");
            secret_rows(&mut rows, cfg, "env", "Environment");
            secret_rows(&mut rows, cfg, "headers", "Headers");
            let source_name = source.file_name().and_then(|n| n.to_str()).unwrap_or("config.json");
            view.push_entry_section(name, rows, entry_metadata("opencode", "view_mcp", &format!("source:{source_name}:{name}"), cfg));
        }
    }
    if view.sections.is_empty() {
        view.push_issue(ConfigTone::Muted, "No MCP servers are configured.");
    }
    view
}

pub(crate) fn remove_mcp(entry_id: &str, expected: &str) -> anyhow::Result<super::RemovalReport> {
    let home = crate::config::effective_home_dir()?;
    let paths = config_paths(&home);
    let Some((path, root)) = first(&paths) else { return Ok(super::RemovalReport::already_absent("opencode", "view_mcp", entry_id)); };
    let text = std::fs::read_to_string(&path)?;
    let source_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("config.json");
    let Some(servers) = root.get("mcp").and_then(Value::as_object) else { return Ok(super::RemovalReport::already_absent("opencode", "view_mcp", entry_id)); };
    let found = servers.iter().find(|(name, cfg)| entry_metadata("opencode", "view_mcp", &format!("source:{source_name}:{name}"), cfg).entry_id == entry_id).map(|(name, cfg)| (name.clone(), cfg.clone()));
    let Some((name, cfg)) = found else { return Ok(super::RemovalReport::already_absent("opencode", "view_mcp", entry_id)); };
    if entry_metadata("opencode", "view_mcp", &format!("source:{source_name}:{name}"), &cfg).fingerprint != expected { return Err(super::RemovalError::Conflict.into()); }
    let updated = crate::hooks::config_formats::jsonc::delete_nested_object_key(&text, "mcp", &name)?;
    let backup = super::backup_config(&path)?;
    crate::storage::atomic_write::write_string_atomic(&path, &updated)?;
    Ok(super::RemovalReport::removed("opencode", "view_mcp", entry_id, backup))
}

fn plugins(home: &Path) -> ConfigView {
    let paths = config_paths(home);
    let tui = tui_paths(home);
    let mut view = ConfigView::new("opencode", "view_plugins", "Plugins");
    sources(&mut view, &paths);
    sources(&mut view, &tui);
    let config = first(&paths).map(|(_, v)| v);
    let tui_config = first(&tui).map(|(_, v)| v);
    if let Some(root) = config {
        if let Some(list) = root.get("plugin").and_then(Value::as_array) {
            for (i, p) in list.iter().enumerate() {
                view.push_section(
                    format!("plugin {}", i + 1),
                    vec![
                        ConfigRow::fact("Plugin", p.clone()),
                        ConfigRow::fact("Configured", true),
                    ],
                );
            }
        }
        if let Some(servers) = root.get("mcp").and_then(Value::as_object) {
            for (name, cfg) in servers {
                if let Some(list) = cfg.get("plugin").and_then(Value::as_array) {
                    for p in list {
                        view.push_section(
                            format!("{} plugin", name),
                            vec![ConfigRow::fact("Plugin", p.clone())],
                        );
                    }
                }
            }
        }
    }
    if let Some(root) = tui_config {
        let mut rows = Vec::new();
        for key in ["plugin", "plugin_enabled"] {
            if let Some(v) = root.get(key) {
                rows.push(ConfigRow::fact(key, v.clone()));
            }
        }
        if !rows.is_empty() {
            view.push_section("TUI", rows);
        }
    }
    if view.sections.is_empty() {
        view.push_issue(ConfigTone::Muted, "No plugins are configured.");
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jsonc_mcp_plugins_hide_secrets() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".config/opencode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("opencode.jsonc"),"{ // config\n \"mcp\": {\"local\": {\"type\":\"local\",\"command\":[\"run\"],\"environment\":{\"TOKEN\":\"secret\"}},\"off\": {\"type\":\"disabled\"}}, \"plugin\":[\"a\"] }").unwrap();
        std::fs::write(
            dir.join("tui.json"),
            "{\"plugin\":\"a\",\"plugin_enabled\":true}",
        )
        .unwrap();
        let v = mcp(d.path());
        let out = serde_json::to_string(&v).unwrap();
        assert!(out.contains("local") && out.contains("TOKEN") && !out.contains("secret"));
        assert_eq!(v.sections.len(), 2);
        let p = plugins(d.path());
        assert!(serde_json::to_string(&p)
            .unwrap()
            .contains("plugin_enabled"));
    }

    #[test]
    fn removal_preserves_jsonc_comments_and_unrelated_configuration() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".config/opencode");
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("opencode.jsonc");
        std::fs::write(&config, "{\n  // keep this comment\n  \"mcp\": {\n    \"remove\": { \"command\": [\"run\"] },\n    \"keep\": { \"command\": [\"stay\"] }\n  },\n  \"plugin\": [\"keep-me\"]\n}\n").unwrap();
        let view = mcp(d.path());
        let entry = view.sections.iter().find(|section| section.label == "remove").unwrap().entry.as_ref().unwrap();

        let updated = crate::hooks::config_formats::jsonc::delete_nested_object_key(
            &std::fs::read_to_string(config).unwrap(), "mcp", "remove").unwrap();
        assert!(updated.contains("keep this comment"));
        assert!(updated.contains("keep-me"));
        assert!(updated.contains("\\\"keep\\\"") || updated.contains("\"keep\""));
        assert!(!updated.contains("\"remove\""));
        assert_eq!(entry.removable, true);
    }
}
