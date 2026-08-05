use crate::provider_config::{entry_metadata, user_visible, ConfigRow, ConfigSource, ConfigTone, ConfigView};
use crate::provider_settings::{SettingDefinition, SettingKind, SettingScope};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) const VIEW_SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "view_mcp",
        title: "MCP servers",
        description: "MCP servers configured for Codex.",
        scope: SettingScope::Global,
        kind: SettingKind::View,
    },
    SettingDefinition {
        id: "view_plugins",
        title: "Plugins",
        description: "Codex plugin configuration (not installation state).",
        scope: SettingScope::Global,
        kind: SettingKind::View,
    },
    SettingDefinition {
        id: "view_statusline",
        title: "Status line",
        description: "Codex TUI status line and terminal title configuration.",
        scope: SettingScope::Global,
        kind: SettingKind::View,
    },
];

pub fn inspect(view_id: &str) -> anyhow::Result<ConfigView> {
    let home = crate::config::effective_home_dir()?;
    match view_id {
        "view_mcp" => Ok(mcp(&home)),
        "view_plugins" => Ok(plugins(&home)),
        "view_statusline" => Ok(statusline(&home)),
        other => anyhow::bail!("Unknown Codex config view: {other}"),
    }
}

fn path(home: &Path) -> PathBuf {
    home.join(".codex").join("config.toml")
}
fn read(path: &Path) -> Option<toml::Value> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}
fn source(view: &mut ConfigView, path: &Path) {
    view.sources.push(ConfigSource {
        path: user_visible(path),
        scope: "user",
        exists: path.is_file(),
    });
}
fn object<'a>(
    value: &'a toml::Value,
    key: &str,
) -> Option<&'a toml::map::Map<String, toml::Value>> {
    value.get(key)?.as_table()
}
fn json(value: &toml::Value) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}
fn secrets(rows: &mut Vec<ConfigRow>, value: &toml::Value, key: &str, label: &str) {
    if let Some(map) = value
        .get(key)
        .and_then(toml::Value::as_table)
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
    } else if let Some(values) = value
        .get(key)
        .and_then(toml::Value::as_array)
        .filter(|v| !v.is_empty())
    {
        rows.push(
            ConfigRow::fact(label, format!("{} variable(s)", values.len()))
                .with_hint("values hidden"),
        );
    }
}

fn mcp(home: &Path) -> ConfigView {
    let p = path(home);
    let mut view = ConfigView::new("codex", "view_mcp", "MCP servers");
    source(&mut view, &p);
    let Some(root) = read(&p) else {
        view.push_issue(
            ConfigTone::Muted,
            "No ~/.codex/config.toml found; no MCP servers are configured.",
        );
        return view;
    };
    if let Some(servers) = object(&root, "mcp_servers") {
        for (name, cfg) in servers {
            let mut rows = vec![];
            if let Some(scope) = cfg.get("scope") {
                rows.push(ConfigRow::fact("Scope", json(scope)));
            }
            let transport = cfg
                .get("transport")
                .and_then(toml::Value::as_str)
                .unwrap_or(if cfg.get("command").is_some() {
                    "stdio"
                } else {
                    "remote"
                });
            rows.push(ConfigRow::fact("Transport", transport));
            for key in ["command", "args", "enabled"] {
                if let Some(v) = cfg.get(key) {
                    rows.push(ConfigRow::fact(key, json(v)));
                }
            }
            secrets(&mut rows, cfg, "env", "Environment");
            secrets(&mut rows, cfg, "env_vars", "Environment variables");
            secrets(&mut rows, cfg, "http_headers", "HTTP headers");
            view.push_entry_section(name, rows, entry_metadata("codex", "view_mcp", &format!("global:{name}"), &json(cfg)));
        }
    }
    if view.sections.is_empty() {
        view.push_issue(ConfigTone::Muted, "No MCP servers are configured.");
    }
    view
}

pub(crate) fn remove_mcp(entry_id: &str, expected: &str) -> anyhow::Result<super::RemovalReport> {
    let home = crate::config::effective_home_dir()?;
    let path = path(&home);
    let Some(text) = std::fs::read_to_string(&path).ok() else { return Ok(super::RemovalReport::already_absent("codex", "view_mcp", entry_id)); };
    let mut root: toml::Value = text.parse().map_err(|e| anyhow::anyhow!("Invalid Codex config: {e}"))?;
    let Some(servers) = root.get("mcp_servers").and_then(toml::Value::as_table) else { return Ok(super::RemovalReport::already_absent("codex", "view_mcp", entry_id)); };
    let found = servers.iter().find(|(name, cfg)| entry_metadata("codex", "view_mcp", &format!("global:{name}"), &json(cfg)).entry_id == entry_id).map(|(name, cfg)| (name.clone(), json(cfg)));
    let Some((name, cfg)) = found else { return Ok(super::RemovalReport::already_absent("codex", "view_mcp", entry_id)); };
    if entry_metadata("codex", "view_mcp", &format!("global:{name}"), &cfg).fingerprint != expected { return Err(super::RemovalError::Conflict.into()); }
    root.get_mut("mcp_servers").and_then(toml::Value::as_table_mut).map(|m| m.remove(&name));
    let backup = super::backup_config(&path)?;
    crate::storage::atomic_write::write_string_atomic(&path, &toml::to_string_pretty(&root)?)?;
    Ok(super::RemovalReport::removed("codex", "view_mcp", entry_id, backup))
}

fn plugins(home: &Path) -> ConfigView {
    let p = path(home);
    let mut view = ConfigView::new("codex", "view_plugins", "Plugins");
    source(&mut view, &p);
    let Some(root) = read(&p) else {
        view.push_issue(
            ConfigTone::Muted,
            "No ~/.codex/config.toml found; no plugins are configured.",
        );
        return view;
    };
    if let Some(plugins) = object(&root, "plugins") {
        for (name, cfg) in plugins {
            let mut rows = vec![];
            if let Some(v) = cfg.get("enabled") {
                rows.push(ConfigRow::fact("Enabled", json(v)));
            }
            if let Some(v) = cfg.get("mcp_servers") {
                rows.push(ConfigRow::fact("MCP policy", json(v)));
            }
            view.push_section(name, rows);
        }
    }
    view.push_issue(
        ConfigTone::Muted,
        "Plugin configuration does not mean the plugin is installed.",
    );
    if view.sections.is_empty() {
        view.push_issue(ConfigTone::Muted, "No plugins are configured.");
    }
    view
}

fn statusline(home: &Path) -> ConfigView {
    let p = path(home);
    let mut view = ConfigView::new("codex", "view_statusline", "Status line");
    source(&mut view, &p);
    let Some(root) = read(&p) else {
        view.push_issue(ConfigTone::Muted, "No ~/.codex/config.toml found.");
        return view;
    };
    let Some(tui) = object(&root, "tui") else {
        view.push_issue(
            ConfigTone::Muted,
            "No TUI status configuration is configured.",
        );
        return view;
    };
    let mut rows = vec![];
    for key in ["status_line", "terminal_title"] {
        if let Some(v) = tui.get(key) {
            rows.push(ConfigRow::fact(key, json(v)));
        }
    }
    if rows.is_empty() {
        view.push_issue(
            ConfigTone::Muted,
            "No status line or terminal title is configured.",
        );
    } else {
        view.push_section("tui", rows);
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toml_views_keep_arrays_and_hide_secrets() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".codex")).unwrap();
        std::fs::write(path(d.path()), "[mcp_servers.demo]\nscope='user'\ntransport='stdio'\ncommand='run'\nargs=['--x']\nenabled=true\nenv={TOKEN='secret'}\n[plugins.foo]\nenabled=false\nmcp='allow'\n[tui]\nstatus_line=['model','cwd']\nterminal_title=['codex']\n").unwrap();
        let m = mcp(d.path());
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("--x") && s.contains("TOKEN") && !s.contains("secret"));
        let t = statusline(d.path());
        assert_eq!(
            t.sections[0].rows[0].value,
            serde_json::json!(["model", "cwd"])
        );
        assert_eq!(plugins(d.path()).sections.len(), 1);
    }
}
