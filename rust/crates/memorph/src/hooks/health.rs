//! Hook verification and repair.

use crate::hooks::installer;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus};
use anyhow::Result;

pub fn status(provider: &str) -> Result<HookInstallStatus> {
    match provider {
        "claude" => claude_status(),
        "cline" => cline_status(),
        "codex" => codex_status(),
        "copilot" => copilot_status(),
        "cursor" => cursor_status(),
        "gemini" => gemini_status(),
        "kimi" => kimi_status(),
        "kiro" => kiro_status(),
        "opencode" => opencode_status(),
        "qwen" => qwen_status(),
        "trae_gui" => trae_gui_status(),
        "traecn" => traecn_status(),
        "qoder" => qoder_status(),
        "droid" | "factory" => droid_status(),
        "codebuddy" => codebuddy_status(),
        "codybuddycn" => codybuddycn_status(),
        "stepfun" => stepfun_status(),
        "antigravity" => antigravity_status(),
        "workbuddy" => workbuddy_status(),
        "hermes" => hermes_status(),
        "pi" => pi_status(),
        "omp" | "oh-my-pi" | "oh_my_pi" => omp_status(),
        "trae" | "traecli" => trae_status(),
        _ => Ok(HookInstallStatus {
            provider: provider.to_string(),
            status: HookHealthStatus::Unsupported,
            config_path: None,
            installed_version: None,
            current_version: None,
            message: Some(format!(
                "Hook management is not implemented for provider: {provider}"
            )),
            last_event_at: last_event_at(provider),
        }),
    }
}

pub fn verify(provider: &str) -> Result<HookInstallStatus> {
    status(provider)
}

fn claude_status() -> Result<HookInstallStatus> {
    let path = installer::claude_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "claude".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Claude settings.json does not exist.".to_string()),
            last_event_at: last_event_at("claude"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::claude_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::claude_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::claude_required_events()
            .iter()
            .filter_map(|event| installer::claude_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::claude_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Claude memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Claude memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "claude".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("claude"),
    })
}

fn cline_status() -> Result<HookInstallStatus> {
    let dirs = installer::cline_hooks_dirs();
    let config_path = Some(
        dirs.iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    );
    if !dirs.iter().any(|dir| dir.exists()) {
        return Ok(HookInstallStatus {
            provider: "cline".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Cline hooks directory does not exist.".to_string()),
            last_event_at: last_event_at("cline"),
        });
    }

    let missing: Vec<&str> = installer::cline_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::cline_event_has_memorph_hook(event))
        .collect();
    let installed_version = summarize_versions(
        installer::cline_required_events()
            .iter()
            .filter_map(|event| installer::cline_event_memorph_hook_version(event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::cline_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Cline memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Cline memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook files: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "cline".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("cline"),
    })
}

fn codex_status() -> Result<HookInstallStatus> {
    let hooks_path = installer::codex_hooks_path();
    let config_path = installer::codex_config_path();
    let config_path_text = Some(hooks_path.display().to_string());
    if !hooks_path.exists() {
        return Ok(HookInstallStatus {
            provider: "codex".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path: config_path_text,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Codex hooks.json does not exist.".to_string()),
            last_event_at: last_event_at("codex"),
        });
    }

    let root = installer::read_json_object_or_empty(&hooks_path)?;
    let missing: Vec<&str> = installer::codex_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::codex_event_has_memorph_hook(&root, event))
        .collect();
    let feature_enabled = std::fs::read_to_string(&config_path)
        .ok()
        .map(|contents| installer::codex_hooks_feature_enabled(&contents))
        .unwrap_or(false);
    let installed_version = summarize_versions(
        installer::codex_required_events()
            .iter()
            .filter_map(|event| installer::codex_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());

    let status = if missing.is_empty() && feature_enabled && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::codex_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };

    let message = match (missing.is_empty(), feature_enabled, stale) {
        (true, true, false) => Some("Codex memorph hooks are installed and enabled.".to_string()),
        (true, _, true) => Some(format!(
            "Codex memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        )),
        (false, true, _) => Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        )),
        (true, false, _) => Some(format!(
            "Codex hook entries exist, but hooks = true is missing in {}.",
            config_path.display()
        )),
        (false, false, _) => Some(format!(
            "Missing memorph hook events: {}; hooks = true is missing in {}.",
            missing.join(", "),
            config_path.display()
        )),
    };

    Ok(HookInstallStatus {
        provider: "codex".to_string(),
        status,
        config_path: config_path_text,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("codex"),
    })
}

fn copilot_status() -> Result<HookInstallStatus> {
    let path = installer::copilot_hooks_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "copilot".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Copilot hook file does not exist.".to_string()),
            last_event_at: last_event_at("copilot"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::copilot_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::copilot_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::copilot_required_events()
            .iter()
            .filter_map(|event| installer::copilot_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::copilot_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Copilot memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Copilot memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "copilot".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("copilot"),
    })
}

fn cursor_status() -> Result<HookInstallStatus> {
    let path = installer::cursor_hooks_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "cursor".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Cursor hooks.json does not exist.".to_string()),
            last_event_at: last_event_at("cursor"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::cursor_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::cursor_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::cursor_required_events()
            .iter()
            .filter_map(|event| installer::cursor_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::cursor_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Cursor memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Cursor memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "cursor".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("cursor"),
    })
}

fn trae_gui_status() -> Result<HookInstallStatus> {
    let path = installer::trae_gui_hooks_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "trae_gui".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Trae hooks.json does not exist.".to_string()),
            last_event_at: last_event_at("trae_gui"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::trae_gui_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::trae_gui_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::trae_gui_required_events()
            .iter()
            .filter_map(|event| installer::trae_gui_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::trae_gui_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Trae memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Trae memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "trae_gui".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("trae_gui"),
    })
}

fn traecn_status() -> Result<HookInstallStatus> {
    let path = installer::traecn_hooks_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "traecn".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Trae CN hooks.json does not exist.".to_string()),
            last_event_at: last_event_at("traecn"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::traecn_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::traecn_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::traecn_required_events()
            .iter()
            .filter_map(|event| installer::traecn_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::traecn_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Trae CN memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Trae CN memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "traecn".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("traecn"),
    })
}

fn gemini_status() -> Result<HookInstallStatus> {
    let path = installer::gemini_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "gemini".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Gemini settings.json does not exist.".to_string()),
            last_event_at: last_event_at("gemini"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::gemini_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::gemini_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::gemini_required_events()
            .iter()
            .filter_map(|event| installer::gemini_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::gemini_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Gemini memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Gemini memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "gemini".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("gemini"),
    })
}

fn kimi_status() -> Result<HookInstallStatus> {
    let path = installer::kimi_config_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "kimi".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Kimi config.toml does not exist.".to_string()),
            last_event_at: last_event_at("kimi"),
        });
    }

    let contents = std::fs::read_to_string(&path)?;
    let missing: Vec<&str> = installer::kimi_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::kimi_contents_contains_memorph_hook(&contents, event))
        .collect();
    let installed_version = summarize_versions(
        installer::kimi_required_events()
            .iter()
            .filter_map(|event| installer::kimi_event_memorph_hook_version(&contents, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::kimi_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Kimi memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Kimi memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "kimi".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("kimi"),
    })
}

fn kiro_status() -> Result<HookInstallStatus> {
    let path = installer::kiro_agent_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "kiro".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Kiro memorph agent file does not exist.".to_string()),
            last_event_at: last_event_at("kiro"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::kiro_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::kiro_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::kiro_required_events()
            .iter()
            .filter_map(|event| installer::kiro_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::kiro_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Kiro memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Kiro memorph hooks are installed. Launch with `kiro --agent memorph`.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "kiro".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("kiro"),
    })
}

fn qwen_status() -> Result<HookInstallStatus> {
    let path = installer::qwen_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "qwen".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Qwen settings.json does not exist.".to_string()),
            last_event_at: last_event_at("qwen"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::qwen_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::qwen_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::qwen_required_events()
            .iter()
            .filter_map(|event| installer::qwen_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::qwen_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Qwen memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Qwen memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "qwen".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("qwen"),
    })
}

fn qoder_status() -> Result<HookInstallStatus> {
    let path = installer::qoder_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "qoder".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Qoder settings.json does not exist.".to_string()),
            last_event_at: last_event_at("qoder"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::qoder_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::qoder_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::qoder_required_events()
            .iter()
            .filter_map(|event| installer::qoder_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::qoder_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Qoder memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Qoder memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "qoder".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("qoder"),
    })
}

fn droid_status() -> Result<HookInstallStatus> {
    let path = installer::droid_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "droid".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Factory settings.json does not exist.".to_string()),
            last_event_at: last_event_at("droid"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::droid_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::droid_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::droid_required_events()
            .iter()
            .filter_map(|event| installer::droid_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::droid_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Factory memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Factory memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "droid".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("droid"),
    })
}

fn codebuddy_status() -> Result<HookInstallStatus> {
    let path = installer::codebuddy_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "codebuddy".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("CodeBuddy settings.json does not exist.".to_string()),
            last_event_at: last_event_at("codebuddy"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::codebuddy_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::codebuddy_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::codebuddy_required_events()
            .iter()
            .filter_map(|event| installer::codebuddy_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::codebuddy_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "CodeBuddy memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("CodeBuddy memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "codebuddy".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("codebuddy"),
    })
}

fn codybuddycn_status() -> Result<HookInstallStatus> {
    let path = installer::codybuddycn_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "codybuddycn".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("CodyBuddyCN settings.json does not exist.".to_string()),
            last_event_at: last_event_at("codybuddycn"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::codybuddycn_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::codybuddycn_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::codybuddycn_required_events()
            .iter()
            .filter_map(|event| installer::codybuddycn_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::codybuddycn_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "CodyBuddyCN memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("CodyBuddyCN memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "codybuddycn".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("codybuddycn"),
    })
}

fn stepfun_status() -> Result<HookInstallStatus> {
    let path = installer::stepfun_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "stepfun".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("StepFun settings.json does not exist.".to_string()),
            last_event_at: last_event_at("stepfun"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::stepfun_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::stepfun_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::stepfun_required_events()
            .iter()
            .filter_map(|event| installer::stepfun_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::stepfun_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "StepFun memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("StepFun memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "stepfun".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("stepfun"),
    })
}

fn antigravity_status() -> Result<HookInstallStatus> {
    let path = installer::antigravity_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "antigravity".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("AntiGravity settings.json does not exist.".to_string()),
            last_event_at: last_event_at("antigravity"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::antigravity_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::antigravity_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::antigravity_required_events()
            .iter()
            .filter_map(|event| installer::antigravity_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::antigravity_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "AntiGravity memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("AntiGravity memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "antigravity".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("antigravity"),
    })
}

fn workbuddy_status() -> Result<HookInstallStatus> {
    let path = installer::workbuddy_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "workbuddy".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("WorkBuddy settings.json does not exist.".to_string()),
            last_event_at: last_event_at("workbuddy"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::workbuddy_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::workbuddy_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::workbuddy_required_events()
            .iter()
            .filter_map(|event| installer::workbuddy_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::workbuddy_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "WorkBuddy memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("WorkBuddy memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "workbuddy".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("workbuddy"),
    })
}

fn hermes_status() -> Result<HookInstallStatus> {
    let path = installer::hermes_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "hermes".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Hermes settings.json does not exist.".to_string()),
            last_event_at: last_event_at("hermes"),
        });
    }

    let root = installer::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = installer::hermes_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::hermes_event_has_memorph_hook(&root, event))
        .collect();
    let installed_version = summarize_versions(
        installer::hermes_required_events()
            .iter()
            .filter_map(|event| installer::hermes_event_memorph_hook_version(&root, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::hermes_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Hermes memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Hermes memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "hermes".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("hermes"),
    })
}

fn trae_status() -> Result<HookInstallStatus> {
    let path = installer::traecli_config_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "trae".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("TraeCli traecli.yaml does not exist.".to_string()),
            last_event_at: last_event_at("trae"),
        });
    }

    let contents = std::fs::read_to_string(&path)?;
    let missing: Vec<&str> = installer::trae_required_events()
        .iter()
        .copied()
        .filter(|event| !installer::trae_contents_contains_memorph_hook(&contents, event))
        .collect();
    let installed_version = summarize_versions(
        installer::trae_required_events()
            .iter()
            .filter_map(|event| installer::trae_event_memorph_hook_version(&contents, event)),
    );
    let current_version = Some(installer::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref() != Some(installer::current_hook_managed_version());
    let status = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == installer::trae_required_events().len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "TraeCli memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("TraeCli memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "trae".to_string(),
        status,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: last_event_at("trae"),
    })
}

fn pi_status() -> Result<HookInstallStatus> {
    let path = installer::pi_extension_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "pi".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("pi memorph extension does not exist.".to_string()),
            last_event_at: last_event_at("pi"),
        });
    }
    let contents = std::fs::read_to_string(&path)?;
    let installed_version = installer::pi_extension_installed_version(&contents).flatten();
    let status = match installer::pi_extension_installed_version(&contents) {
        None => HookHealthStatus::NotInstalled,
        Some(version) if version.as_deref() == Some(installer::current_hook_managed_version()) => {
            HookHealthStatus::InstalledOk
        }
        Some(_) => HookHealthStatus::InstalledStaleBinary,
    };
    let message = match status {
        HookHealthStatus::InstalledOk => Some("pi memorph extension is installed.".to_string()),
        HookHealthStatus::InstalledStaleBinary => Some(format!(
            "pi memorph extension is installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        )),
        _ => Some("pi extension file is not managed by memorph.".to_string()),
    };
    Ok(HookInstallStatus {
        provider: "pi".to_string(),
        status,
        config_path,
        installed_version,
        current_version: Some(installer::current_hook_managed_version().to_string()),
        message,
        last_event_at: last_event_at("pi"),
    })
}

fn omp_status() -> Result<HookInstallStatus> {
    let path = installer::omp_extension_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "omp".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(installer::current_hook_managed_version().to_string()),
            message: Some("Oh My Pi memorph extension does not exist.".to_string()),
            last_event_at: last_event_at("omp"),
        });
    }
    let contents = std::fs::read_to_string(&path)?;
    let installed_version = installer::omp_extension_installed_version(&contents).flatten();
    let status = match installer::omp_extension_installed_version(&contents) {
        None => HookHealthStatus::NotInstalled,
        Some(version) if version.as_deref() == Some(installer::current_hook_managed_version()) => {
            HookHealthStatus::InstalledOk
        }
        Some(_) => HookHealthStatus::InstalledStaleBinary,
    };
    let message = match status {
        HookHealthStatus::InstalledOk => {
            Some("Oh My Pi memorph extension is installed.".to_string())
        }
        HookHealthStatus::InstalledStaleBinary => Some(format!(
            "Oh My Pi memorph extension is installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            installer::current_hook_managed_version()
        )),
        _ => Some("Oh My Pi extension file is not managed by memorph.".to_string()),
    };
    Ok(HookInstallStatus {
        provider: "omp".to_string(),
        status,
        config_path,
        installed_version,
        current_version: Some(installer::current_hook_managed_version().to_string()),
        message,
        last_event_at: last_event_at("omp"),
    })
}

fn opencode_status() -> Result<HookInstallStatus> {
    let config_dir = installer::opencode_config_dir();
    let plugin_path = installer::opencode_plugin_path();
    if !config_dir.exists() {
        return Ok(HookInstallStatus {
            provider: "opencode".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path: Some(config_dir.display().to_string()),
            installed_version: None,
            current_version: Some(installer::current_opencode_plugin_version().to_string()),
            message: Some("OpenCode config directory does not exist.".to_string()),
            last_event_at: last_event_at("opencode"),
        });
    }

    let installed = installer::opencode_plugin_installed();
    let installed_version = installer::opencode_installed_plugin_version();
    let current_version = Some(installer::current_opencode_plugin_version().to_string());
    let stale = installed_version
        .as_deref()
        .map(|version| version != installer::current_opencode_plugin_version())
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
        last_event_at: last_event_at("opencode"),
    })
}

fn summarize_versions(versions: impl Iterator<Item = Option<String>>) -> Option<String> {
    let mut normalized: Vec<String> = versions
        .map(|version| version.unwrap_or_else(|| "legacy".to_string()))
        .collect();
    normalized.sort();
    normalized.dedup();
    match normalized.len() {
        0 => None,
        1 => normalized.pop(),
        _ => Some("mixed".to_string()),
    }
}

fn last_event_at(provider: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    crate::hooks::store::load_recent_events(500)
        .ok()
        .and_then(|events| {
            events
                .into_iter()
                .filter(|event| event.provider == provider)
                .map(|event| event.timestamp)
                .max()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn unsupported_provider_reports_unsupported() {
        let status = status("unknown-provider").unwrap();
        assert_eq!(status.status, HookHealthStatus::Unsupported);
    }

    #[test]
    fn codex_feature_flag_reader_detects_features_hooks_true() {
        assert!(installer::codex_hooks_feature_enabled(
            "[features]\nhooks = true\n"
        ));
        assert!(!installer::codex_hooks_feature_enabled(
            "[features]\nhooks = false\n"
        ));
    }

    #[test]
    fn opencode_config_reader_detects_plugin_ref() {
        assert!(installer::opencode_config_contains_memorph_plugin(
            r#"{ "plugin": ["file:///Users/me/.config/opencode/plugins/memorph.js"] }"#
        ));
        assert!(installer::opencode_config_contains_memorph_plugin(
            r#"{ // comment
              "plugin": "file:///Users/me/.config/opencode/plugins/memorph.js"
            }"#
        ));
    }

    #[test]
    fn empty_claude_config_is_not_installed_shape() {
        let root = serde_json::Map::<String, Value>::new();
        assert!(!installer::claude_event_has_memorph_hook(
            &root,
            "PreToolUse"
        ));
    }

    #[test]
    fn summarizes_legacy_and_current_hook_versions() {
        assert_eq!(
            summarize_versions(vec![None].into_iter()).as_deref(),
            Some("legacy")
        );
        assert_eq!(
            summarize_versions(
                vec![Some(installer::current_hook_managed_version().to_string())].into_iter()
            )
            .as_deref(),
            Some(installer::current_hook_managed_version())
        );
        assert_eq!(
            summarize_versions(
                vec![
                    None,
                    Some(installer::current_hook_managed_version().to_string())
                ]
                .into_iter()
            )
            .as_deref(),
            Some("mixed")
        );
    }
}
