//! Provider-owned setting module registry.
//!
//! This keeps provider-specific setting-module dispatch out of `provider_settings::mod`
//! so the mod file stays focused on orchestration and shared setting behavior.

pub(super) fn find_provider_setting_module(
    provider_id: &str,
) -> Option<&'static dyn super::ProviderSettingModule> {
    let provider_id = crate::providers::canonical_provider_id(provider_id);
    match provider_id.as_str() {
        "claude" => Some(&super::claude::ClaudeSettingModule),
        "cline" => Some(&super::cline::ClineSettingModule),
        "codex" => Some(&super::codex::CodexSettingModule),
        "copilot" => Some(&super::copilot::CopilotSettingModule),
        "cursor" => Some(&super::cursor::CursorSettingModule),
        "gemini" => Some(&super::gemini::GeminiSettingModule),
        "kimi" => Some(&super::kimi::KimiSettingModule),
        "kiro" => Some(&super::kiro::KiroSettingModule),
        "opencode" => Some(&super::opencode::OpenCodeSettingModule),
        "pi" => Some(&super::pi::PiSettingModule),
        "omp" => Some(&super::omp::OmpSettingModule),
        "qwen" => Some(&super::qwen::QwenSettingModule),
        "qoder" => Some(&super::qoder::QoderSettingModule),
        "droid" => Some(&super::droid::DroidSettingModule),
        "codebuddy" => Some(&super::codebuddy::CodeBuddySettingModule),
        "codybuddycn" => Some(&super::codybuddycn::CodyBuddyCnSettingModule),
        "stepfun" => Some(&super::stepfun::StepFunSettingModule),
        "antigravity" => Some(&super::antigravity::AntiGravitySettingModule),
        "workbuddy" => Some(&super::workbuddy::WorkBuddySettingModule),
        "hermes" => Some(&super::hermes::HermesSettingModule),
        "trae_gui" => Some(&super::trae_gui::TraeGuiSettingModule),
        "traecn" => Some(&super::traecn::TraeCnSettingModule),
        "trae" => Some(&super::trae::TraeSettingModule),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_known_provider_setting_modules() {
        assert_eq!(
            find_provider_setting_module("codex")
                .expect("codex setting module")
                .provider_id(),
            "codex"
        );
        assert_eq!(
            find_provider_setting_module("opencode")
                .expect("opencode setting module")
                .provider_id(),
            "opencode"
        );
    }

    #[test]
    fn registry_resolves_setting_aliases() {
        assert_eq!(
            find_provider_setting_module("factory")
                .expect("factory alias module")
                .provider_id(),
            "droid"
        );
    }
}
