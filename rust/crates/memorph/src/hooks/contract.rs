//! Provider-owned hook management contract.
//!
//! The common hook layer owns dispatch and shared models. Provider-specific
//! install, verification, repair, and uninstall details should live behind this
//! trait in each provider module.

use anyhow::Result;

use crate::hooks::model::{HookEvent, HookInstallStatus, HookOperationReport};
use crate::hooks::protocol::{HookDecision, HookIngestRequest, HookIngestResponse};
use crate::hooks::registry::HookProviderDescriptor;
use serde_json::{json, Value};

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

    /// Render the provider-compatible stdout response for blocking hooks.
    ///
    /// Most providers accept the generic `{ "decision": ... }` shape. Providers
    /// with custom hook response protocols override this in their adapter.
    fn blocking_response_json(
        &self,
        _event_name: &str,
        response: &HookIngestResponse,
    ) -> Option<Value> {
        generic_blocking_response_json(response)
    }
}

pub fn generic_blocking_response_json(response: &HookIngestResponse) -> Option<Value> {
    let decision = response.decision.as_ref()?;
    let behavior = decision_behavior(decision)?;
    let mut value = json!({"decision": behavior});
    if let Some(text) = response.response_text.as_deref() {
        value["response"] = json!(text);
    }
    Some(value)
}

pub fn hook_specific_output_response_json(
    event_name: &str,
    response: &HookIngestResponse,
) -> Option<Value> {
    let decision = response.decision.as_ref()?;
    let behavior = decision_behavior(decision)?;
    let mut value = json!({
        "hookSpecificOutput": {
            "hookEventName": event_name,
            "decision": {
                "behavior": behavior
            }
        }
    });
    if let Some(text) = response.response_text.as_deref() {
        value["hookSpecificOutput"]["response"] = json!(text);
    }
    Some(value)
}

fn decision_behavior(decision: &HookDecision) -> Option<&'static str> {
    match decision {
        HookDecision::Allow => Some("allow"),
        HookDecision::Deny => Some("deny"),
        HookDecision::AskUser => Some("ask_user"),
        HookDecision::Ignore => Some("ignore"),
        HookDecision::RecordOnly | HookDecision::ProviderDefault => None,
    }
}
