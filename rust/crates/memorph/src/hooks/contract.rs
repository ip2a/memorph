//! Provider-owned hook management contract.
//!
//! The common hook layer owns dispatch and shared models. Provider-specific
//! install, verification, repair, and uninstall details should live behind this
//! trait in each provider module.

use anyhow::Result;

use crate::hooks::model::{HookEvent, HookInstallStatus, HookOperationReport};
use crate::hooks::protocol::{HookIngestRequest, HookIngestResponse};
use crate::hooks::registry::HookProviderDescriptor;
use serde_json::Value;

pub trait ProviderHook: Send + Sync {
    fn provider_id(&self) -> &'static str;

    fn descriptor(&self) -> Option<HookProviderDescriptor> {
        crate::hooks::registry::find(self.provider_id())
    }

    fn status(&self) -> Result<HookInstallStatus>;
    fn install(&self) -> Result<HookOperationReport>;
    fn verify(&self) -> Result<HookOperationReport>;
    fn repair(&self) -> Result<HookOperationReport>;
    fn uninstall(&self) -> Result<HookOperationReport>;
}

/// Provider-owned hook payload normalizer.
///
/// Implementations live in `providers/<provider>/adapter.rs` and translate raw
/// provider hook payloads into memorph's canonical hook event model.
pub trait HookAdapter: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn normalize(&self, request: &HookIngestRequest) -> Result<Vec<HookEvent>>;

    /// Render provider stdout for blocking hooks.
    ///
    /// memorph observes hook events only. It does not approve, deny, ask users,
    /// or otherwise influence provider decisions, so the default response is no
    /// provider-specific output.
    fn blocking_response_json(
        &self,
        _event_name: &str,
        _response: &HookIngestResponse,
    ) -> Option<Value> {
        None
    }
}

pub fn generic_blocking_response_json(_response: &HookIngestResponse) -> Option<Value> {
    None
}

pub fn hook_specific_output_response_json(
    _event_name: &str,
    _response: &HookIngestResponse,
) -> Option<Value> {
    None
}
