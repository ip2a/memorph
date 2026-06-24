//! Hook provider operation capability matrix.
//!
//! This is the single backend contract used by API/UI/diagnostics to decide
//! which hook management operations are available for a provider.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookProviderCapabilities {
    pub detect: bool,
    pub verify: bool,
    pub install: bool,
    pub repair: bool,
    pub uninstall: bool,
}

impl HookProviderCapabilities {
    pub const fn unsupported() -> Self {
        Self {
            detect: false,
            verify: false,
            install: false,
            repair: false,
            uninstall: false,
        }
    }

    pub const fn managed_hook() -> Self {
        Self {
            detect: true,
            verify: true,
            install: true,
            repair: true,
            uninstall: true,
        }
    }

    pub fn supports_setting(self, setting_id: &str) -> bool {
        match setting_id {
            "install_hook" => self.install,
            "verify_hook" => self.verify,
            "repair_hook" => self.repair,
            "uninstall_hook" => self.uninstall,
            _ => true,
        }
    }
}

pub fn for_provider(provider: &str) -> HookProviderCapabilities {
    if crate::hooks::operations::find_provider_hook(provider).is_some() {
        HookProviderCapabilities::managed_hook()
    } else {
        HookProviderCapabilities::unsupported()
    }
}

pub fn supports_setting(provider: &str, setting_id: &str) -> bool {
    for_provider(provider).supports_setting(setting_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_profiled_provider_has_full_managed_hook_capabilities() {
        for descriptor in crate::hooks::registry::all() {
            assert_eq!(
                for_provider(descriptor.provider()),
                HookProviderCapabilities::managed_hook(),
                "missing capability coverage for {}",
                descriptor.provider()
            );
        }
    }

    #[test]
    fn unknown_provider_has_no_hook_capabilities() {
        assert_eq!(
            for_provider("unknown-provider"),
            HookProviderCapabilities::unsupported()
        );
    }

    #[test]
    fn capabilities_gate_hook_settings_only() {
        let unsupported = HookProviderCapabilities::unsupported();
        assert!(!unsupported.supports_setting("install_hook"));
        assert!(!unsupported.supports_setting("verify_hook"));
        assert!(!unsupported.supports_setting("repair_hook"));
        assert!(!unsupported.supports_setting("uninstall_hook"));
        assert!(unsupported.supports_setting("repair_workspace_sessions"));
    }
}
