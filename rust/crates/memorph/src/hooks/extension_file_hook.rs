//! Shared operations for provider hooks installed as generated extension files.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};
use crate::storage::atomic_write;

#[derive(Clone, Copy)]
pub struct ExtensionFileHookSpec {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub extension_dir: fn() -> PathBuf,
    pub extension_path: fn() -> PathBuf,
    pub marker: &'static str,
    pub source: fn() -> Result<String>,
    pub missing_status_message: &'static str,
    pub install_message: &'static str,
    pub uninstall_missing_message: &'static str,
    pub unmanaged_uninstall_message: &'static str,
    pub uninstall_message: &'static str,
}

pub fn status(spec: ExtensionFileHookSpec) -> Result<HookInstallStatus> {
    let path = (spec.extension_path)();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: spec.provider.to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some(spec.missing_status_message.to_string()),
            last_event_at: crate::hooks::health::last_event_at(spec.provider),
        });
    }
    let contents = fs::read_to_string(&path)?;
    let installed = installed_version(&contents, spec.marker);
    let installed_version = installed.clone().flatten();
    let health = match installed {
        None => HookHealthStatus::NotInstalled,
        Some(version)
            if version.as_deref() == Some(crate::hooks::shared::current_hook_managed_version()) =>
        {
            HookHealthStatus::InstalledOk
        }
        Some(_) => HookHealthStatus::InstalledStaleBinary,
    };
    let message = match health {
        HookHealthStatus::InstalledOk => Some(format!(
            "{} memorph extension is installed.",
            spec.display_name
        )),
        HookHealthStatus::InstalledStaleBinary => Some(format!(
            "{} memorph extension is installed but stale: installed {}, current {}.",
            spec.display_name,
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        )),
        _ => Some(format!(
            "{} extension file is not managed by memorph.",
            spec.display_name
        )),
    };
    Ok(HookInstallStatus {
        provider: spec.provider.to_string(),
        status: health,
        config_path,
        installed_version,
        current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
        message,
        last_event_at: crate::hooks::health::last_event_at(spec.provider),
    })
}

pub fn install(spec: ExtensionFileHookSpec) -> Result<HookOperationReport> {
    let path = (spec.extension_path)();
    fs::create_dir_all((spec.extension_dir)()).with_context(|| {
        format!(
            "Failed to create {} extension directory: {}",
            spec.display_name,
            (spec.extension_dir)().display()
        )
    })?;
    let original = fs::read_to_string(&path).ok();
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let source = (spec.source)()?;
    let changed = original.as_deref() != Some(source.as_str());
    if changed {
        atomic_write::write_string_atomic(&path, &source)?;
    }
    let status = status(spec)?;
    Ok(HookOperationReport {
        provider: spec.provider.to_string(),
        operation: "install".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some(spec.install_message.to_string()),
    })
}

pub fn uninstall(spec: ExtensionFileHookSpec) -> Result<HookOperationReport> {
    let path = (spec.extension_path)();
    if !path.exists() {
        let status = status(spec)?;
        return Ok(HookOperationReport {
            provider: spec.provider.to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some(spec.uninstall_missing_message.to_string()),
        });
    }
    let contents = fs::read_to_string(&path).unwrap_or_default();
    if !contents.contains(spec.marker) {
        let status = status(spec)?;
        return Ok(HookOperationReport {
            provider: spec.provider.to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some(spec.unmanaged_uninstall_message.to_string()),
        });
    }
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    fs::remove_file(&path).with_context(|| {
        format!(
            "Failed to remove {} extension file: {}",
            spec.display_name,
            path.display()
        )
    })?;
    let status = status(spec)?;
    Ok(HookOperationReport {
        provider: spec.provider.to_string(),
        operation: "uninstall".to_string(),
        changed: true,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some(spec.uninstall_message.to_string()),
    })
}

pub fn installed_version(contents: &str, marker: &str) -> Option<Option<String>> {
    if !contents.contains(marker) || !contents.contains(crate::hooks::shared::HOOK_COMMAND_MARKER) {
        return None;
    }
    let version = contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("// version:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    });
    Some(version)
}

pub fn pi_agent_extension_source(
    provider: &str,
    marker: &str,
    extension_api_import: &str,
    session_prefix: &str,
) -> Result<String> {
    memorph_pi_extension_source(
        provider,
        marker,
        crate::hooks::shared::current_hook_managed_version(),
        extension_api_import,
        session_prefix,
    )
}

fn bridge_executable_path() -> Result<String> {
    Ok(std::env::current_exe()
        .context("Failed to resolve current memorph executable")?
        .to_string_lossy()
        .to_string())
}

fn memorph_pi_extension_source(
    provider: &str,
    marker: &str,
    version: &str,
    extension_api_import: &str,
    session_prefix: &str,
) -> Result<String> {
    let exe = serde_json::to_string(&bridge_executable_path()?)?;
    let provider_json = serde_json::to_string(provider)?;
    let marker_json = serde_json::to_string(marker)?;
    let version_json = serde_json::to_string(version)?;
    let session_prefix_json = serde_json::to_string(session_prefix)?;
    Ok(format!(
        r#"// {marker}
// version: {version}
// Generated by memorph. Relays pi/OMP agent lifecycle events to memorph hooks.

import {{ execFile, execFileSync }} from "node:child_process";
{extension_api_import}

const MEMORPH_EXE = {exe};
const PROVIDER = {provider_json};
const SESSION_PREFIX = {session_prefix_json};
const MARKER = {marker_json};
const VERSION = {version_json};
const MANAGED_VERSION = "{managed_version}";

const ENV_KEYS = [
  "TERM_PROGRAM",
  "ITERM_SESSION_ID",
  "TERM_SESSION_ID",
  "TMUX",
  "TMUX_PANE",
  "KITTY_WINDOW_ID",
  "__CFBundleIdentifier",
] as const;

const DANGEROUS_PATTERNS: RegExp[] = [
  /\brm\s+(-rf?|--recursive)/i,
  /\bsudo\b/i,
  /\b(chmod|chown)\b.*777/i,
];

function detectTty(): string | null {{
  try {{
    let pid = process.pid;
    for (let i = 0; i < 8; i++) {{
      const out = execFileSync("ps", ["-o", "tty=,ppid=", "-p", String(pid)], {{
        timeout: 1000,
      }})
        .toString()
        .trim();
      const [tty, ppidStr] = out.split(/\s+/);
      if (tty && tty !== "??" && tty !== "?") {{
        return tty.startsWith("/dev/") ? tty : `/dev/${{tty}}`;
      }}
      const ppid = parseInt(ppidStr ?? "0", 10);
      if (!ppid || ppid <= 1) break;
      pid = ppid;
    }}
  }} catch {{}}
  return null;
}}

function sendToMemorph(
  eventName: string,
  payload: Record<string, unknown>,
  timeoutMs = 5_000,
): Promise<Record<string, unknown> | null> {{
  return new Promise((resolve) => {{
    const args = [
      "__hook-bridge",
      "--managed-version",
      MANAGED_VERSION,
      "--provider",
      PROVIDER,
      "--event",
      eventName,
    ];
    try {{
      const child = execFile(
        MEMORPH_EXE,
        args,
        {{ timeout: timeoutMs, maxBuffer: 1_048_576 }},
        (error, stdout) => {{
          if (error || !stdout.trim()) {{
            resolve(null);
            return;
          }}
          try {{
            resolve(JSON.parse(stdout));
          }} catch {{
            resolve(null);
          }}
        }},
      );
      child.stdin?.write(JSON.stringify(payload));
      child.stdin?.end();
    }} catch {{
      resolve(null);
    }}
  }});
}}

function base(
  sessionId: string,
  cwd: string,
  extra: Record<string, unknown>,
  tty: string | null,
): Record<string, unknown> {{
  return {{
    session_id: `${{SESSION_PREFIX}}-${{sessionId}}`,
    provider: PROVIDER,
    _source: PROVIDER,
    _ppid: process.pid,
    _env: collectEnv(),
    _tty: tty,
    cwd,
    ...extra,
  }};
}}

function displayToolName(name: string): string {{
  return name.charAt(0).toUpperCase() + name.slice(1);
}}

function extractLastAssistantText(messages: readonly unknown[]): string {{
  const assistants = messages.filter(
    (message): message is {{ role: "assistant"; content: unknown }} =>
      !!message &&
      typeof message === "object" &&
      (message as {{ role?: string }}).role === "assistant",
  );
  const last = assistants.at(-1);
  if (!last || !Array.isArray(last.content)) return "";
  return last.content
    .filter((part): part is {{ type: "text"; text: string }} =>
      !!part &&
      typeof part === "object" &&
      (part as {{ type?: string }}).type === "text" &&
      typeof (part as {{ text?: unknown }}).text === "string",
    )
    .map((part) => part.text)
    .join("")
    .trim();
}}

export default function memorphExtension(pi: ExtensionAPI) {{
  void MARKER;
  void VERSION;
  const tty = detectTty();

  pi.on("session_start", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    const sessionName = typeof pi.getSessionName === "function" ? pi.getSessionName() : undefined;
    await sendToMemorph(
      "SessionStart",
      base(sessionId, ctx.cwd, {{
        hook_event_name: "SessionStart",
        ...(sessionName ? {{ session_title: sessionName }} : {{}}),
      }}, tty),
    );
  }});

  pi.on("session_shutdown", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    await sendToMemorph("SessionEnd", base(sessionId, ctx.cwd, {{ hook_event_name: "SessionEnd" }}, tty));
  }});

  pi.on("before_agent_start", async (event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    await sendToMemorph(
      "UserPromptSubmit",
      base(sessionId, ctx.cwd, {{
        hook_event_name: "UserPromptSubmit",
        prompt: event.prompt ?? "",
      }}, tty),
    );
  }});

  pi.on("agent_end", async (event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    const sessionName = typeof pi.getSessionName === "function" ? pi.getSessionName() : undefined;
    await sendToMemorph(
      "Stop",
      base(sessionId, ctx.cwd, {{
        hook_event_name: "Stop",
        last_assistant_message: extractLastAssistantText(event.messages) || undefined,
        ...(sessionName ? {{ session_title: sessionName }} : {{}}),
      }}, tty),
    );
  }});

  pi.on("tool_call", async (event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    const toolName = displayToolName(event.toolName);
    const toolInput: Record<string, unknown> = {{ ...event.input }};
    if (event.toolName === "bash") {{
      const command = event.input.command as string | undefined;
      if (command) toolInput.patterns = [command];
    }}
    if (event.toolName === "edit" || event.toolName === "write") {{
      const path = event.input.path as string | undefined;
      if (path) toolInput.file_path = path;
    }}



    await sendToMemorph(
      "PreToolUse",
      base(sessionId, ctx.cwd, {{
        hook_event_name: "PreToolUse",
        tool_name: toolName,
        tool_input: toolInput,
      }}, tty),
    );
    return undefined;
  }});

  pi.on("tool_result", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    await sendToMemorph("PostToolUse", base(sessionId, ctx.cwd, {{ hook_event_name: "PostToolUse" }}, tty));
  }});

  pi.on("session_before_compact", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    await sendToMemorph("PreCompact", base(sessionId, ctx.cwd, {{ hook_event_name: "PreCompact" }}, tty));
  }});

  pi.on("session_compact", async (_event, ctx) => {{
    const sessionId = ctx.sessionManager.getSessionId();
    await sendToMemorph("PostCompact", base(sessionId, ctx.cwd, {{ hook_event_name: "PostCompact" }}, tty));
  }});
}}
"#,
        marker = marker,
        version = version,
        extension_api_import = extension_api_import,
        exe = exe,
        provider_json = provider_json,
        session_prefix_json = session_prefix_json,
        marker_json = marker_json,
        version_json = version_json,
        managed_version = crate::hooks::shared::HOOK_MANAGED_VERSION,
    ))
}
