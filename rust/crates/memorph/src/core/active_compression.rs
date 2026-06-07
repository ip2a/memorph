use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalSession, EventBlock, SessionEventKind};
use crate::provider::canonical_event_text;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveCompressionParams {
    pub source_provider_id: String,
    pub target_provider_id: String,
    #[serde(default)]
    pub policy: ActiveCompressionPolicy,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveCompressionPolicy {
    #[serde(default = "default_recent_message_protection")]
    pub protect_recent_message_events: usize,
    #[serde(default = "default_min_candidate_bytes")]
    pub min_candidate_bytes: usize,
    #[serde(default = "default_min_savings_ratio")]
    pub min_savings_ratio_percent: u8,
    #[serde(default)]
    pub mode: ActiveCompressionMode,
}

impl Default for ActiveCompressionPolicy {
    fn default() -> Self {
        Self {
            protect_recent_message_events: default_recent_message_protection(),
            min_candidate_bytes: default_min_candidate_bytes(),
            min_savings_ratio_percent: default_min_savings_ratio(),
            mode: ActiveCompressionMode::PlanOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActiveCompressionMode {
    PlanOnly,
    Auto,
    Manual,
}

impl Default for ActiveCompressionMode {
    fn default() -> Self {
        Self::PlanOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveCompressionReport {
    pub source_provider_id: String,
    pub target_provider_id: String,
    pub dry_run: bool,
    pub policy: ActiveCompressionPolicy,
    pub session_event_count: usize,
    pub message_event_count: usize,
    pub already_compressed_event_count: usize,
    pub original_estimated_bytes: usize,
    pub original_estimated_tokens: usize,
    pub compressed_estimated_bytes: usize,
    pub compressed_estimated_tokens: usize,
    pub estimated_bytes_saved: usize,
    pub estimated_tokens_saved: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<CompressionCandidateReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<CompressionSkipReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressionCandidateReport {
    pub id: String,
    pub kind: CompressionCandidateKind,
    pub event_ids: Vec<String>,
    pub start_event_index: usize,
    pub end_event_index: usize,
    pub reason: CompressionSelectionReason,
    pub risk: CompressionRisk,
    pub original_estimated_bytes: usize,
    pub original_estimated_tokens: usize,
    pub compressed_estimated_bytes: usize,
    pub compressed_estimated_tokens: usize,
    pub estimated_bytes_saved: usize,
    pub estimated_tokens_saved: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompressionSkipReport {
    pub event_id: String,
    pub event_index: usize,
    pub reason: CompressionSkipReason,
    pub estimated_bytes: usize,
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionCandidateKind {
    HistoricalConversationRange,
    LargeToolOutput,
    LargeLogOutput,
    SearchResults,
    ProviderPayloadText,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionSelectionReason {
    HistoricalContext,
    LargeToolOutput,
    LargeCommandOutput,
    LargeSearchResult,
    ManualSelection,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionSkipReason {
    ProtectedRecentMessage,
    SystemOrDeveloperInstruction,
    AlreadyCompressed,
    BelowByteThreshold,
    UnsupportedEventShape,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionRisk {
    Low,
    Medium,
    High,
}

/// Phase 2 only builds a read-only dry-run report. Candidate selection,
/// archive writing, and session mutation are introduced in later phases.
pub fn build_dry_run_report(
    session: &CanonicalSession,
    params: ActiveCompressionParams,
) -> ActiveCompressionReport {
    let original_estimated_bytes = estimate_session_bytes(session);
    let original_estimated_tokens = estimate_tokens_from_bytes(original_estimated_bytes);
    ActiveCompressionReport {
        source_provider_id: params.source_provider_id,
        target_provider_id: params.target_provider_id,
        dry_run: true,
        policy: params.policy,
        session_event_count: session.events.len(),
        message_event_count: session
            .events
            .iter()
            .filter(|event| event.kind == SessionEventKind::Message)
            .count(),
        already_compressed_event_count: session
            .events
            .iter()
            .filter(|event| {
                event
                    .blocks
                    .iter()
                    .any(|block| matches!(block, EventBlock::Compressed { .. }))
            })
            .count(),
        original_estimated_bytes,
        original_estimated_tokens,
        compressed_estimated_bytes: original_estimated_bytes,
        compressed_estimated_tokens: original_estimated_tokens,
        estimated_bytes_saved: 0,
        estimated_tokens_saved: 0,
        candidates: Vec::new(),
        skipped: Vec::new(),
        archive_refs: Vec::new(),
    }
}

pub fn estimate_session_bytes(session: &CanonicalSession) -> usize {
    session
        .events
        .iter()
        .map(|event| canonical_event_text(event).len())
        .sum()
}

pub fn estimate_tokens_from_bytes(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

fn default_recent_message_protection() -> usize {
    6
}

fn default_min_candidate_bytes() -> usize {
    4 * 1024
}

fn default_min_savings_ratio() -> u8 {
    20
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{
        CanonicalSchema, EventBlock, EventLinks, EventMetadata, EventRole, EventSource,
        MappingDisposition, ProviderSessionRef, SessionContext, SessionEvent, SessionIdentity,
        SessionProvenance,
    };
    use chrono::Utc;
    use std::collections::BTreeMap;

    #[test]
    fn dry_run_report_is_constructible_without_mutating_session() {
        let session = sample_session();
        let original_event_ids = session
            .events
            .iter()
            .map(|event| event.id.clone())
            .collect::<Vec<_>>();
        let report = build_dry_run_report(
            &session,
            ActiveCompressionParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy::default(),
                dry_run: true,
            },
        );

        assert!(report.dry_run);
        assert_eq!(report.source_provider_id, "claude");
        assert_eq!(report.target_provider_id, "codex");
        assert_eq!(report.session_event_count, 3);
        assert_eq!(report.message_event_count, 3);
        assert_eq!(report.already_compressed_event_count, 1);
        assert!(report.original_estimated_bytes > 0);
        assert_eq!(
            report.original_estimated_tokens,
            estimate_tokens_from_bytes(report.original_estimated_bytes)
        );
        assert_eq!(
            report.compressed_estimated_bytes,
            report.original_estimated_bytes
        );
        assert_eq!(
            session
                .events
                .iter()
                .map(|event| event.id.clone())
                .collect::<Vec<_>>(),
            original_event_ids
        );
    }

    #[test]
    fn dry_run_report_serializes_with_stable_policy_fields() {
        let report = build_dry_run_report(
            &sample_session(),
            ActiveCompressionParams {
                source_provider_id: "kimi".to_string(),
                target_provider_id: "deepseek".to_string(),
                policy: ActiveCompressionPolicy::default(),
                dry_run: true,
            },
        );

        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["policy"]["protect_recent_message_events"], 6);
        assert_eq!(value["policy"]["min_candidate_bytes"], 4096);
        assert_eq!(value["policy"]["min_savings_ratio_percent"], 20);
        assert_eq!(value["policy"]["mode"], "plan_only");
    }

    fn sample_session() -> CanonicalSession {
        let now = Utc::now();
        CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "active-compression-sample".to_string(),
                source_title: Some("Active compression sample".to_string()),
            },
            provenance: SessionProvenance {
                imported_at: now,
                imported_by: Some("test".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: "claude".to_string(),
                    session_id: "s1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext::default(),
            events: vec![
                text_event("u1", EventRole::User, "original task"),
                text_event("a1", EventRole::Assistant, "implementation notes"),
                compressed_event("c1"),
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        }
    }

    fn text_event(id: &str, role: EventRole, text: &str) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Message,
            role,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: text.to_string(),
            }],
            metadata: event_metadata("claude", id),
        }
    }

    fn compressed_event(id: &str) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Compressed {
                source_provider_id: "claude".to_string(),
                summary: "compressed summary".to_string(),
                source_event_ids: vec!["u0".to_string()],
                source_event_count: Some(1),
                archive_ref: Some("memorph-archive://s1/a.json.gz".to_string()),
            }],
            metadata: event_metadata("memorph", id),
        }
    }

    fn event_metadata(provider_id: &str, original_id: &str) -> EventMetadata {
        EventMetadata {
            source: EventSource {
                provider_id: provider_id.to_string(),
                original_id: Some(original_id.to_string()),
                original_role: None,
                phase: None,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: BTreeMap::new(),
        }
    }
}
