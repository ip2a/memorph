use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

const VERSION_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentEnvironmentStatus {
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_dir: Option<String>,
    pub config_path: String,
    pub install_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_version: Option<String>,
}

pub fn detect_provider_environment(provider_id: &str) -> AgentEnvironmentStatus {
    detect_provider_environment_cached(provider_id, false)
}

pub fn detect_provider_environment_fast(provider_id: &str) -> AgentEnvironmentStatus {
    detect_provider_environment_fast_cached(provider_id)
}

pub fn refresh_provider_environment(provider_id: &str) -> AgentEnvironmentStatus {
    detect_provider_environment_cached(provider_id, true)
}

fn detect_provider_environment_fast_cached(provider_id: &str) -> AgentEnvironmentStatus {
    let cache = crate::cache::agent_environment_cache();
    if let Some(status) = cache.get_fast(provider_id) {
        return status;
    }

    let status = detect_provider_environment_uncached(provider_id, false);
    cache.set_fast(provider_id, status.clone());
    status
}

fn detect_provider_environment_cached(provider_id: &str, refresh: bool) -> AgentEnvironmentStatus {
    if !refresh {
        let cache = crate::cache::agent_environment_cache();
        if let Some(status) = cache.get_full(provider_id) {
            return status;
        }
    }

    let status = detect_provider_environment_uncached(provider_id, true);
    crate::cache::agent_environment_cache().set_full(provider_id, status.clone());
    status
}

fn detect_provider_environment_uncached(
    provider_id: &str,
    include_version: bool,
) -> AgentEnvironmentStatus {
    let executable_path = find_executable_path(
        crate::providers::environment_profiles::executable_candidates(provider_id),
    )
    .or_else(|| find_bundled_executable_path(provider_id))
    .map(|path| path.canonicalize().unwrap_or(path));
    let executable_dir = executable_path
        .as_ref()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let executable_version = if include_version {
        executable_path
            .as_deref()
            .and_then(detect_executable_version)
    } else {
        None
    };

    AgentEnvironmentStatus {
        installed: executable_path.is_some(),
        executable_path: executable_path.as_deref().map(display_path),
        executable_dir: executable_dir.as_deref().map(display_path),
        config_path: display_path(&crate::providers::environment_profiles::config_path(
            provider_id,
        )),
        install_method: detect_install_method(executable_path.as_deref()).to_string(),
        executable_version,
    }
}

fn detect_executable_version(executable_path: &Path) -> Option<String> {
    let mut child = Command::new(executable_path)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let started_at = Instant::now();
    loop {
        if let Some(status) = child.try_wait().ok()? {
            if !status.success() {
                return None;
            }
            break;
        }
        if started_at.elapsed() >= VERSION_COMMAND_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut output = Vec::new();
    let mut stdout = child.stdout.take()?;
    std::io::Read::read_to_end(&mut stdout, &mut output).ok()?;
    let stdout = String::from_utf8_lossy(&output).trim().to_string();
    if stdout.is_empty() {
        return None;
    }
    Some(stdout)
}

fn find_executable_path(candidates: &[&str]) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    let suffixes = executable_suffixes();

    for dir in env::split_paths(&path_var) {
        for candidate in candidates {
            for suffix in &suffixes {
                let path = dir.join(format!("{candidate}{suffix}"));
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }

    None
}

fn executable_suffixes() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["", ".exe", ".cmd", ".bat"]
    } else {
        vec![""]
    }
}

fn find_bundled_executable_path(provider_id: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if crate::providers::canonical_provider_id(provider_id) != "cursor" {
            return None;
        }
        let mut app_paths = vec![PathBuf::from("/Applications/Cursor.app")];
        if let Some(home) = dirs::home_dir() {
            app_paths.push(home.join("Applications/Cursor.app"));
        }
        return find_cursor_app_executable_path(app_paths);
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = provider_id;
        None
    }
}

#[cfg(any(target_os = "macos", test))]
fn find_cursor_app_executable_path(
    app_paths: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf> {
    for app_path in app_paths {
        let executable_path = app_path.join("Contents/Resources/app/bin/cursor");
        if executable_path.is_file() {
            return Some(executable_path);
        }
    }
    None
}

pub(crate) fn provider_config_path(provider_id: &str) -> PathBuf {
    crate::providers::environment_profiles::config_path(provider_id)
}

fn detect_install_method(executable_path: Option<&Path>) -> &'static str {
    let Some(executable_path) = executable_path else {
        return "unknown";
    };
    let normalized = normalize_path(executable_path);

    if normalized.contains("/node_modules/") {
        return "npm";
    }
    if normalized.contains("/cellar/") || normalized.contains("/homebrew/") {
        return "homebrew";
    }
    if normalized.contains("/.cargo/bin/") {
        return "cargo";
    }
    if normalized.contains("/uv/tools/") {
        return "uv tool";
    }
    if normalized.contains("/pipx/venvs/") {
        return "pipx";
    }
    if normalized.contains("/site-packages/") {
        return "pip";
    }

    "manual"
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn display_path(path: &Path) -> String {
    let visible = crate::utils::user_visible_path(&path.to_string_lossy());
    let visible_path = PathBuf::from(&visible);
    if let Some(home) = dirs::home_dir() {
        if let Ok(stripped) = visible_path.strip_prefix(&home) {
            let suffix = stripped.to_string_lossy();
            if suffix.is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", suffix);
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_install_method_from_path_patterns() {
        assert_eq!(
            detect_install_method(Some(Path::new(
                "/usr/local/lib/node_modules/@openai/codex/bin/codex"
            ))),
            "npm"
        );
        assert_eq!(
            detect_install_method(Some(Path::new(
                "/Users/me/.local/share/uv/tools/opencode/bin/opencode"
            ))),
            "uv tool"
        );
        assert_eq!(
            detect_install_method(Some(Path::new("/Users/me/.cargo/bin/claude"))),
            "cargo"
        );
        assert_eq!(detect_install_method(None), "unknown");
    }

    #[test]
    fn finds_cursor_cli_inside_application_bundle() {
        let app_path = tempfile::tempdir().unwrap().path().join("Cursor.app");
        let executable_path = app_path.join("Contents/Resources/app/bin/cursor");
        std::fs::create_dir_all(executable_path.parent().unwrap()).unwrap();
        std::fs::write(&executable_path, b"#!/bin/sh\n").unwrap();

        assert_eq!(
            find_cursor_app_executable_path([app_path]),
            Some(executable_path)
        );
    }

    #[test]
    fn detect_provider_environment_uses_known_provider_defaults() {
        let codex = detect_provider_environment("codex");
        assert_eq!(
            codex.config_path,
            display_path(&crate::providers::environment_profiles::config_path(
                "codex"
            ))
        );
        assert!(!codex.install_method.trim().is_empty());
        if codex.installed {
            assert!(codex.executable_path.is_some());
        } else {
            assert_eq!(codex.install_method, "unknown");
            assert!(codex.executable_path.is_none());
        }
    }
}
