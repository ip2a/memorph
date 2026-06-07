use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::canonical::{
    CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole, EventSource,
    MappingDisposition, SessionEvent, SessionEventKind,
};
use crate::core::compression;
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
pub struct ActiveCompressionApplyParams {
    pub source_provider_id: String,
    pub target_provider_id: String,
    #[serde(default)]
    pub policy: ActiveCompressionPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCompressionApplyResult {
    pub session: CanonicalSession,
    pub report: ActiveCompressionReport,
}

pub trait ActiveCompressionSummarizer {
    fn summarize(&self, request: &CompressionSummaryRequest<'_>)
        -> Result<CompressionSummaryDraft>;
}

#[derive(Debug, Clone)]
pub struct CompressionSummaryRequest<'a> {
    pub candidate: &'a CompressionCandidateReport,
    pub source_events: &'a [SessionEvent],
    pub archive_ref: Option<&'a str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompressionSummaryDraft {
    pub summary: String,
    pub goals: Vec<String>,
    pub decisions: Vec<String>,
    pub completed_work: Vec<String>,
    pub open_questions: Vec<String>,
    pub files: Vec<String>,
    pub risks: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionSummarySource {
    Deterministic,
    External,
    DeterministicFallback,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionSummaryRejectionReason {
    Empty,
    TooLarge,
    SummarizerError,
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
    #[serde(default)]
    pub token_estimator: CompressionTokenEstimatorReport,
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
pub struct CompressionTokenEstimatorReport {
    pub strategy: CompressionTokenEstimatorStrategy,
    pub source_provider_chars_per_token_x100: usize,
    pub target_provider_chars_per_token_x100: usize,
    pub effective_provider_id: String,
    pub effective_chars_per_token_x100: usize,
}

impl Default for CompressionTokenEstimatorReport {
    fn default() -> Self {
        Self {
            strategy: CompressionTokenEstimatorStrategy::ProviderHeuristic,
            source_provider_chars_per_token_x100: default_chars_per_token_x100(),
            target_provider_chars_per_token_x100: default_chars_per_token_x100(),
            effective_provider_id: "unknown".to_string(),
            effective_chars_per_token_x100: default_chars_per_token_x100(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionTokenEstimatorStrategy {
    ProviderHeuristic,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_source: Option<CompressionSummarySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_rejection_reason: Option<CompressionSummaryRejectionReason>,
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

/// Build a read-only report for active compression planning.
pub fn build_dry_run_report(
    session: &CanonicalSession,
    params: ActiveCompressionParams,
) -> ActiveCompressionReport {
    let policy = params.policy;
    let token_estimator =
        build_token_estimator(&params.source_provider_id, &params.target_provider_id);
    let original_estimated_bytes = estimate_session_bytes(session);
    let original_estimated_tokens =
        estimate_tokens_from_bytes_with_estimator(original_estimated_bytes, &token_estimator);
    let (candidates, skipped) =
        plan_compression_candidates_with_estimator(session, &policy, &token_estimator);
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
        token_estimator,
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
    plan_compression_candidates_with_estimator(
        session,
        policy,
        &CompressionTokenEstimatorReport::default(),
    )
}

fn plan_compression_candidates_with_estimator(
    session: &CanonicalSession,
    policy: &ActiveCompressionPolicy,
    token_estimator: &CompressionTokenEstimatorReport,
) -> (Vec<CompressionCandidateReport>, Vec<CompressionSkipReport>) {
    let protected_message_indexes = protected_recent_message_indexes(session, policy);
    let mut candidates = Vec::new();
    let mut skipped = Vec::new();

    for (event_index, event) in session.events.iter().enumerate() {
        let estimated_bytes = estimate_event_bytes(event);
        let estimated_tokens =
            estimate_tokens_from_bytes_with_estimator(estimated_bytes, token_estimator);

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
        let compressed_estimated_tokens =
            estimate_tokens_from_bytes_with_estimator(compressed_estimated_bytes, token_estimator);
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
            summary_source: None,
            summary_rejection_reason: None,
        });
    }

    (candidates, skipped)
}

pub fn apply_active_compression(
    session: &CanonicalSession,
    params: ActiveCompressionApplyParams,
) -> Result<ActiveCompressionApplyResult> {
    let archive_dir = crate::config::memorph_dir()?.join("compression_archives");
    apply_active_compression_with_archive_dir(session, params, &archive_dir)
}

pub(crate) fn apply_active_compression_with_archive_dir(
    session: &CanonicalSession,
    params: ActiveCompressionApplyParams,
    archive_dir: &Path,
) -> Result<ActiveCompressionApplyResult> {
    apply_active_compression_with_archive_dir_and_summarizer(session, params, archive_dir, None)
}

pub(crate) fn apply_active_compression_with_archive_dir_and_summarizer(
    session: &CanonicalSession,
    params: ActiveCompressionApplyParams,
    archive_dir: &Path,
    summarizer: Option<&dyn ActiveCompressionSummarizer>,
) -> Result<ActiveCompressionApplyResult> {
    let mut report = build_dry_run_report(
        session,
        ActiveCompressionParams {
            source_provider_id: params.source_provider_id.clone(),
            target_provider_id: params.target_provider_id.clone(),
            policy: params.policy,
            dry_run: false,
        },
    );
    report.dry_run = false;

    let requested_ids = params.candidate_ids.iter().cloned().collect::<HashSet<_>>();
    let selected = report
        .candidates
        .iter()
        .filter(|candidate| requested_ids.is_empty() || requested_ids.contains(&candidate.id))
        .cloned()
        .collect::<Vec<_>>();

    report.candidates = selected.clone();
    recompute_report_estimates(&mut report);

    if selected.is_empty() {
        return Ok(ActiveCompressionApplyResult {
            session: session.clone(),
            report,
        });
    }

    let mut archive_refs = Vec::new();
    let mut summary_outcomes = Vec::new();
    let mut replacement_events = Vec::with_capacity(session.events.len());
    let mut event_index = 0usize;
    while event_index < session.events.len() {
        let Some(candidate) = selected
            .iter()
            .find(|candidate| candidate.start_event_index == event_index)
        else {
            replacement_events.push(session.events[event_index].clone());
            event_index += 1;
            continue;
        };

        let source_events =
            session.events[candidate.start_event_index..=candidate.end_event_index].to_vec();
        let source_event_ids = source_events
            .iter()
            .map(|event| event.id.clone())
            .collect::<Vec<_>>();
        let summary_seed = build_deterministic_summary(candidate, &source_events, None);
        let summary_event = active_summary_event(
            candidate,
            &source_events,
            &params.source_provider_id,
            summary_seed,
            None,
        );
        let archive_ref = compression::write_active_compression_archive_in_dir(
            archive_dir,
            session,
            &params.source_provider_id,
            &params.target_provider_id,
            &summary_event,
            source_event_ids,
            source_events.clone(),
        )?;
        let summary_outcome =
            build_summary(candidate, &source_events, Some(&archive_ref), summarizer);
        replacement_events.push(active_summary_event(
            candidate,
            &source_events,
            &params.source_provider_id,
            summary_outcome.summary.clone(),
            Some(archive_ref.clone()),
        ));
        summary_outcomes.push((candidate.id.clone(), summary_outcome));
        archive_refs.push(archive_ref);
        event_index = candidate.end_event_index + 1;
    }

    let mut next = session.clone();
    next.events = replacement_events;
    for candidate in &mut report.candidates {
        if let Some(archive_ref) = archive_refs
            .iter()
            .find(|archive_ref| archive_ref.contains(&candidate.id.replace('-', "_")))
        {
            candidate.archive_refs = vec![archive_ref.clone()];
        }
    }
    if report
        .candidates
        .iter()
        .any(|candidate| candidate.archive_refs.is_empty())
    {
        for (candidate, archive_ref) in report.candidates.iter_mut().zip(archive_refs.iter()) {
            candidate.archive_refs = vec![archive_ref.clone()];
        }
    }
    for candidate in &mut report.candidates {
        if let Some((_, outcome)) = summary_outcomes
            .iter()
            .find(|(candidate_id, _)| candidate_id == &candidate.id)
        {
            candidate.summary_source = Some(outcome.source);
            candidate.summary_rejection_reason = outcome.rejection_reason;
        }
    }
    report.archive_refs = archive_refs;

    Ok(ActiveCompressionApplyResult {
        session: next,
        report,
    })
}

pub fn estimate_session_bytes(session: &CanonicalSession) -> usize {
    session.events.iter().map(estimate_event_bytes).sum()
}

pub fn estimate_tokens_from_bytes(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

fn estimate_tokens_from_bytes_with_estimator(
    bytes: usize,
    estimator: &CompressionTokenEstimatorReport,
) -> usize {
    let chars_per_token_x100 = estimator.effective_chars_per_token_x100.max(1);
    bytes.saturating_mul(100).div_ceil(chars_per_token_x100)
}

fn build_token_estimator(
    source_provider_id: &str,
    target_provider_id: &str,
) -> CompressionTokenEstimatorReport {
    let source_provider_chars_per_token_x100 = provider_chars_per_token_x100(source_provider_id);
    let target_provider_chars_per_token_x100 = provider_chars_per_token_x100(target_provider_id);
    CompressionTokenEstimatorReport {
        strategy: CompressionTokenEstimatorStrategy::ProviderHeuristic,
        source_provider_chars_per_token_x100,
        target_provider_chars_per_token_x100,
        effective_provider_id: target_provider_id.to_string(),
        effective_chars_per_token_x100: target_provider_chars_per_token_x100,
    }
}

fn provider_chars_per_token_x100(provider_id: &str) -> usize {
    match provider_id.trim().to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => 350,
        "codex" | "openai" | "opencode" => 400,
        "gemini" | "google" => 360,
        "kimi" => 320,
        "deepseek" => 330,
        _ => default_chars_per_token_x100(),
    }
}

fn default_chars_per_token_x100() -> usize {
    400
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

fn recompute_report_estimates(report: &mut ActiveCompressionReport) {
    report.estimated_bytes_saved = report
        .candidates
        .iter()
        .map(|candidate| candidate.estimated_bytes_saved)
        .sum();
    report.estimated_tokens_saved = report
        .candidates
        .iter()
        .map(|candidate| candidate.estimated_tokens_saved)
        .sum();
    report.compressed_estimated_bytes = report
        .original_estimated_bytes
        .saturating_sub(report.estimated_bytes_saved);
    report.compressed_estimated_tokens = report
        .original_estimated_tokens
        .saturating_sub(report.estimated_tokens_saved);
}

fn active_summary_event(
    candidate: &CompressionCandidateReport,
    source_events: &[SessionEvent],
    source_provider_id: &str,
    summary: String,
    archive_ref: Option<String>,
) -> SessionEvent {
    let source_event_ids = source_events
        .iter()
        .map(|event| event.id.clone())
        .collect::<Vec<_>>();
    let source_event_count = source_event_ids.len();
    let timestamp = source_events
        .last()
        .map(|event| event.timestamp)
        .unwrap_or_else(chrono::Utc::now);
    let mut provider_ext = BTreeMap::new();
    provider_ext.insert(
        "memorph_compression".to_string(),
        serde_json::json!({
            "active": true,
            "candidate_id": candidate.id,
            "candidate_kind": candidate.kind,
            "selection_reason": candidate.reason,
            "source_event_count": source_event_count,
            "archive_ref": archive_ref.clone(),
        }),
    );

    SessionEvent {
        id: format!("memorph-active-compressed-{}", candidate.id),
        kind: SessionEventKind::Message,
        role: EventRole::Assistant,
        timestamp,
        links: EventLinks::default(),
        blocks: vec![EventBlock::Compressed {
            source_provider_id: source_provider_id.to_string(),
            summary,
            source_event_ids,
            source_event_count: Some(source_event_count),
            archive_ref,
        }],
        metadata: EventMetadata {
            source: EventSource {
                provider_id: "memorph".to_string(),
                original_id: Some(candidate.id.clone()),
                original_role: Some("assistant".to_string()),
                phase: Some("active-compression".to_string()),
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Normalized,
            provider_ext,
        },
    }
}

#[derive(Debug, Clone)]
struct CompressionSummaryOutcome {
    summary: String,
    source: CompressionSummarySource,
    rejection_reason: Option<CompressionSummaryRejectionReason>,
}

fn build_summary(
    candidate: &CompressionCandidateReport,
    source_events: &[SessionEvent],
    archive_ref: Option<&str>,
    summarizer: Option<&dyn ActiveCompressionSummarizer>,
) -> CompressionSummaryOutcome {
    let Some(summarizer) = summarizer else {
        return CompressionSummaryOutcome {
            summary: build_deterministic_summary(candidate, source_events, archive_ref),
            source: CompressionSummarySource::Deterministic,
            rejection_reason: None,
        };
    };

    let request = CompressionSummaryRequest {
        candidate,
        source_events,
        archive_ref,
    };
    let draft = match summarizer.summarize(&request) {
        Ok(draft) => draft,
        Err(_) => {
            return fallback_summary(
                candidate,
                source_events,
                archive_ref,
                CompressionSummaryRejectionReason::SummarizerError,
            );
        }
    };
    let summary = external_summary_text(draft, archive_ref);
    match validate_external_summary(candidate, &summary) {
        Ok(()) => CompressionSummaryOutcome {
            summary,
            source: CompressionSummarySource::External,
            rejection_reason: None,
        },
        Err(reason) => fallback_summary(candidate, source_events, archive_ref, reason),
    }
}

fn fallback_summary(
    candidate: &CompressionCandidateReport,
    source_events: &[SessionEvent],
    archive_ref: Option<&str>,
    reason: CompressionSummaryRejectionReason,
) -> CompressionSummaryOutcome {
    CompressionSummaryOutcome {
        summary: build_deterministic_summary(candidate, source_events, archive_ref),
        source: CompressionSummarySource::DeterministicFallback,
        rejection_reason: Some(reason),
    }
}

fn validate_external_summary(
    candidate: &CompressionCandidateReport,
    summary: &str,
) -> std::result::Result<(), CompressionSummaryRejectionReason> {
    if summary.trim().is_empty() {
        return Err(CompressionSummaryRejectionReason::Empty);
    }
    let summary_bytes = summary.len();
    if summary_bytes >= candidate.original_estimated_bytes
        || summary_bytes > candidate.compressed_estimated_bytes
    {
        return Err(CompressionSummaryRejectionReason::TooLarge);
    }
    Ok(())
}

fn external_summary_text(draft: CompressionSummaryDraft, archive_ref: Option<&str>) -> String {
    let mut lines = vec![
        "[LLM compressed session segment]".to_string(),
        format!("Summary: {}", draft.summary.trim()),
    ];
    push_summary_section(&mut lines, "Goals", draft.goals);
    push_summary_section(&mut lines, "Decisions", draft.decisions);
    push_summary_section(&mut lines, "Completed work", draft.completed_work);
    push_summary_section(&mut lines, "Open questions", draft.open_questions);
    push_summary_section(&mut lines, "Files", draft.files);
    push_summary_section(&mut lines, "Risks", draft.risks);
    if let Some(archive_ref) = archive_ref {
        lines.push(format!("Archive: {}", archive_ref));
    }
    lines.join("\n")
}

fn push_summary_section(lines: &mut Vec<String>, title: &str, values: Vec<String>) {
    let values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return;
    }
    lines.push(format!("{}:", title));
    for value in values {
        lines.push(format!("- {}", value));
    }
}

fn build_deterministic_summary(
    candidate: &CompressionCandidateReport,
    source_events: &[SessionEvent],
    archive_ref: Option<&str>,
) -> String {
    let mut lines = vec![
        "[Active compressed session segment]".to_string(),
        format!("Kind: {:?}", candidate.kind),
        format!("Reason: {:?}", candidate.reason),
        format!("Source event count: {}", source_events.len()),
        format!("Source event ids: {}", candidate.event_ids.join(", ")),
        format!(
            "Original estimated bytes: {}",
            candidate.original_estimated_bytes
        ),
        format!(
            "Original estimated tokens: {}",
            candidate.original_estimated_tokens
        ),
    ];
    if let Some(first) = source_events.first() {
        let preview = canonical_event_text(first);
        let preview = preview.trim();
        if !preview.is_empty() {
            lines.push(format!("Preview: {}", truncate_preview(preview, 240)));
        }
    }
    if let Some(archive_ref) = archive_ref {
        lines.push(format!("Archive: {}", archive_ref));
    }
    lines.join("\n")
}

fn truncate_preview(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
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
            estimate_tokens_from_bytes_with_estimator(
                report.original_estimated_bytes,
                &report.token_estimator
            )
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
        assert_eq!(value["token_estimator"]["strategy"], "provider_heuristic");
        assert_eq!(
            value["token_estimator"]["effective_provider_id"],
            "deepseek"
        );
        assert_eq!(
            value["token_estimator"]["effective_chars_per_token_x100"],
            330
        );
    }

    #[test]
    fn dry_run_uses_target_provider_token_estimator() {
        let session = CanonicalSession {
            events: vec![text_event("old-user", EventRole::User, &"x".repeat(400))],
            ..sample_session()
        };
        let policy = ActiveCompressionPolicy {
            protect_recent_message_events: 0,
            min_candidate_bytes: 16,
            min_savings_ratio_percent: 20,
            mode: ActiveCompressionMode::PlanOnly,
        };

        let codex_report = build_dry_run_report(
            &session,
            ActiveCompressionParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: policy.clone(),
                dry_run: true,
            },
        );
        let kimi_report = build_dry_run_report(
            &session,
            ActiveCompressionParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "kimi".to_string(),
                policy,
                dry_run: true,
            },
        );

        assert_eq!(
            codex_report.token_estimator.effective_chars_per_token_x100,
            400
        );
        assert_eq!(
            kimi_report.token_estimator.effective_chars_per_token_x100,
            320
        );
        assert!(kimi_report.original_estimated_tokens > codex_report.original_estimated_tokens);
        assert!(
            kimi_report.candidates[0].original_estimated_tokens
                > codex_report.candidates[0].original_estimated_tokens
        );
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

    #[test]
    fn apply_writes_archive_and_replaces_selected_candidate() {
        let archive_dir = tempfile::tempdir().unwrap();
        let mut session = planner_sample_session();
        session.events.retain(|event| event.id != "compressed");
        let result = apply_active_compression_with_archive_dir(
            &session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 1,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::Auto,
                },
                candidate_ids: vec!["candidate-0001".to_string()],
            },
            archive_dir.path(),
        )
        .unwrap();

        assert_eq!(result.report.candidates.len(), 1);
        assert_eq!(result.report.archive_refs.len(), 1);
        assert!(result
            .session
            .events
            .iter()
            .all(|event| event.id != "old-user"));

        let compressed_event = result
            .session
            .events
            .iter()
            .find(|event| event.id == "memorph-active-compressed-candidate-0001")
            .expect("compressed replacement event");
        let EventBlock::Compressed {
            source_provider_id,
            source_event_ids,
            source_event_count,
            archive_ref,
            summary,
        } = compressed_event.blocks.first().expect("compressed block")
        else {
            panic!("expected compressed block");
        };
        assert_eq!(source_provider_id, "claude");
        assert_eq!(source_event_ids, &vec!["old-user".to_string()]);
        assert_eq!(*source_event_count, Some(1));
        assert_eq!(
            archive_ref.as_deref(),
            Some(result.report.archive_refs[0].as_str())
        );
        assert!(summary.contains("Archive: memorph-archive://"));

        let (expanded, expand_report) = compression::expand_compressed_segments_in_dir(
            &result.session,
            "claude",
            "codex",
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(expand_report.expanded_segments, 1);
        assert_eq!(expand_report.restored_events, 1);
        assert!(expanded.events.iter().any(|event| event.id == "old-user"));
    }

    #[test]
    fn apply_uses_external_summary_when_it_passes_size_gate() {
        let archive_dir = tempfile::tempdir().unwrap();
        let mut session = planner_sample_session();
        session.events.retain(|event| event.id != "compressed");
        let summarizer = FixedSummarizer {
            draft: CompressionSummaryDraft {
                summary: "LLM concise task state".to_string(),
                goals: Vec::new(),
                decisions: Vec::new(),
                completed_work: Vec::new(),
                open_questions: Vec::new(),
                files: Vec::new(),
                risks: Vec::new(),
            },
        };

        let result = apply_active_compression_with_archive_dir_and_summarizer(
            &session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 1,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::Auto,
                },
                candidate_ids: vec!["candidate-0001".to_string()],
            },
            archive_dir.path(),
            Some(&summarizer),
        )
        .unwrap();

        assert_eq!(
            result.report.candidates[0].summary_source,
            Some(CompressionSummarySource::External)
        );
        assert_eq!(result.report.candidates[0].summary_rejection_reason, None);
        let summary = compressed_summary(&result.session);
        assert!(summary.contains("[LLM compressed session segment]"));
        assert!(summary.contains("LLM concise task state"));
        assert!(summary.contains("Archive: memorph-archive://"));

        let (expanded, expand_report) = compression::expand_compressed_segments_in_dir(
            &result.session,
            "claude",
            "codex",
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(expand_report.restored_events, 1);
        assert!(expanded.events.iter().any(|event| event.id == "old-user"));
    }

    #[test]
    fn apply_rejects_oversized_external_summary_and_falls_back() {
        let archive_dir = tempfile::tempdir().unwrap();
        let mut session = planner_sample_session();
        session.events.retain(|event| event.id != "compressed");
        let summarizer = FixedSummarizer {
            draft: CompressionSummaryDraft {
                summary: "oversized ".repeat(200),
                ..CompressionSummaryDraft::default()
            },
        };

        let result = apply_active_compression_with_archive_dir_and_summarizer(
            &session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 1,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::Auto,
                },
                candidate_ids: vec!["candidate-0001".to_string()],
            },
            archive_dir.path(),
            Some(&summarizer),
        )
        .unwrap();

        assert_eq!(
            result.report.candidates[0].summary_source,
            Some(CompressionSummarySource::DeterministicFallback)
        );
        assert_eq!(
            result.report.candidates[0].summary_rejection_reason,
            Some(CompressionSummaryRejectionReason::TooLarge)
        );
        let summary = compressed_summary(&result.session);
        assert!(summary.contains("[Active compressed session segment]"));
        assert!(!summary.contains(&"oversized ".repeat(50)));
        assert!(summary.contains("Archive: memorph-archive://"));
    }

    #[test]
    fn apply_falls_back_when_external_summarizer_errors() {
        let archive_dir = tempfile::tempdir().unwrap();
        let mut session = planner_sample_session();
        session.events.retain(|event| event.id != "compressed");
        let summarizer = ErrorSummarizer;

        let result = apply_active_compression_with_archive_dir_and_summarizer(
            &session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 1,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::Auto,
                },
                candidate_ids: vec!["candidate-0001".to_string()],
            },
            archive_dir.path(),
            Some(&summarizer),
        )
        .unwrap();

        assert_eq!(
            result.report.candidates[0].summary_source,
            Some(CompressionSummarySource::DeterministicFallback)
        );
        assert_eq!(
            result.report.candidates[0].summary_rejection_reason,
            Some(CompressionSummaryRejectionReason::SummarizerError)
        );
        assert!(compressed_summary(&result.session).contains("[Active compressed session segment]"));
    }

    #[test]
    fn apply_without_candidates_returns_original_session() {
        let archive_dir = tempfile::tempdir().unwrap();
        let session = sample_session();
        let result = apply_active_compression_with_archive_dir(
            &session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy::default(),
                candidate_ids: Vec::new(),
            },
            archive_dir.path(),
        )
        .unwrap();

        assert!(result.report.candidates.is_empty());
        assert!(result.report.archive_refs.is_empty());
        assert_eq!(
            result
                .session
                .events
                .iter()
                .map(|event| event.id.clone())
                .collect::<Vec<_>>(),
            session
                .events
                .iter()
                .map(|event| event.id.clone())
                .collect::<Vec<_>>()
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

    fn compressed_summary(session: &CanonicalSession) -> String {
        session
            .events
            .iter()
            .flat_map(|event| event.blocks.iter())
            .find_map(|block| match block {
                EventBlock::Compressed { summary, .. } => Some(summary.clone()),
                _ => None,
            })
            .expect("compressed summary")
    }

    struct FixedSummarizer {
        draft: CompressionSummaryDraft,
    }

    impl ActiveCompressionSummarizer for FixedSummarizer {
        fn summarize(
            &self,
            _request: &CompressionSummaryRequest<'_>,
        ) -> Result<CompressionSummaryDraft> {
            Ok(self.draft.clone())
        }
    }

    struct ErrorSummarizer;

    impl ActiveCompressionSummarizer for ErrorSummarizer {
        fn summarize(
            &self,
            _request: &CompressionSummaryRequest<'_>,
        ) -> Result<CompressionSummaryDraft> {
            anyhow::bail!("summarizer failed")
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
