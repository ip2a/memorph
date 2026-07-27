use crate::canonical::{Block, Event, EventKind, Role, Session};
use crate::provider::canonical_event_text;

use super::content::{content_profile, DetectedContentKind};
use super::{
    estimate_event_bytes, estimate_tokens_from_bytes_with_estimator,
    protected_recent_message_indexes, ActiveCompressionPolicy, CompressionCandidateKind,
    CompressionCandidateReport, CompressionRisk, CompressionSelectionReason, CompressionSkipReason,
    CompressionSkipReport, CompressionTokenEstimatorReport,
};

pub(super) fn plan_compression_candidates_with_estimator(
    session: &Session,
    policy: &ActiveCompressionPolicy,
    token_estimator: &CompressionTokenEstimatorReport,
) -> (Vec<CompressionCandidateReport>, Vec<CompressionSkipReport>) {
    let protected_message_indexes = protected_recent_message_indexes(session, policy);
    let mut candidates = Vec::new();
    let mut skipped = Vec::new();

    let mut event_index = 0usize;
    while event_index < session.events.len() {
        let event = &session.events[event_index];
        let estimated_bytes = estimate_event_bytes(event);
        let estimated_tokens =
            estimate_tokens_from_bytes_with_estimator(estimated_bytes, token_estimator);

        if let Some(reason) = hard_skip_reason(event, event_index, &protected_message_indexes) {
            skipped.push(skip_report(
                event,
                event_index,
                reason,
                estimated_bytes,
                estimated_tokens,
            ));
            event_index += 1;
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
            event_index += 1;
            continue;
        };

        if kind == CompressionCandidateKind::HistoricalConversationRange {
            let range = collect_historical_range(
                session,
                event_index,
                &protected_message_indexes,
                token_estimator,
            );
            if range.estimated_bytes < policy.min_candidate_bytes {
                push_range_skips(
                    session,
                    range.start_event_index,
                    range.end_event_index,
                    CompressionSkipReason::BelowByteThreshold,
                    token_estimator,
                    &mut skipped,
                );
                event_index = range.end_event_index + 1;
                continue;
            }
            let range_estimated_saved = estimated_savings_bytes(
                range.estimated_bytes,
                CompressionCandidateKind::HistoricalConversationRange,
            );
            if savings_ratio_is_too_low(
                range.estimated_bytes,
                range_estimated_saved,
                policy.min_savings_ratio_percent,
            ) {
                push_range_skips(
                    session,
                    range.start_event_index,
                    range.end_event_index,
                    CompressionSkipReason::InsufficientEstimatedSavings,
                    token_estimator,
                    &mut skipped,
                );
                event_index = range.end_event_index + 1;
                continue;
            }
            push_candidate(
                &mut candidates,
                CandidateInput {
                    kind,
                    reason,
                    risk,
                    event_ids: range.event_ids,
                    start_event_index: range.start_event_index,
                    end_event_index: range.end_event_index,
                    estimated_bytes: range.estimated_bytes,
                    estimated_tokens: range.estimated_tokens,
                },
                token_estimator,
            );
            event_index = range.end_event_index + 1;
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
            event_index += 1;
            continue;
        }

        let estimated_bytes_saved = estimated_savings_bytes(estimated_bytes, kind);
        if savings_ratio_is_too_low(
            estimated_bytes,
            estimated_bytes_saved,
            policy.min_savings_ratio_percent,
        ) {
            skipped.push(skip_report(
                event,
                event_index,
                CompressionSkipReason::InsufficientEstimatedSavings,
                estimated_bytes,
                estimated_tokens,
            ));
            event_index += 1;
            continue;
        }

        push_candidate(
            &mut candidates,
            CandidateInput {
                kind,
                reason,
                risk,
                event_ids: vec![event.id.clone()],
                start_event_index: event_index,
                end_event_index: event_index,
                estimated_bytes,
                estimated_tokens,
            },
            token_estimator,
        );
        event_index += 1;
    }

    (candidates, skipped)
}

fn hard_skip_reason(
    event: &Event,
    event_index: usize,
    protected_message_indexes: &[usize],
) -> Option<CompressionSkipReason> {
    if matches!(event.role, Role::System | Role::Developer) {
        return Some(CompressionSkipReason::SystemOrDeveloperInstruction);
    }

    if event
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Compressed { .. }))
    {
        return Some(CompressionSkipReason::AlreadyCompressed);
    }

    if protected_message_indexes.contains(&event_index) {
        return Some(CompressionSkipReason::ProtectedRecentMessage);
    }

    None
}

struct HistoricalRange {
    start_event_index: usize,
    end_event_index: usize,
    event_ids: Vec<String>,
    estimated_bytes: usize,
    estimated_tokens: usize,
}

fn collect_historical_range(
    session: &Session,
    start_event_index: usize,
    protected_message_indexes: &[usize],
    token_estimator: &CompressionTokenEstimatorReport,
) -> HistoricalRange {
    let mut end_event_index = start_event_index;
    let mut event_ids = Vec::new();
    let mut estimated_bytes = 0usize;
    let mut estimated_tokens = 0usize;

    for idx in start_event_index..session.events.len() {
        let event = &session.events[idx];
        if hard_skip_reason(event, idx, protected_message_indexes).is_some() {
            break;
        }
        if !matches!(
            classify_candidate(event),
            Some((CompressionCandidateKind::HistoricalConversationRange, _, _))
        ) {
            break;
        }

        let event_bytes = estimate_event_bytes(event);
        event_ids.push(event.id.clone());
        estimated_bytes = estimated_bytes.saturating_add(event_bytes);
        estimated_tokens = estimated_tokens.saturating_add(
            estimate_tokens_from_bytes_with_estimator(event_bytes, token_estimator),
        );
        end_event_index = idx;
    }

    HistoricalRange {
        start_event_index,
        end_event_index,
        event_ids,
        estimated_bytes,
        estimated_tokens,
    }
}

fn push_range_skips(
    session: &Session,
    start_event_index: usize,
    end_event_index: usize,
    reason: CompressionSkipReason,
    token_estimator: &CompressionTokenEstimatorReport,
    skipped: &mut Vec<CompressionSkipReport>,
) {
    for idx in start_event_index..=end_event_index {
        let skipped_event = &session.events[idx];
        let skipped_bytes = estimate_event_bytes(skipped_event);
        let skipped_tokens =
            estimate_tokens_from_bytes_with_estimator(skipped_bytes, token_estimator);
        skipped.push(skip_report(
            skipped_event,
            idx,
            reason,
            skipped_bytes,
            skipped_tokens,
        ));
    }
}

struct CandidateInput {
    kind: CompressionCandidateKind,
    reason: CompressionSelectionReason,
    risk: CompressionRisk,
    event_ids: Vec<String>,
    start_event_index: usize,
    end_event_index: usize,
    estimated_bytes: usize,
    estimated_tokens: usize,
}

fn push_candidate(
    candidates: &mut Vec<CompressionCandidateReport>,
    input: CandidateInput,
    token_estimator: &CompressionTokenEstimatorReport,
) {
    let CandidateInput {
        kind,
        reason,
        risk,
        event_ids,
        start_event_index,
        end_event_index,
        estimated_bytes,
        estimated_tokens,
    } = input;
    let compressed_estimated_bytes = estimate_candidate_compressed_bytes(estimated_bytes, kind);
    let compressed_estimated_tokens =
        estimate_tokens_from_bytes_with_estimator(compressed_estimated_bytes, token_estimator);
    let estimated_bytes_saved = estimated_bytes.saturating_sub(compressed_estimated_bytes);
    let estimated_tokens_saved = estimated_tokens.saturating_sub(compressed_estimated_tokens);

    candidates.push(CompressionCandidateReport {
        id: format!("candidate-{:04}", candidates.len() + 1),
        kind,
        event_ids,
        start_event_index,
        end_event_index,
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

fn estimated_savings_bytes(estimated_bytes: usize, kind: CompressionCandidateKind) -> usize {
    estimated_bytes.saturating_sub(estimate_candidate_compressed_bytes(estimated_bytes, kind))
}

fn savings_ratio_is_too_low(
    estimated_bytes: usize,
    estimated_bytes_saved: usize,
    min_savings_ratio_percent: u8,
) -> bool {
    estimated_bytes_saved.saturating_mul(100)
        < estimated_bytes.saturating_mul(min_savings_ratio_percent as usize)
}

pub(super) fn classify_candidate(
    event: &Event,
) -> Option<(
    CompressionCandidateKind,
    CompressionSelectionReason,
    CompressionRisk,
)> {
    let text = canonical_event_text(event);
    let profile = content_profile(&text);

    if profile.kind == DetectedContentKind::SearchResults {
        return Some((
            CompressionCandidateKind::SearchResults,
            CompressionSelectionReason::LargeSearchResult,
            CompressionRisk::Low,
        ));
    }

    if profile.kind == DetectedContentKind::Diff {
        return Some((
            CompressionCandidateKind::LargeDiffOutput,
            CompressionSelectionReason::LargeDiffOutput,
            CompressionRisk::Low,
        ));
    }

    if event
        .blocks
        .iter()
        .any(|block| matches!(block, Block::CommandResult { .. }))
        || profile.kind == DetectedContentKind::Log
    {
        return Some((
            CompressionCandidateKind::LargeLogOutput,
            CompressionSelectionReason::LargeCommandOutput,
            CompressionRisk::Low,
        ));
    }

    if event
        .blocks
        .iter()
        .any(|block| matches!(block, Block::ToolResult { .. }))
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
        .any(|block| matches!(block, Block::ProviderPayload { .. }))
    {
        return Some((
            CompressionCandidateKind::ProviderPayloadText,
            CompressionSelectionReason::HistoricalContext,
            CompressionRisk::High,
        ));
    }

    if event.kind == EventKind::Message
        && matches!(event.role, Role::User | Role::Assistant | Role::Tool)
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

fn estimate_candidate_compressed_bytes(
    original_bytes: usize,
    kind: CompressionCandidateKind,
) -> usize {
    let ratio = match kind {
        CompressionCandidateKind::LargeToolOutput => 20,
        CompressionCandidateKind::LargeLogOutput => 20,
        CompressionCandidateKind::LargeDiffOutput => 25,
        CompressionCandidateKind::SearchResults => 30,
        CompressionCandidateKind::HistoricalConversationRange => 35,
        CompressionCandidateKind::ProviderPayloadText => 50,
    };
    let estimated = original_bytes.saturating_mul(ratio) / 100;
    estimated.max(128).min(original_bytes)
}

fn skip_report(
    event: &Event,
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
