use std::path::PathBuf;

use anyhow::Result;

use crate::hooks::contract::ProviderHook;
use crate::hooks::extension_file_hook::ExtensionFileHookSpec;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

const MARKER: &str = "memorph pi extension";

pub struct PiHook;

pub static PI_HOOK: PiHook = PiHook;

impl ProviderHook for PiHook {
    fn provider_id(&self) -> &'static str {
        "pi"
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
        .join(".pi")
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
        "pi",
        MARKER,
        r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";"#,
        "pi",
    )
}

fn spec() -> ExtensionFileHookSpec {
    ExtensionFileHookSpec {
        provider: "pi",
        display_name: "pi",
        extension_dir,
        extension_path,
        marker: MARKER,
        source: extension_source,
        missing_status_message: "pi memorph extension does not exist.",
        install_message: "pi memorph extension installed.",
        uninstall_missing_message: "pi memorph extension file does not exist.",
        unmanaged_uninstall_message: "pi extension file is not managed by memorph.",
        uninstall_message: "pi memorph extension removed.",
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
        let descriptor = PI_HOOK.descriptor().expect("pi descriptor");
        assert_eq!(descriptor.provider(), PI_HOOK.provider_id());
    }
    #[test]
    fn renders_pi_extension_with_memorph_bridge() {
        let pi = extension_source().unwrap();
        assert!(pi.contains("memorph pi extension"));
        assert!(pi.contains("__hook-bridge"));
        assert!(pi.contains("--provider"));
        assert!(pi.contains("const PROVIDER = \"pi\""));
        assert!(!pi.contains("codeisland-bridge"));
        assert!(!pi.contains("codeisland-"));
    }

    #[test]
    fn detects_memorph_extension_versions() {
        let current = format!(
            "// memorph pi extension\n// version: {}\nmemorph __hook-bridge\n",
            crate::hooks::shared::HOOK_MANAGED_VERSION
        );
        assert_eq!(
            installed_version(&current).flatten().as_deref(),
            Some(crate::hooks::shared::HOOK_MANAGED_VERSION)
        );
        assert!(installed_version("// unrelated extension").is_none());
    }
    #[test]
    fn installs_and_uninstalls_pi_extension() {
        let _home = TestHookHomeGuard::new();
        assert_eq!(
            PI_HOOK.verify().unwrap().status.status,
            HookHealthStatus::NotInstalled
        );
        let installed = PI_HOOK.install().unwrap();
        assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
        assert!(installed.changed);

        let path = extension_path();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("__hook-bridge"));
        assert!(contents.contains("const PROVIDER = \"pi\""));
        assert!(!contents.contains("codeisland-bridge"));

        let removed = PI_HOOK.uninstall().unwrap();
        assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
        assert!(!path.exists());
    }
}
