use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde_json::{Map, Value};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};
use crate::storage::atomic_write;

const OPENCODE_PLUGIN_FILE: &str = "memorph.js";
const OPENCODE_PLUGIN_MARKER: &str = "memorph-opencode-hook-plugin";
const OPENCODE_PLUGIN_VERSION: &str = "v1";

pub struct OpenCodeHook;

pub static OPENCODE_HOOK: OpenCodeHook = OpenCodeHook;

impl ProviderHook for OpenCodeHook {
    fn provider_id(&self) -> &'static str {
        "opencode"
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

fn opencode_config_dir() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".config")
        .join("opencode")
}

fn opencode_plugin_dir() -> PathBuf {
    opencode_config_dir().join("plugins")
}
fn opencode_plugin_path() -> PathBuf {
    opencode_plugin_dir().join(OPENCODE_PLUGIN_FILE)
}

fn opencode_config_candidates() -> Vec<PathBuf> {
    let dir = opencode_config_dir();
    vec![
        dir.join("opencode.jsonc"),
        dir.join("opencode.json"),
        dir.join("config.json"),
    ]
}

fn status() -> Result<HookInstallStatus> {
    let config_dir = opencode_config_dir();
    let plugin_path = opencode_plugin_path();
    if !config_dir.exists() {
        return Ok(HookInstallStatus {
            provider: "opencode".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path: Some(config_dir.display().to_string()),
            installed_version: None,
            current_version: Some(OPENCODE_PLUGIN_VERSION.to_string()),
            message: Some("OpenCode config directory does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("opencode"),
        });
    }

    let installed = opencode_plugin_installed();
    let installed_version = opencode_installed_plugin_version();
    let current_version = Some(OPENCODE_PLUGIN_VERSION.to_string());
    let stale = installed_version
        .as_deref()
        .map(|version| version != OPENCODE_PLUGIN_VERSION)
        .unwrap_or(false);
    Ok(HookInstallStatus {
        provider: "opencode".to_string(),
        status: if installed && !stale {
            HookHealthStatus::InstalledOk
        } else if stale {
            HookHealthStatus::InstalledStaleBinary
        } else {
            HookHealthStatus::Repairable
        },
        config_path: Some(plugin_path.display().to_string()),
        installed_version,
        current_version,
        message: Some(if installed {
            "OpenCode memorph plugin is installed.".to_string()
        } else if stale {
            "OpenCode memorph plugin is stale.".to_string()
        } else {
            "OpenCode memorph plugin is missing, stale, or not registered.".to_string()
        }),
        last_event_at: crate::hooks::health::last_event_at("opencode"),
    })
}

fn install() -> Result<HookOperationReport> {
    let config_dir = opencode_config_dir();
    if !config_dir.exists() {
        let status = status()?;
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
        crate::hooks::shared::backup_if_exists(&target_path)?
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

    let status = status()?;
    Ok(HookOperationReport {
        provider: "opencode".to_string(),
        operation: "install".to_string(),
        changed: plugin_changed || config_changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("OpenCode memorph plugin installed.".to_string()),
    })
}

fn uninstall() -> Result<HookOperationReport> {
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
            backup_path = crate::hooks::shared::backup_if_exists(&config_path)?;
        } else {
            let _ = crate::hooks::shared::backup_if_exists(&config_path);
        }
        atomic_write::write_string_atomic(&config_path, &cleaned)?;
        changed = true;
    }

    let status = status()?;
    Ok(HookOperationReport {
        provider: "opencode".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("OpenCode memorph plugin removed.".to_string()),
    })
}

fn opencode_plugin_installed() -> bool {
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

fn opencode_installed_plugin_version() -> Option<String> {
    let contents = fs::read_to_string(opencode_plugin_path()).ok()?;
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

fn opencode_config_contains_memorph_plugin(contents: &str) -> bool {
    crate::hooks::config_formats::jsonc::parse_object(contents)
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

fn opencode_registration_target() -> PathBuf {
    let candidates = opencode_config_candidates();
    if candidates[0].exists() {
        candidates[0].clone()
    } else {
        candidates[1].clone()
    }
}

fn bridge_executable_path() -> Result<String> {
    Ok(std::env::current_exe()
        .context("Failed to resolve current memorph executable")?
        .to_string_lossy()
        .to_string())
}

fn opencode_plugin_source() -> Result<String> {
    let exe = serde_json::to_string(&bridge_executable_path()?)?;
    let template = r#"// __OPENCODE_PLUGIN_MARKER__
// version: __OPENCODE_PLUGIN_VERSION__
// Auto-generated by memorph. Forwards OpenCode runtime events into memorph hooks.
import {{ execFile }} from "child_process";

const MEMORPH = __MEMORPH_EXE_JSON__;
function send(mapped) {{
  return new Promise((resolve) => {{
    const args = ["__hook-bridge", "--provider", "opencode", "--event", mapped.hook_event_name || "Event"];
    const child = execFile(MEMORPH, args, {{ timeout: 5000, maxBuffer: 1024 * 1024 }}, (error, stdout) => {{
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
        await send(mapped);
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
        return Ok(serde_json::to_string_pretty(&Value::Object(root))?
            + "
");
    };

    let parsed = crate::hooks::config_formats::jsonc::parse_object(contents)?;
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

    let mut merged = crate::hooks::config_formats::jsonc::set_top_level_value(
        contents,
        "plugin",
        &Value::Array(plugins),
    )?;
    if !parsed.contains_key("$schema") {
        merged = crate::hooks::config_formats::jsonc::set_top_level_value(
            &merged,
            "$schema",
            &Value::String("https://opencode.ai/config.json".to_string()),
        )?;
    }
    Ok(crate::hooks::config_formats::jsonc::ensure_trailing_newline(merged))
}

fn remove_opencode_plugin_ref(original: &str) -> Result<Option<String>> {
    let mut root = crate::hooks::config_formats::jsonc::parse_object(original)?;
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
        return Ok(Some(
            crate::hooks::config_formats::jsonc::ensure_trailing_newline(
                crate::hooks::config_formats::jsonc::delete_top_level_key(original, "plugin")?,
            ),
        ));
    }
    Ok(Some(
        crate::hooks::config_formats::jsonc::ensure_trailing_newline(
            crate::hooks::config_formats::jsonc::set_top_level_value(
                original,
                "plugin",
                &Value::Array(plugins),
            )?,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::test_support::TestHookHomeGuard;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = OPENCODE_HOOK.descriptor().expect("opencode descriptor");
        assert_eq!(descriptor.provider(), OPENCODE_HOOK.provider_id());
    }
    #[test]
    fn opencode_install_uninstall_preserves_jsonc_comments_and_foreign_plugins() {
        let _home = TestHookHomeGuard::new();
        let opencode_config_dir = crate::hooks::shared::hook_home_dir()
            .join(".config")
            .join("opencode");
        std::fs::create_dir_all(&opencode_config_dir).unwrap();
        let config_path = opencode_config_dir.join("opencode.jsonc");
        crate::storage::atomic_write::write_string_atomic(
            &config_path,
            "{\n  // keep this comment\n  \"plugin\": [\n    \"file:///tmp/keep.js\"\n  ],\n  \"theme\": \"dark\"\n}\n",
        )
        .unwrap();

        OPENCODE_HOOK.install().unwrap();
        let installed = std::fs::read_to_string(&config_path).unwrap();
        assert!(installed.contains("// keep this comment"));
        assert!(installed.contains("file:///tmp/keep.js"));
        assert!(installed.contains("memorph.js"));
        assert!(installed.contains("\"theme\": \"dark\""));

        OPENCODE_HOOK.uninstall().unwrap();
        let removed = std::fs::read_to_string(&config_path).unwrap();
        assert!(removed.contains("// keep this comment"));
        assert!(removed.contains("file:///tmp/keep.js"));
        assert!(!removed.contains("memorph.js"));
        assert!(removed.contains("\"theme\": \"dark\""));
    }

    #[test]
    fn opencode_config_reader_detects_array_and_string_plugin_refs() {
        assert!(opencode_config_contains_memorph_plugin(
            r#"{ "plugin": ["file:///Users/me/.config/opencode/plugins/memorph.js"] }"#
        ));
        assert!(opencode_config_contains_memorph_plugin(
            r#"{ // comment
              "plugin": "file:///Users/me/.config/opencode/plugins/memorph.js"
            }"#
        ));
    }
}
