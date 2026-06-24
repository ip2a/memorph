use std::path::PathBuf;

use anyhow::Result;

use crate::hooks::contract::ProviderHook;
use crate::hooks::extension_file_hook::ExtensionFileHookSpec;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

const MARKER: &str = "memorph omp extension";

pub struct OmpHook;

pub static OMP_HOOK: OmpHook = OmpHook;

impl ProviderHook for OmpHook {
    fn provider_id(&self) -> &'static str {
        "omp"
    }

    fn status(&self) -> Result<HookInstallStatus> {
        crate::hooks::extension_file_hook::status(spec())
    }

    fn install(&self) -> Result<HookOperationReport> {
        crate::hooks::extension_file_hook::install(spec())
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
        crate::hooks::extension_file_hook::uninstall(spec())
    }
}

pub(crate) fn agent_dir() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".omp")
        .join("agent")
}

pub(crate) fn extension_dir() -> PathBuf {
    agent_dir().join("extensions")
}

pub(crate) fn extension_path() -> PathBuf {
    extension_dir().join("memorph.ts")
}

pub(crate) fn extension_source() -> Result<String> {
    crate::hooks::extension_file_hook::pi_agent_extension_source(
        "omp",
        MARKER,
        r#"import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent/extensibility/extensions/types";"#,
        "omp",
    )
}

fn spec() -> ExtensionFileHookSpec {
    ExtensionFileHookSpec {
        provider: "omp",
        display_name: "Oh My Pi",
        extension_dir,
        extension_path,
        marker: MARKER,
        source: extension_source,
        missing_status_message: "Oh My Pi memorph extension does not exist.",
        install_message: "Oh My Pi memorph extension installed.",
        uninstall_missing_message: "Oh My Pi memorph extension file does not exist.",
        unmanaged_uninstall_message: "Oh My Pi extension file is not managed by memorph.",
        uninstall_message: "Oh My Pi memorph extension removed.",
    }
}

#[cfg(test)]
pub(crate) fn installed_version(contents: &str) -> Option<Option<String>> {
    crate::hooks::extension_file_hook::installed_version(contents, MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::HookHealthStatus;
    use crate::hooks::test_support::TestHookHomeGuard;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = OMP_HOOK.descriptor().expect("omp descriptor");
        assert_eq!(descriptor.provider(), OMP_HOOK.provider_id());
    }
    #[test]
    fn installs_and_uninstalls_omp_extension() {
        let _home = TestHookHomeGuard::new();
        assert_eq!(
            OMP_HOOK.verify().unwrap().status.status,
            HookHealthStatus::NotInstalled
        );
        let installed = OMP_HOOK.install().unwrap();
        assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
        assert!(installed.changed);

        let path = extension_path();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("__hook-bridge"));
        assert!(contents.contains("const PROVIDER = \"omp\""));
        assert!(!contents.contains("codeisland-bridge"));

        let removed = OMP_HOOK.uninstall().unwrap();
        assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
        assert!(!path.exists());
    }
}
