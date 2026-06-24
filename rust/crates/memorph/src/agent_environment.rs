use std::env;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AgentEnvironmentStatus {
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_dir: Option<String>,
    pub config_path: String,
    pub install_method: String,
}

pub fn detect_provider_environment(provider_id: &str) -> AgentEnvironmentStatus {
    let executable_path = find_executable_path(
        crate::providers::environment_profiles::executable_candidates(provider_id),
    )
    .map(|path| path.canonicalize().unwrap_or(path));
    let executable_dir = executable_path
        .as_ref()
        .and_then(|path| path.parent().map(Path::to_path_buf));

    AgentEnvironmentStatus {
        installed: executable_path.is_some(),
        executable_path: executable_path.as_deref().map(display_path),
        executable_dir: executable_dir.as_deref().map(display_path),
        config_path: display_path(&crate::providers::environment_profiles::config_path(
            provider_id,
        )),
        install_method: detect_install_method(executable_path.as_deref()).to_string(),
    }
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
