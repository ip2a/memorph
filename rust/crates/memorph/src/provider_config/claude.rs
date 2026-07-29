//! Claude Code configuration inspectors.
//!
//! Reads the user's `~/.claude` tree (resolved through [`crate::config::effective_home_dir`])
//! and projects it into structured, redacted [`ConfigView`]s. Sources follow the
//! layout documented in the project's Claude config analysis: MCP servers live in
//! `~/.claude.json`, plugins in `~/.claude/plugins/installed_plugins.json` crossed
//! with `enabledPlugins` in `~/.claude/settings.json`, and the status line in the
//! same settings file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::provider_config::{user_visible, ConfigRow, ConfigSource, ConfigTone, ConfigView};
use crate::provider_settings::{SettingDefinition, SettingKind, SettingScope};

/// The `View`-kind settings the agent page advertises for Claude. This slice is the
/// single source of truth — [`crate::provider_settings::claude`] hands it out
/// unchanged, so the declared views and the inspectors below cannot drift apart.
pub(crate) const VIEW_SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "view_mcp",
        title: "MCP servers",
        description: "Model Context Protocol servers configured for Claude Code (stdio and HTTP).",
        scope: SettingScope::Global,
        kind: SettingKind::View,
    },
    SettingDefinition {
        id: "view_plugins",
        title: "Plugins",
        description: "Installed Claude Code plugins and their enable state.",
        scope: SettingScope::Global,
        kind: SettingKind::View,
    },
    SettingDefinition {
        id: "view_statusline",
        title: "Status line",
        description: "The Claude Code status line renderer and its options.",
        scope: SettingScope::Global,
        kind: SettingKind::View,
    },
];

/// Inspect one declared Claude view. Resolves the home directory here so the
/// individual inspectors stay pure functions of a home path (and testable with a
/// temporary directory, no global state).
pub fn inspect(view_id: &str) -> anyhow::Result<ConfigView> {
    let home = crate::config::effective_home_dir()?;
    match view_id {
        "view_mcp" => Ok(mcp(&home)),
        "view_plugins" => Ok(plugins(&home)),
        "view_statusline" => Ok(statusline(&home)),
        other => anyhow::bail!("Unknown Claude config view: {other}"),
    }
}

fn claude_json(home: &Path) -> PathBuf {
    home.join(".claude.json")
}

fn settings_json(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

fn installed_plugins_json(home: &Path) -> PathBuf {
    home.join(".claude")
        .join("plugins")
        .join("installed_plugins.json")
}

fn record_source(path: &Path, scope: &'static str, view: &mut ConfigView) {
    view.sources.push(ConfigSource {
        path: user_visible(path),
        scope,
        exists: path.is_file(),
    });
}

// --- MCP servers -------------------------------------------------------------

fn mcp(home: &Path) -> ConfigView {
    let mut view = ConfigView::new("claude", "view_mcp", "MCP servers");
    let path = claude_json(home);
    record_source(&path, "user", &mut view);

    let Some(root) = crate::provider_config::read_json(&path) else {
        view.push_issue(
            ConfigTone::Muted,
            "No ~/.claude.json found; no MCP servers are configured.",
        );
        return view;
    };

    let mut count = 0usize;
    if let Some(servers) = root.get("mcpServers").and_then(Value::as_object) {
        for (name, cfg) in servers {
            push_server_section(&mut view, name, cfg, "User");
            count += 1;
        }
    }
    if let Some(projects) = root.get("projects").and_then(Value::as_object) {
        for (project_dir, project) in projects {
            let Some(servers) = project.get("mcpServers").and_then(Value::as_object) else {
                continue;
            };
            let scope = format!("Project · {}", user_visible(Path::new(project_dir)));
            for (name, cfg) in servers {
                push_server_section(&mut view, name, cfg, &scope);
                count += 1;
            }
        }
    }

    if count == 0 {
        view.push_issue(ConfigTone::Muted, "No MCP servers are configured.");
    }
    view.push_issue(
        ConfigTone::Muted,
        "Run `claude mcp list` for live connectivity health — this view shows configuration only.",
    );
    view
}

fn push_server_section(view: &mut ConfigView, name: &str, cfg: &Value, scope: &str) {
    let transport = cfg.get("type").and_then(Value::as_str).unwrap_or("");
    let mut rows = vec![ConfigRow::fact("Scope", scope)];

    match (cfg.get("command").and_then(Value::as_str), cfg.get("url").and_then(Value::as_str)) {
        (Some(command), _) => {
            rows.push(ConfigRow::fact("Type", "stdio"));
            rows.push(ConfigRow::fact("Command", command));
        }
        (None, Some(url)) => {
            rows.push(ConfigRow::fact("Type", "http"));
            rows.push(ConfigRow::fact("URL", url));
        }
        _ => rows.push(ConfigRow::fact("Type", transport).with_tone(ConfigTone::Muted)),
    }

    if let Some(args) = cfg.get("args").and_then(Value::as_array) {
        let joined = args
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        if !joined.is_empty() {
            rows.push(ConfigRow::fact("Args", joined));
        }
    }
    if let Some(env) = cfg.get("env").and_then(Value::as_object).filter(|env| !env.is_empty()) {
        let keys = env.keys().cloned().collect::<Vec<_>>().join(", ");
        rows.push(ConfigRow::fact("Environment", keys).with_hint("values hidden"));
    }
    if let Some(headers) = cfg.get("headers") {
        // Labelled "Headers" so the redaction pass masks the whole object.
        rows.push(ConfigRow::fact("Headers", headers.clone()));
    }

    view.push_section(name, rows);
}

// --- Plugins -----------------------------------------------------------------

fn plugins(home: &Path) -> ConfigView {
    let mut view = ConfigView::new("claude", "view_plugins", "Plugins");
    let installed_path = installed_plugins_json(home);
    let settings_path = settings_json(home);
    record_source(&installed_path, "user", &mut view);
    record_source(&settings_path, "user", &mut view);

    let installed = crate::provider_config::read_json(&installed_path);
    let settings = crate::provider_config::read_json(&settings_path);

    let installed_plugins = installed
        .as_ref()
        .and_then(|value| value.get("plugins").and_then(Value::as_object));
    let enabled = settings
        .as_ref()
        .and_then(|value| value.get("enabledPlugins").and_then(Value::as_object));

    let mut installed_ids: Vec<&str> = installed_plugins
        .map(|plugins| plugins.keys().map(String::as_str).collect())
        .unwrap_or_default();
    installed_ids.sort();

    for id in &installed_ids {
        let plugins = installed_plugins.expect("present when ids are non-empty");
        let version = plugins
            .get(*id)
            .and_then(Value::as_array)
            .and_then(|records| records.first())
            .and_then(|record| record.get("version"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        let marketplace = id.split_once('@').map(|(_, market)| market).unwrap_or("-");
        let enabled_state = enabled.and_then(|map| map.get(*id)).and_then(Value::as_bool);

        let mut rows = vec![
            ConfigRow::fact("Marketplace", marketplace),
            ConfigRow::fact("Version", version),
        ];
        rows.push(match enabled_state {
            Some(true) => ConfigRow::fact("Enabled", "yes").with_tone(ConfigTone::Ok),
            Some(false) => ConfigRow::fact("Enabled", "no").with_tone(ConfigTone::Muted),
            None => ConfigRow::fact("Enabled", "not listed").with_tone(ConfigTone::Warning),
        });
        view.push_section(*id, rows);
    }

    if let Some(enabled) = enabled {
        let installed_set: BTreeSet<&str> = installed_ids.iter().copied().collect();
        let mut dangling: Vec<&str> = enabled
            .keys()
            .map(String::as_str)
            .filter(|id| !installed_set.contains(id))
            .collect();
        dangling.sort();
        if !dangling.is_empty() {
            view.push_issue(
                ConfigTone::Warning,
                format!(
                    "{} enabled plugin(s) are not installed: {}",
                    dangling.len(),
                    dangling.join(", ")
                ),
            );
        }
    }

    if installed_ids.is_empty() {
        view.push_issue(ConfigTone::Muted, "No plugins are installed.");
    }
    view
}

// --- Status line -------------------------------------------------------------

fn statusline(home: &Path) -> ConfigView {
    let mut view = ConfigView::new("claude", "view_statusline", "Status line");
    let path = settings_json(home);
    record_source(&path, "user", &mut view);

    let Some(settings) = crate::provider_config::read_json(&path) else {
        view.push_issue(ConfigTone::Muted, "No ~/.claude/settings.json found.");
        return view;
    };

    let Some(status_line) = settings.get("statusLine") else {
        view.push_issue(ConfigTone::Muted, "No status line is configured.");
        return view;
    };

    let kind = status_line
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let mut rows = vec![ConfigRow::fact("Type", kind)];
    if let Some(command) = status_line.get("command").and_then(Value::as_str) {
        rows.push(ConfigRow::fact("Command", command));
    }
    if let Some(padding) = status_line.get("padding") {
        rows.push(ConfigRow::fact("Padding", padding.clone()));
    }
    view.push_section("statusLine", rows);
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_config::redaction;
    use serde_json::json;

    fn write_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude").join("plugins")).unwrap();
        dir
    }

    #[test]
    fn mcp_view_redacts_headers_and_env_values() {
        let dir = write_home();
        std::fs::write(
            claude_json(dir.path()),
            json!({
                "mcpServers": {
                    "http-svc": {
                        "type": "http",
                        "url": "https://example.test/mcp",
                        "headers": { "Authorization": "Bearer super-secret" }
                    },
                    "stdio-svc": {
                        "type": "stdio",
                        "command": "uvx",
                        "args": ["DrissionPage-MCP"],
                        "env": { "API_TOKEN": "hush", "DEBUG": "1" }
                    }
                },
                "projects": {
                    "/tmp/proj": {
                        "mcpServers": { "proj-svc": { "type": "stdio", "command": "run" } }
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let mut view = mcp(dir.path());
        redaction::redact(&mut view);
        let serialized = serde_json::to_string(&view).unwrap();

        assert!(!serialized.contains("super-secret"));
        assert!(!serialized.contains("hush"));
        assert!(serialized.contains("DrissionPage-MCP"));
        assert!(serialized.contains("https://example.test/mcp"));
        assert!(serialized.contains("values hidden"));
        // env keys are shown, values are not
        assert!(serialized.contains("API_TOKEN"));
        assert!(serialized.contains("••••••"));
        // all three scopes surface as sections
        assert_eq!(view.sections.len(), 3);
    }

    #[test]
    fn plugins_view_flags_dangling_enabled_switches() {
        let dir = write_home();
        std::fs::write(
            installed_plugins_json(dir.path()),
            json!({
                "version": 2,
                "plugins": {
                    "ponytail@ponytail": [{ "version": "4.8.4" }],
                    "warp@claude-code-warp": [{ "version": "2.0.0" }]
                }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            settings_json(dir.path()),
            json!({
                "enabledPlugins": {
                    "ponytail@ponytail": true,
                    "ghost@missing-market": true
                }
            })
            .to_string(),
        )
        .unwrap();

        let view = plugins(dir.path());

        assert_eq!(view.sections.len(), 2);
        let ponytail = view
            .sections
            .iter()
            .find(|section| section.label == "ponytail@ponytail")
            .unwrap();
        assert!(ponytail.rows.iter().any(|row| row.label == "Enabled" && row.value == "yes"));
        assert!(view
            .issues
            .iter()
            .any(|issue| issue.message.contains("ghost@missing-market")));
    }

    #[test]
    fn statusline_view_reports_missing_config() {
        let dir = write_home();
        let view = statusline(dir.path());
        assert!(view.sections.is_empty());
        assert!(view.issues.iter().any(|issue| issue.tone == ConfigTone::Muted));
    }

    #[test]
    fn view_settings_match_inspectable_ids() {
        let declared: Vec<&str> = VIEW_SETTINGS.iter().map(|setting| setting.id).collect();
        assert_eq!(declared, vec!["view_mcp", "view_plugins", "view_statusline"]);
        for setting in VIEW_SETTINGS {
            assert_eq!(setting.kind, SettingKind::View);
        }
    }
}
