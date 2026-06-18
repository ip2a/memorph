//! Hook doctor: one-shot verification and optional repair for supported hooks.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::hooks::model::{HookHealthStatus, HookInstallStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectedProvider {
    id: &'static str,
    explicitly_requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HookDoctorRequest {
    #[serde(default)]
    pub repair: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookDoctorReport {
    pub checked: usize,
    pub repaired: usize,
    pub failed: usize,
    pub results: Vec<HookDoctorProviderResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookDoctorProviderResult {
    pub provider: String,
    pub before: HookInstallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<HookInstallStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<crate::hooks::installer::HookOperationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn run(request: HookDoctorRequest) -> HookDoctorReport {
    let providers = selected_providers(&request.providers);
    let mut results = Vec::with_capacity(providers.len());
    let mut repaired = 0;
    let mut failed = 0;

    for provider in providers {
        let before = match crate::hooks::health::status(provider.id) {
            Ok(status) => status,
            Err(error) => {
                failed += 1;
                results.push(HookDoctorProviderResult {
                    provider: provider.id.to_string(),
                    before: unsupported_status(provider.id, error.to_string()),
                    after: None,
                    operation: None,
                    error: Some(error.to_string()),
                });
                continue;
            }
        };

        if request.repair && should_repair(provider, &before.status) {
            match crate::hooks::installer::repair(provider.id) {
                Ok(operation) => {
                    repaired += usize::from(operation.changed);
                    results.push(HookDoctorProviderResult {
                        provider: provider.id.to_string(),
                        before,
                        after: Some(operation.status.clone()),
                        operation: Some(operation),
                        error: None,
                    });
                }
                Err(error) => {
                    failed += 1;
                    results.push(HookDoctorProviderResult {
                        provider: provider.id.to_string(),
                        before,
                        after: None,
                        operation: None,
                        error: Some(error.to_string()),
                    });
                }
            }
        } else {
            results.push(HookDoctorProviderResult {
                provider: provider.id.to_string(),
                before,
                after: None,
                operation: None,
                error: None,
            });
        }
    }

    HookDoctorReport {
        checked: results.len(),
        repaired,
        failed,
        results,
    }
}

fn selected_providers(requested: &[String]) -> Vec<SelectedProvider> {
    if requested.is_empty() {
        return crate::hooks::profiles::provider_ids()
            .map(|id| SelectedProvider {
                id,
                explicitly_requested: false,
            })
            .collect();
    }
    requested
        .iter()
        .filter_map(|provider| {
            crate::hooks::profiles::find(provider).map(|profile| SelectedProvider {
                id: profile.provider,
                explicitly_requested: true,
            })
        })
        .collect()
}

fn should_repair(provider: SelectedProvider, status: &HookHealthStatus) -> bool {
    match status {
        HookHealthStatus::Repairable | HookHealthStatus::InstalledStaleBinary => true,
        HookHealthStatus::NotInstalled => {
            provider.explicitly_requested || provider_appears_available_for_hook_repair(provider.id)
        }
        _ => false,
    }
}

fn provider_appears_available_for_hook_repair(provider: &str) -> bool {
    let environment = crate::agent_environment::detect_provider_environment(provider);
    environment.installed || crate::agent_environment::provider_config_path(provider).exists()
}

fn unsupported_status(provider: &str, message: String) -> HookInstallStatus {
    HookInstallStatus {
        provider: provider.to_string(),
        status: HookHealthStatus::Unsupported,
        config_path: None,
        installed_version: None,
        current_version: None,
        message: Some(message),
        last_event_at: None,
    }
}

pub fn verify(request: HookDoctorRequest) -> Result<HookDoctorReport> {
    Ok(run(request))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_defaults_to_all_profiled_providers() {
        let report = run(HookDoctorRequest::default());
        assert_eq!(report.checked, crate::hooks::profiles::all().len());
        assert_eq!(report.results.len(), report.checked);
    }

    #[test]
    fn doctor_filters_requested_providers_through_profiles() {
        let report = run(HookDoctorRequest {
            repair: false,
            providers: vec!["claude".to_string(), "missing".to_string()],
        });
        assert_eq!(report.checked, 1);
        assert_eq!(report.results[0].provider, "claude");
    }

    #[test]
    fn doctor_default_repair_does_not_install_missing_absent_providers() {
        let selected = SelectedProvider {
            id: "provider-that-does-not-exist",
            explicitly_requested: false,
        };
        assert!(!should_repair(selected, &HookHealthStatus::NotInstalled));
    }

    #[test]
    fn doctor_explicit_repair_can_install_missing_provider_hooks() {
        let selected = SelectedProvider {
            id: "provider-that-does-not-exist",
            explicitly_requested: true,
        };
        assert!(should_repair(selected, &HookHealthStatus::NotInstalled));
    }

    #[test]
    fn doctor_default_repair_still_repairs_stale_or_partial_hooks() {
        let selected = SelectedProvider {
            id: "provider-that-does-not-exist",
            explicitly_requested: false,
        };
        assert!(should_repair(
            selected,
            &HookHealthStatus::InstalledStaleBinary
        ));
        assert!(should_repair(selected, &HookHealthStatus::Repairable));
    }
}
