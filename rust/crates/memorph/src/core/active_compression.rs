use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalSession, EventBlock, EventRole, SessionEvent, SessionEventKind};
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
    InsufficientEstimatedSavings,
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
    let policy = params.policy;
    let original_estimated_bytes = estimate_session_bytes(session);
    let original_estimated_tokens = estimate_tokens_from_bytes(original_estimated_bytes);
    let (candidates, skipped) = plan_compression_candidates(session, &policy);
    let estimated_bytes_saved = candidates
        .iter()
        .map(|candidate| candidate.estimated_bytes_saved)
        .sum();
    let estimated_tokens_saved = candidates
        .iter()
        .map(|candidate| candidate.estimated_tokens_saved)
        .sum();
    let compressed_estimated_bytes = original_estimated_bytes.saturating_sub(estimated_bytes_saved);
    let compressed_estimated_tokens =
        original_estimated_tokens.saturating_sub(estimated_tokens_saved);
    ActiveCompressionReport {
        source_provider_id: params.source_provider_id,
        target_provider_id: params.target_provider_id,
        dry_run: true,
        policy,
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
        compressed_estimated_bytes,
        compressed_estimated_tokens,
        estimated_bytes_saved,
        estimated_tokens_saved,
        candidates,
        skipped,
        archive_refs: Vec::new(),
    }
}

pub fn plan_compression_candidates(
    session: &CanonicalSession,
    policy: &ActiveCompressionPolicy,
) -> (Vec<CompressionCandidateReport>, Vec<CompressionSkipReport>) {
    let protected_message_indexes = protected_recent_message_indexes(session, policy);
    let mut candidates = Vec::new();
    let mut skipped = Vec::new();

    for (event_index, event) in session.events.iter().enumerate() {
        let estimated_bytes = estimate_event_bytes(event);
        let estimated_tokens = estimate_tokens_from_bytes(estimated_bytes);

        if matches!(event.role, EventRole::System | EventRole::Developer) {
            skipped.push(skip_report(
                event,
                event_index,
                CompressionSkipReason::SystemOrDeveloperInstruction,
                estimated_bytes,
                estimated_tokens,
            ));
            continue;
        }

        if event
            .blocks
            .iter()
            .any(|block| matches!(block, EventBlock::Compressed { .. }))
        {
            skipped.push(skip_report(
                event,
                event_index,
                CompressionSkipReason::AlreadyCompressed,
                estimated_bytes,
                estimated_tokens,
            ));
            continue;
        }

        if protected_message_indexes.contains(&event_index) {
            skipped.push(skip_report(
                event,
                event_index,
                CompressionSkipReason::ProtectedRecentMessage,
                estimated_bytes,
                estimated_tokens,
            ));
            continue;
        }

        if estimated_bytes < policy.min_candidate_bytes {
            skipped.push(skip_report(
                event,
                event_index,
                CompressionSkipReason::BelowByteThreshold,
                estimated_bytes,
                estimated_tokens,
            ));
            continue;
        }

        let Some((kind, reason, risk)) = classify_candidate(event) else {
            skipped.push(skip_report(
                event,
                event_index,
                CompressionSkipReason::UnsupportedEventShape,
                estimated_bytes,
                estimated_tokens,
            ));
            continue;
        };

        let compressed_estimated_bytes = estimate_candidate_compressed_bytes(estimated_bytes, kind);
        let compressed_estimated_tokens = estimate_tokens_from_bytes(compressed_estimated_bytes);
        let estimated_bytes_saved = estimated_bytes.saturating_sub(compressed_estimated_bytes);
        let estimated_tokens_saved = estimated_tokens.saturating_sub(compressed_estimated_tokens);
        if estimated_bytes_saved.saturating_mul(100)
            < estimated_bytes.saturating_mul(policy.min_savings_ratio_percent as usize)
        {
            skipped.push(skip_report(
                event,
                event_index,
                CompressionSkipReason::InsufficientEstimatedSavings,
                estimated_bytes,
                estimated_tokens,
            ));
            continue;
        }
        candidates.push(CompressionCandidateReport {
            id: format!("candidate-{:04}", candidates.len() + 1),
            kind,
            event_ids: vec![event.id.clone()],
            start_event_index: event_index,
            end_event_index: event_index,
            reason,
            risk,
            original_estimated_bytes: estimated_bytes,
            original_estimated_tokens: estimated_tokens,
            compressed_estimated_bytes,
            compressed_estimated_tokens,
            estimated_bytes_saved,
            estimated_tokens_saved,
            archive_refs: Vec::new(),
        });
    }

    (candidates, skipped)
}

pub fn estimate_session_bytes(session: &CanonicalSession) -> usize {
    session.events.iter().map(estimate_event_bytes).sum()
}

pub fn estimate_tokens_from_bytes(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

fn protected_recent_message_indexes(
    session: &CanonicalSession,
    policy: &ActiveCompressionPolicy,
) -> Vec<usize> {
    session
        .events
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, event)| event.kind == SessionEventKind::Message)
        .take(policy.protect_recent_message_events)
        .map(|(idx, _)| idx)
        .collect()
}

fn estimate_event_bytes(event: &SessionEvent) -> usize {
    canonical_event_text(event).len()
}

fn classify_candidate(
    event: &SessionEvent,
) -> Option<(
    CompressionCandidateKind,
    CompressionSelectionReason,
    CompressionRisk,
)> {
    if event
        .blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolResult { .. }))
    {
        return Some((
            CompressionCandidateKind::LargeToolOutput,
            CompressionSelectionReason::LargeToolOutput,
            CompressionRisk::Low,
        ));
    }

    if event
        .blocks
        .iter()
        .any(|block| matches!(block, EventBlock::CommandResult { .. }))
    {
        return Some((
            CompressionCandidateKind::LargeLogOutput,
            CompressionSelectionReason::LargeCommandOutput,
            CompressionRisk::Low,
        ));
    }

    let text = canonical_event_text(event);
    if looks_like_search_results(&text) {
        return Some((
            CompressionCandidateKind::SearchResults,
            CompressionSelectionReason::LargeSearchResult,
            CompressionRisk::Low,
        ));
    }

    if event
        .blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ProviderPayload { .. }))
    {
        return Some((
            CompressionCandidateKind::ProviderPayloadText,
            CompressionSelectionReason::HistoricalContext,
            CompressionRisk::High,
        ));
    }

    if event.kind == SessionEventKind::Message
        && matches!(
            event.role,
            EventRole::User | EventRole::Assistant | EventRole::Tool
        )
        && !text.trim().is_empty()
    {
        return Some((
            CompressionCandidateKind::HistoricalConversationRange,
            CompressionSelectionReason::HistoricalContext,
            CompressionRisk::Medium,
        ));
    }

    None
}

fn looks_like_search_results(text: &str) -> bool {
    let matching_lines = text
        .lines()
        .filter(|line| {
            let mut parts = line.splitn(3, ':');
            let Some(path) = parts.next() else {
                return false;
            };
            let Some(line_number) = parts.next() else {
                return false;
            };
            parts.next().is_some()
                && (path.contains('/') || path.contains('.'))
                && line_number.parse::<usize>().is_ok()
        })
        .take(3)
        .count();
    matching_lines >= 2
}

fn estimate_candidate_compressed_bytes(
    original_bytes: usize,
    kind: CompressionCandidateKind,
) -> usize {
    let ratio = match kind {
        CompressionCandidateKind::LargeToolOutput => 20,
        CompressionCandidateKind::LargeLogOutput => 20,
        CompressionCandidateKind::SearchResults => 30,
        CompressionCandidateKind::HistoricalConversationRange => 35,
        CompressionCandidateKind::ProviderPayloadText => 50,
    };
    let estimated = original_bytes.saturating_mul(ratio) / 100;
    estimated.max(128).min(original_bytes)
}

fn skip_report(
    event: &SessionEvent,
    event_index: usize,
    reason: CompressionSkipReason,
    estimated_bytes: usize,
    estimated_tokens: usize,
) -> CompressionSkipReport {
    CompressionSkipReport {
        event_id: event.id.clone(),
        event_index,
        reason,
        estimated_bytes,
        estimated_tokens,
    }
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

    #[test]
    fn planner_selects_large_candidates_and_reports_skips() {
        let session = planner_sample_session();
        let policy = ActiveCompressionPolicy {
            protect_recent_message_events: 1,
            min_candidate_bytes: 16,
            min_savings_ratio_percent: 20,
            mode: ActiveCompressionMode::PlanOnly,
        };
        let report = build_dry_run_report(
            &session,
            ActiveCompressionParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy,
                dry_run: true,
            },
        );

        let candidate_kinds = report
            .candidates
            .iter()
            .map(|candidate| candidate.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            candidate_kinds,
            vec![
                CompressionCandidateKind::HistoricalConversationRange,
                CompressionCandidateKind::LargeToolOutput,
                CompressionCandidateKind::LargeLogOutput,
                CompressionCandidateKind::SearchResults,
            ]
        );
        assert_eq!(report.candidates[0].id, "candidate-0001");
        assert_eq!(report.candidates[0].event_ids, vec!["old-user"]);
        assert!(report.estimated_bytes_saved > 0);
        assert!(report.compressed_estimated_bytes < report.original_estimated_bytes);

        assert_skip(
            &report,
            "system",
            CompressionSkipReason::SystemOrDeveloperInstruction,
        );
        assert_skip(
            &report,
            "compressed",
            CompressionSkipReason::AlreadyCompressed,
        );
        assert_skip(&report, "small", CompressionSkipReason::BelowByteThreshold);
        assert_skip(
            &report,
            "recent",
            CompressionSkipReason::ProtectedRecentMessage,
        );
    }

    #[test]
    fn planner_rejects_candidates_below_required_savings_ratio() {
        let mut session = sample_session();
        session.events = vec![text_event("old-user", EventRole::User, &"x".repeat(200))];
        let report = build_dry_run_report(
            &session,
            ActiveCompressionParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 0,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 90,
                    mode: ActiveCompressionMode::PlanOnly,
                },
                dry_run: true,
            },
        );

        assert!(report.candidates.is_empty());
        assert_skip(
            &report,
            "old-user",
            CompressionSkipReason::InsufficientEstimatedSavings,
        );
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

    fn planner_sample_session() -> CanonicalSession {
        let mut session = sample_session();
        session.events = vec![
            text_event(
                "system",
                EventRole::System,
                &"system instruction ".repeat(4),
            ),
            text_event(
                "old-user",
                EventRole::User,
                &"historical context ".repeat(40),
            ),
            tool_result_event("tool-output", &"tool output line\n".repeat(12)),
            command_result_event("command-output", &"compiler warning\n".repeat(12)),
            text_event(
                "search",
                EventRole::Tool,
                &[
                    "src/lib.rs:10:match one repeated context",
                    "src/core.rs:22:match two repeated context",
                    "src/api.rs:33:match three repeated context",
                ]
                .join("\n")
                .repeat(8),
            ),
            compressed_event("compressed"),
            text_event("small", EventRole::Assistant, "tiny"),
            text_event(
                "recent",
                EventRole::User,
                &"latest active request ".repeat(8),
            ),
        ];
        session
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

    fn tool_result_event(id: &str, content: &str) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::ToolResult,
            role: EventRole::Tool,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::ToolResult {
                tool_call_id: "tool-1".to_string(),
                content: content.to_string(),
                is_error: false,
            }],
            metadata: event_metadata("claude", id),
        }
    }

    fn command_result_event(id: &str, stdout: &str) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::CommandResult,
            role: EventRole::Tool,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::CommandResult {
                command: Some("cargo check".to_string()),
                exit_code: Some(0),
                stdout: Some(stdout.to_string()),
                stderr: None,
            }],
            metadata: event_metadata("claude", id),
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

    fn assert_skip(
        report: &ActiveCompressionReport,
        event_id: &str,
        reason: CompressionSkipReason,
    ) {
        assert!(
            report
                .skipped
                .iter()
                .any(|skip| skip.event_id == event_id && skip.reason == reason),
            "missing skip reason {:?} for {} in {:?}",
            reason,
            event_id,
            report.skipped
        );
    }
}
