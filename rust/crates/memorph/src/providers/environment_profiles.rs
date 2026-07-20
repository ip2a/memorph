use std::path::PathBuf;

pub(crate) fn executable_candidates(provider_id: &str) -> &'static [&'static str] {
    let provider_id = crate::providers::canonical_provider_id(provider_id);
    match provider_id.as_str() {
        "antigravity" => &["antigravity"],
        "claude" => &["claude"],
        "codex" => &["codex"],
        "cline" => &["cline"],
        "copilot" => &["gh"],
        "cursor" => &["cursor-agent", "cursor"],
        "deepseek" => &["deepseek"],
        "codebuddy" => &["codebuddy"],
        "droid" => &["factory", "droid"],
        "gemini" => &["gemini"],
        "hermes" => &["hermes"],
        "kiro" => &["kiro"],
        "kimi" => &["kimi"],
        "opencode" => &["opencode"],
        "pi" => &["pi"],
        "qoder" => &["qoder"],
        "qwen" => &["qwen"],
        "trae" => &["trae"],
        "workbuddy" => &["workbuddy"],
        "windsurf" => &["windsurf"],
        _ => &[],
    }
}

pub(crate) fn config_path(provider_id: &str) -> PathBuf {
    let provider_id = crate::providers::canonical_provider_id(provider_id);
    match provider_id.as_str() {
        "antigravity" => home_join(".antigravity"),
        "claude" => home_join(".claude"),
        "codex" => home_join(".codex"),
        "cline" => home_join("Documents/Cline"),
        "copilot" => app_config_dir("Code", ".config/Code")
            .join("User")
            .join("globalStorage"),
        "cursor" => cursor_config_dir(),
        "deepseek" => home_join(".deepseek"),
        "codebuddy" => home_join(".codebuddy"),
        "droid" => home_join(".factory"),
        "gemini" => home_join(".gemini"),
        "hermes" => home_join(".hermes"),
        "kiro" => kiro_config_dir(),
        "kimi" => home_join(".kimi"),
        "opencode" => home_join(".config/opencode"),
        "pi" => home_join(".pi/agent"),
        "qoder" => home_join(".qoder"),
        "qwen" => home_join(".qwen"),
        "trae" => home_join(".trae"),
        "workbuddy" => home_join(".workbuddy"),
        "windsurf" => app_config_dir("Windsurf", ".config/Windsurf"),
        other => PathBuf::from(other),
    }
}

fn home_join(relative: &str) -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(relative))
        .unwrap_or_else(|| PathBuf::from(relative))
}

fn app_config_dir(macos_app: &str, _linux_relative: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        return home_join(&format!("Library/Application Support/{macos_app}"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
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

fn cursor_config_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        return home_join("Library/Application Support/Cursor");
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
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
        if let Ok(appdata) = std::env::var("APPDATA") {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_maps_known_roots() {
        assert_eq!(config_path("codex"), home_join(".codex"));
        assert_eq!(config_path("opencode"), home_join(".config/opencode"));
        assert_eq!(config_path("gemini"), home_join(".gemini"));
        assert_eq!(config_path("kimi"), home_join(".kimi"));
    }

    #[test]
    fn config_path_maps_codeisland_hook_roots() {
        assert_eq!(config_path("qoder"), home_join(".qoder"));
        assert_eq!(config_path("droid"), home_join(".factory"));
        assert_eq!(config_path("factory"), home_join(".factory"));
        assert_eq!(config_path("codebuddy"), home_join(".codebuddy"));
        assert_eq!(config_path("antigravity"), home_join(".antigravity"));
        assert_eq!(config_path("workbuddy"), home_join(".workbuddy"));
        assert_eq!(config_path("hermes"), home_join(".hermes"));
        assert_eq!(config_path("pi"), home_join(".pi/agent"));
    }

    #[test]
    fn executable_candidates_cover_codeisland_hook_providers() {
        for provider in [
            "qoder",
            "droid",
            "codebuddy",
            "antigravity",
            "workbuddy",
            "hermes",
            "pi",
        ] {
            assert!(
                !executable_candidates(provider).is_empty(),
                "missing executable candidates for {provider}"
            );
        }
    }
}
