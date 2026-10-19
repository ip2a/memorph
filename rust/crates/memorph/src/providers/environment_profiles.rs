use std::path::PathBuf;

pub(crate) fn executable_candidates(provider_id: &str) -> &'static [&'static str] {
    let provider_id = crate::providers::canonical_provider_id(provider_id);
    match provider_id.as_str() {
        "antigravity" => &["antigravity"],
        "grok" => &["grok"],
        "omp" => &["omp"],
        "zcode" => &["zcode"],
        "claude" => &["claude"],
        "codex" => &["codex"],
        "copilot" => &["gh"],
        "cursor" => &["cursor-agent", "cursor"],
        "gemini" => &["gemini"],
        "kiro" => &["kiro"],
        "kimi" => &["kimi"],
        "opencode" => &["opencode"],
        "pi" => &["pi"],
        "qoder" => &["qoder"],
        "workbuddy" => &["workbuddy"],
        _ => &[],
    }
}

pub(crate) fn config_path(provider_id: &str) -> PathBuf {
    let provider_id = crate::providers::canonical_provider_id(provider_id);
    match provider_id.as_str() {
        "antigravity" => home_join(".antigravity"),
        "grok" => home_join(".grok"),
        "omp" => home_join(".omp"),
        "zcode" => home_join(".zcode"),
        "claude" => home_join(".claude"),
        "codex" => home_join(".codex"),
        "copilot" => app_config_dir("Code", ".config/Code")
            .join("User")
            .join("globalStorage"),
        "cursor" => cursor_config_dir(),
        "gemini" => home_join(".gemini"),
        "kiro" => kiro_config_dir(),
        "kimi" => home_join(".kimi"),
        "opencode" => home_join(".config/opencode"),
        "pi" => home_join(".pi/agent"),
        "qoder" => home_join(".qoder"),
        "workbuddy" => home_join(".workbuddy"),
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
        assert_eq!(config_path("antigravity"), home_join(".antigravity"));
        assert_eq!(config_path("workbuddy"), home_join(".workbuddy"));
        assert_eq!(config_path("pi"), home_join(".pi/agent"));
    }

    #[test]
    fn executable_candidates_cover_codeisland_hook_providers() {
        for provider in [
            "qoder",
            "antigravity",
            "workbuddy",
            "pi",
        ] {
            assert!(
                !executable_candidates(provider).is_empty(),
                "missing executable candidates for {provider}"
            );
        }
    }
}
