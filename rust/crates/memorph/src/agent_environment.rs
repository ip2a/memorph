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
    let executable_path = find_executable_path(executable_candidates(provider_id))
        .map(|path| path.canonicalize().unwrap_or(path));
    let executable_dir = executable_path
        .as_ref()
        .and_then(|path| path.parent().map(Path::to_path_buf));

    AgentEnvironmentStatus {
        installed: executable_path.is_some(),
        executable_path: executable_path.as_deref().map(display_path),
        executable_dir: executable_dir.as_deref().map(display_path),
        config_path: display_path(&provider_config_path(provider_id)),
        install_method: detect_install_method(executable_path.as_deref()).to_string(),
    }
}

fn executable_candidates(provider_id: &str) -> &'static [&'static str] {
    match provider_id {
        "antigravity" => &["antigravity"],
        "claude" => &["claude"],
        "codex" => &["codex"],
        "cline" => &["cline"],
        "copilot" => &["gh"],
        "cursor" => &["cursor-agent", "cursor"],
        "deepseek" => &["deepseek"],
        "cidebuddy" | "codebuddy" => &["codebuddy", "cidebuddy"],
        "codybuddycn" => &["codybuddycn", "codybuddy", "codebuddy-cn"],
        "droid" | "factory" => &["factory", "droid"],
        "gemini" => &["gemini"],
        "hermes" => &["hermes"],
        "kiro" => &["kiro"],
        "kimi" => &["kimi"],
        "omp" => &["omp", "oh-my-pi"],
        "opencode" => &["opencode"],
        "pi" => &["pi"],
        "qoder" => &["qoder"],
        "qwen" => &["qwen"],
        "stepfun" => &["stepfun"],
        "trae" => &["trae"],
        "trae_gui" => &["trae"],
        "traecn" => &["trae-cn", "traecn"],
        "workbuddy" => &["workbuddy"],
        "windsurf" => &["windsurf"],
        _ => &[],
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
    match provider_id {
        "antigravity" => home_join(".antigravity"),
        "claude" => home_join(".claude"),
        "codex" => home_join(".codex"),
        "cline" => cline_config_dir(),
        "copilot" => copilot_config_dir(),
        "cursor" => cursor_config_dir(),
        "deepseek" => home_join(".deepseek"),
        "cidebuddy" | "codebuddy" => codebuddy_config_dir(),
        "codybuddycn" => home_join(".codybuddycn"),
        "droid" | "factory" => home_join(".factory"),
        "gemini" => home_join(".gemini"),
        "hermes" => home_join(".hermes"),
        "kiro" => kiro_config_dir(),
        "kimi" => home_join(".kimi"),
        "omp" => home_join(".omp/agent"),
        "opencode" => home_join(".config/opencode"),
        "pi" => home_join(".pi/agent"),
        "qoder" => home_join(".qoder"),
        "qwen" => home_join(".qwen"),
        "stepfun" => home_join(".stepfun"),
        "trae" => home_join(".trae"),
        "trae_gui" => home_join(".trae"),
        "traecn" => home_join(".trae-cn"),
        "workbuddy" => home_join(".workbuddy"),
        "windsurf" => windsurf_config_dir(),
        _ => PathBuf::from(provider_id),
    }
}

fn home_join(relative: &str) -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(relative))
        .unwrap_or_else(|| PathBuf::from(relative))
}

fn cline_config_dir() -> PathBuf {
    home_join("Documents/Cline")
}

fn cursor_config_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        return home_join("Library/Application Support/Cursor");
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata).join("Cursor");
        }
        return PathBuf::from("Cursor");
    }

    #[cfg(target_os = "linux")]
    {
        return home_join(".config/Cursor");
    }

    #[allow(unreachable_code)]
    PathBuf::from("Cursor")
}

fn kiro_config_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        return home_join("Library/Application Support/Kiro");
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata).join("Kiro");
        }
        return PathBuf::from("Kiro");
    }

    #[cfg(target_os = "linux")]
    {
        return home_join(".config/Kiro");
    }

    #[allow(unreachable_code)]
    PathBuf::from("Kiro")
}

fn windsurf_config_dir() -> PathBuf {
    app_config_dir("Windsurf", ".config/Windsurf")
}

fn codebuddy_config_dir() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".codebuddy"))
        .unwrap_or_else(|| PathBuf::from(".codebuddy"))
}

fn copilot_config_dir() -> PathBuf {
    app_config_dir("Code", ".config/Code")
        .join("User")
        .join("globalStorage")
}

fn app_config_dir(macos_app: &str, _linux_relative: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        return home_join(&format!("Library/Application Support/{macos_app}"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata).join(macos_app);
        }
        return PathBuf::from(macos_app);
    }

    #[cfg(target_os = "linux")]
    {
        return home_join(_linux_relative);
    }

    #[allow(unreachable_code)]
    PathBuf::from(macos_app)
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
    fn provider_config_path_maps_known_roots() {
        assert_eq!(provider_config_path("codex"), home_join(".codex"));
        assert_eq!(
            provider_config_path("opencode"),
            home_join(".config/opencode")
        );
        assert_eq!(provider_config_path("gemini"), home_join(".gemini"));
        assert_eq!(provider_config_path("kimi"), home_join(".kimi"));
    }

    #[test]
    fn provider_config_path_maps_codeisland_hook_roots() {
        assert_eq!(provider_config_path("qoder"), home_join(".qoder"));
        assert_eq!(provider_config_path("droid"), home_join(".factory"));
        assert_eq!(provider_config_path("codebuddy"), home_join(".codebuddy"));
        assert_eq!(
            provider_config_path("codybuddycn"),
            home_join(".codybuddycn")
        );
        assert_eq!(provider_config_path("stepfun"), home_join(".stepfun"));
        assert_eq!(
            provider_config_path("antigravity"),
            home_join(".antigravity")
        );
        assert_eq!(provider_config_path("workbuddy"), home_join(".workbuddy"));
        assert_eq!(provider_config_path("hermes"), home_join(".hermes"));
        assert_eq!(provider_config_path("trae_gui"), home_join(".trae"));
        assert_eq!(provider_config_path("traecn"), home_join(".trae-cn"));
        assert_eq!(provider_config_path("pi"), home_join(".pi/agent"));
        assert_eq!(provider_config_path("omp"), home_join(".omp/agent"));
    }

    #[test]
    fn executable_candidates_cover_codeisland_hook_providers() {
        for provider in [
            "qoder",
            "droid",
            "codebuddy",
            "codybuddycn",
            "stepfun",
            "antigravity",
            "workbuddy",
            "hermes",
            "trae_gui",
            "traecn",
            "pi",
            "omp",
        ] {
            assert!(
                !executable_candidates(provider).is_empty(),
                "missing executable candidates for {provider}"
            );
        }
    }

    #[test]
    fn detect_provider_environment_uses_known_provider_defaults() {
        let codex = detect_provider_environment("codex");
        assert_eq!(codex.config_path, display_path(&home_join(".codex")));
        assert!(!codex.install_method.trim().is_empty());
        if codex.installed {
            assert!(codex.executable_path.is_some());
        } else {
            assert_eq!(codex.install_method, "unknown");
            assert!(codex.executable_path.is_none());
        }
    }
}
