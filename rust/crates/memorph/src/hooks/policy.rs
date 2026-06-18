//! Permission and safety policy engine.
//!
//! Policy decides whether blocking hook events should be recorded only,
//! allowed, denied, ignored, or escalated. The default is deliberately
//! conservative: record the event and let the provider's native UI handle it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::hooks::model::{HookEvent, HookEventType};
use crate::hooks::protocol::HookDecision;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookPolicy {
    #[serde(default = "current_version")]
    pub version: u32,
    #[serde(default)]
    pub global: HookPolicyMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, HookPolicyMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_rules: Vec<HookToolPolicyRule>,
}

impl Default for HookPolicy {
    fn default() -> Self {
        Self {
            version: current_version(),
            global: HookPolicyMode::RecordOnly,
            providers: BTreeMap::new(),
            tool_rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookPolicyMode {
    RecordOnly,
    Allow,
    Deny,
    AskUser,
    Ignore,
    ProviderDefault,
}

impl Default for HookPolicyMode {
    fn default() -> Self {
        Self::RecordOnly
    }
}

impl From<HookPolicyMode> for HookDecision {
    fn from(value: HookPolicyMode) -> Self {
        match value {
            HookPolicyMode::RecordOnly => HookDecision::RecordOnly,
            HookPolicyMode::Allow => HookDecision::Allow,
            HookPolicyMode::Deny => HookDecision::Deny,
            HookPolicyMode::AskUser => HookDecision::AskUser,
            HookPolicyMode::Ignore => HookDecision::Ignore,
            HookPolicyMode::ProviderDefault => HookDecision::ProviderDefault,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookToolPolicyRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<HookEventType>,
    pub mode: HookPolicyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookPolicyEvaluation {
    pub decision: HookDecision,
    pub source: HookPolicySource,
    pub mode: HookPolicyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookPolicySource {
    NotBlocking,
    ToolRule,
    ProviderOverride,
    GlobalDefault,
}

fn current_version() -> u32 {
    1
}

pub fn evaluate(
    policy: &HookPolicy,
    event: &HookEvent,
    request_blocking: bool,
) -> HookPolicyEvaluation {
    if !request_blocking && !event.event_type.is_blocking() {
        return HookPolicyEvaluation {
            decision: HookDecision::RecordOnly,
            source: HookPolicySource::NotBlocking,
            mode: HookPolicyMode::RecordOnly,
        };
    }

    if let Some(rule) = policy.tool_rules.iter().find(|rule| rule.matches(event)) {
        return HookPolicyEvaluation {
            decision: rule.mode.into(),
            source: HookPolicySource::ToolRule,
            mode: rule.mode,
        };
    }

    if let Some(mode) = policy.providers.get(&event.provider) {
        return HookPolicyEvaluation {
            decision: (*mode).into(),
            source: HookPolicySource::ProviderOverride,
            mode: *mode,
        };
    }

    HookPolicyEvaluation {
        decision: policy.global.into(),
        source: HookPolicySource::GlobalDefault,
        mode: policy.global,
    }
}

pub fn effective_decision(
    policy: &HookPolicy,
    events: &[HookEvent],
    request_blocking: bool,
) -> Option<HookPolicyEvaluation> {
    if !request_blocking && !events.iter().any(|event| event.event_type.is_blocking()) {
        return None;
    }
    events
        .iter()
        .find(|event| request_blocking || event.event_type.is_blocking())
        .map(|event| evaluate(policy, event, request_blocking))
}

impl HookToolPolicyRule {
    fn matches(&self, event: &HookEvent) -> bool {
        if self
            .provider
            .as_deref()
            .is_some_and(|provider| provider != event.provider)
        {
            return false;
        }
        if self
            .event_type
            .as_ref()
            .is_some_and(|event_type| event_type != &event.event_type)
        {
            return false;
        }
        if let Some(expected_tool) = self.tool_name.as_deref() {
            let actual = event
                .tool
                .as_ref()
                .map(|tool| tool.name.as_str())
                .or_else(|| {
                    event
                        .permission
                        .as_ref()
                        .and_then(|permission| permission.tool.as_ref())
                        .map(|tool| tool.name.as_str())
                });
            if actual.map(normalize_name) != Some(normalize_name(expected_tool)) {
                return false;
            }
        }
        true
    }
}

fn normalize_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{HookEvent, HookToolCall};
    use serde_json::{json, Value};

    #[test]
    fn default_policy_records_blocking_events_only() {
        let event = HookEvent::new("claude", HookEventType::PermissionRequested, Value::Null);
        let eval = evaluate(&HookPolicy::default(), &event, true);
        assert_eq!(eval.decision, HookDecision::RecordOnly);
        assert_eq!(eval.source, HookPolicySource::GlobalDefault);
    }

    #[test]
    fn tool_rule_overrides_provider_and_global() {
        let mut policy = HookPolicy {
            global: HookPolicyMode::RecordOnly,
            providers: BTreeMap::from([("claude".to_string(), HookPolicyMode::Allow)]),
            ..HookPolicy::default()
        };
        policy.tool_rules.push(HookToolPolicyRule {
            provider: Some("claude".to_string()),
            tool_name: Some("Bash".to_string()),
            event_type: Some(HookEventType::PermissionRequested),
            mode: HookPolicyMode::Deny,
        });
        let mut event = HookEvent::new("claude", HookEventType::PermissionRequested, Value::Null);
        event.tool = Some(HookToolCall {
            id: None,
            name: "bash".to_string(),
            input: json!({"command": "rm -rf target"}),
        });
        let eval = evaluate(&policy, &event, true);
        assert_eq!(eval.decision, HookDecision::Deny);
        assert_eq!(eval.source, HookPolicySource::ToolRule);
    }
}
