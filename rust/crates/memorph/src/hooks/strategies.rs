//! Hook configuration strategy metadata.
//!
//! This module keeps shared hook configuration enums. Operation execution lives
//! in `hooks::operations` and provider-specific behavior belongs in provider
//! modules.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::hooks::model::{HookInstallStatus, HookOperationReport};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookConfigStrategyKind {
    ClaudeLikeJson,
    ClineFiles,
    CodexJson,
    CopilotJson,
    FlatJson,
    GeminiNestedJson,
    KimiToml,
    KiroAgentJson,
    OpenCodePlugin,
    PiExtension,
    OmpExtension,
    TraeYaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookConfigOperation {
    Install,
    Verify,
    Repair,
    Uninstall,
}

impl HookConfigOperation {
    pub fn from_setting_id(setting_id: &str) -> Option<Self> {
        match setting_id {
            "install" | "install_hook" => Some(Self::Install),
            "verify" | "verify_hook" => Some(Self::Verify),
            "repair" | "repair_hook" => Some(Self::Repair),
            "uninstall" | "uninstall_hook" => Some(Self::Uninstall),
            _ => None,
        }
    }

    pub fn setting_id(self) -> &'static str {
        match self {
            Self::Install => "install_hook",
            Self::Verify => "verify_hook",
            Self::Repair => "repair_hook",
            Self::Uninstall => "uninstall_hook",
        }
    }
}

pub fn status(provider: &str) -> Result<HookInstallStatus> {
    crate::hooks::operations::status(provider)
}

pub fn run_operation(
    provider: &str,
    operation: HookConfigOperation,
) -> Result<HookOperationReport> {
    crate::hooks::operations::run_operation(provider, operation)
}

pub fn run_setting_operation(provider: &str, setting_id: &str) -> Result<HookOperationReport> {
    let operation = HookConfigOperation::from_setting_id(setting_id)
        .ok_or_else(|| anyhow::anyhow!("Unknown hook config operation: {setting_id}"))?;
    crate::hooks::operations::run_operation(provider, operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_parses_short_and_setting_names() {
        assert_eq!(
            HookConfigOperation::from_setting_id("install"),
            Some(HookConfigOperation::Install)
        );
        assert_eq!(
            HookConfigOperation::from_setting_id("verify_hook"),
            Some(HookConfigOperation::Verify)
        );
        assert_eq!(HookConfigOperation::from_setting_id("approve"), None);
        assert_eq!(HookConfigOperation::Repair.setting_id(), "repair_hook");
    }
}
