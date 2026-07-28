use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::session::{Block, Event, EventKind, Fidelity, Links, Metadata, Role, Session, Source};
use crate::core::compression;
use crate::provider::canonical_event_text;

mod adaptive;
mod content;
mod planner;
mod reducer;

use reducer::reduce_candidate_to_summary;

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
    pub session: Session,
    pub report: ActiveCompressionReport,
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
#[derive(Default)]
pub enum ActiveCompressionMode {
    #[default]
    PlanOnly,
    Auto,
    Manual,
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
    LargeDiffOutput,
    SearchResults,
    ProviderPayloadText,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionSelectionReason {
    HistoricalContext,
    LargeToolOutput,
    LargeCommandOutput,
    LargeDiffOutput,
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
    session: &Session,
    params: ActiveCompressionParams,
) -> ActiveCompressionReport {
    let policy = params.policy;
    let token_estimator =
        build_token_estimator(&params.source_provider_id, &params.target_provider_id);
    let original_estimated_bytes = estimate_session_bytes(session);
    let original_estimated_tokens =
        estimate_tokens_from_bytes_with_estimator(original_estimated_bytes, &token_estimator);
    let (candidates, skipped) =
        planner::plan_compression_candidates_with_estimator(session, &policy, &token_estimator);
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
            .filter(|event| event.kind == EventKind::Message)
            .count(),
        already_compressed_event_count: session
            .events
            .iter()
            .filter(|event| {
                event
                    .blocks
                    .iter()
                    .any(|block| matches!(block, Block::Compressed { .. }))
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
    session: &Session,
    policy: &ActiveCompressionPolicy,
) -> (Vec<CompressionCandidateReport>, Vec<CompressionSkipReport>) {
    planner::plan_compression_candidates_with_estimator(
        session,
        policy,
        &CompressionTokenEstimatorReport::default(),
    )
}

pub fn apply_active_compression(
    session: &Session,
    params: ActiveCompressionApplyParams,
) -> Result<ActiveCompressionApplyResult> {
    let archive_dir = crate::config::memorph_dir()?.join("compression_archives");
    apply_active_compression_with_archive_dir(session, params, &archive_dir)
}

pub(crate) fn apply_active_compression_with_archive_dir(
    session: &Session,
    params: ActiveCompressionApplyParams,
    archive_dir: &Path,
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
        let summary_seed = reduce_candidate_to_summary(candidate, &source_events, None);
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
        let summary = reduce_candidate_to_summary(candidate, &source_events, Some(&archive_ref));
        replacement_events.push(active_summary_event(
            candidate,
            &source_events,
            &params.source_provider_id,
            summary,
            Some(archive_ref.clone()),
        ));
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
    report.archive_refs = archive_refs;

    Ok(ActiveCompressionApplyResult {
        session: next,
        report,
    })
}

pub fn estimate_session_bytes(session: &Session) -> usize {
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
    session: &Session,
    policy: &ActiveCompressionPolicy,
) -> Vec<usize> {
    session
        .events
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, event)| event.kind == EventKind::Message)
        .take(policy.protect_recent_message_events)
        .map(|(idx, _)| idx)
        .collect()
}

fn estimate_event_bytes(event: &Event) -> usize {
    canonical_event_text(event).len()
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
    source_events: &[Event],
    source_provider_id: &str,
    summary: String,
    archive_ref: Option<String>,
) -> Event {
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

    Event {
        id: format!("memorph-active-compressed-{}", candidate.id),
        kind: EventKind::Message,
        role: Role::Assistant,
        timestamp,
        links: Links::default(),
        blocks: vec![Block::Compressed {
            source_provider_id: source_provider_id.to_string(),
            summary,
            source_event_ids,
            source_event_count: Some(source_event_count),
            archive_ref,
        }],
        metadata: Metadata {
            source: Source {
                provider_id: "memorph".to_string(),
                original_id: Some(candidate.id.clone()),
                original_role: Some("assistant".to_string()),
                phase: Some("active-compression".to_string()),
            },
            model: None,
            usage: None,
            fidelity: Fidelity::Normalized,
            provider_ext,
        },
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
    use crate::session::{
        Block, Context, Event, Fidelity, Identity, Links, Metadata, Provenance, ProviderRef, Role,
        Schema, Source,
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
        let session = Session {
            events: vec![text_event("old-user", Role::User, &"x".repeat(400))],
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
        session.events = vec![text_event("old-user", Role::User, &"x".repeat(200))];
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
        let Block::Compressed {
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
        assert!(summary.contains("Recovery archive: memorph-archive://"));
        assert!(summary.contains("Retained signals:"));
        assert!(summary.contains("Rule strategy: conversation-range reducer"));
        assert!(summary.contains("Content profile: kind=conversation_text"));

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

    #[test]
    fn planner_groups_contiguous_historical_messages() {
        let mut session = sample_session();
        session.events = vec![
            text_event(
                "old-user",
                Role::User,
                &"user goal mentions src/core.rs ".repeat(8),
            ),
            text_event(
                "old-assistant",
                Role::Assistant,
                &"assistant decision mentions rust/crates/memorph/src/core.rs ".repeat(8),
            ),
            command_result_event(
                "command-output",
                &"warning: later command output\n".repeat(8),
            ),
            text_event("recent", Role::User, &"latest request ".repeat(8)),
        ];

        let report = build_dry_run_report(
            &session,
            ActiveCompressionParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 1,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::PlanOnly,
                },
                dry_run: true,
            },
        );

        assert_eq!(
            report.candidates[0].kind,
            CompressionCandidateKind::HistoricalConversationRange
        );
        assert_eq!(
            report.candidates[0].event_ids,
            vec!["old-user".to_string(), "old-assistant".to_string()]
        );
        assert_eq!(report.candidates[0].start_event_index, 0);
        assert_eq!(report.candidates[0].end_event_index, 1);
    }

    #[test]
    fn planner_routes_tool_result_by_detected_content() {
        let search_event = tool_result_event(
            "tool-search",
            &[
                "src/lib.rs:10:first match",
                "src/core.rs:22:second match",
                "src/api.rs:33:third match",
            ]
            .join("\n"),
        );
        let log_event = tool_result_event(
            "tool-log",
            "Compiling memorph\nwarning: unused import\nerror: build failed\n",
        );
        let diff_event = tool_result_event(
            "tool-diff",
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n-old\n+new\n",
        );
        let generic_event = tool_result_event("tool-generic", &"plain tool output ".repeat(16));

        assert_eq!(
            planner::classify_candidate(&search_event).map(|(kind, _, _)| kind),
            Some(CompressionCandidateKind::SearchResults)
        );
        assert_eq!(
            planner::classify_candidate(&log_event).map(|(kind, _, _)| kind),
            Some(CompressionCandidateKind::LargeLogOutput)
        );
        assert_eq!(
            planner::classify_candidate(&diff_event).map(|(kind, _, _)| kind),
            Some(CompressionCandidateKind::LargeDiffOutput)
        );
        assert_eq!(
            planner::classify_candidate(&generic_event).map(|(kind, _, _)| kind),
            Some(CompressionCandidateKind::LargeToolOutput)
        );
    }

    #[test]
    fn apply_reduces_diff_output_with_structural_signals() {
        let archive_dir = tempfile::tempdir().unwrap();
        let mut session = sample_session();
        session.events = vec![tool_result_event(
            "tool-diff",
            &[
                "diff --git a/src/lib.rs b/src/lib.rs",
                "--- a/src/lib.rs",
                "+++ b/src/lib.rs",
                "@@ -1,3 +1,3 @@",
                "-old behavior",
                "+new behavior",
            ]
            .join("\n")
            .repeat(12),
        )];

        let result = apply_active_compression_with_archive_dir(
            &session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 0,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::Auto,
                },
                candidate_ids: Vec::new(),
            },
            archive_dir.path(),
        )
        .unwrap();

        assert_eq!(
            result.report.candidates[0].kind,
            CompressionCandidateKind::LargeDiffOutput
        );
        let compressed_event = result
            .session
            .events
            .iter()
            .find(|event| event.id == "memorph-active-compressed-candidate-0001")
            .expect("compressed diff replacement event");
        let Block::Compressed { summary, .. } =
            compressed_event.blocks.first().expect("compressed block")
        else {
            panic!("expected compressed block");
        };
        assert!(summary.contains("Rule strategy: diff reducer"));
        assert!(summary.contains("Content profile: kind=diff"));
        assert!(summary.contains("Changed files: src/lib.rs"));
        assert!(summary.contains("Diff scale:"));
        assert!(summary.contains("Representative change: src/lib.rs"));
        assert!(summary.contains("Recovery archive: memorph-archive://"));
    }

    #[test]
    fn apply_reduces_search_results_with_grouped_matches_and_omissions() {
        let archive_dir = tempfile::tempdir().unwrap();
        let mut session = sample_session();
        let mut lines = Vec::new();
        for index in 1..=16 {
            lines.push(format!(
                "src/search.rs:{}:fn repeated_match_{}() {{}}",
                index * 2,
                index
            ));
        }
        lines.push("src/error.rs:91:error: important failure anchor".to_string());
        lines.push("src/error.rs:120:warning: important warning anchor".to_string());
        session.events = vec![tool_result_event("tool-search", &lines.join("\n"))];

        let result = apply_active_compression_with_archive_dir(
            &session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 0,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::Auto,
                },
                candidate_ids: Vec::new(),
            },
            archive_dir.path(),
        )
        .unwrap();

        assert_eq!(
            result.report.candidates[0].kind,
            CompressionCandidateKind::SearchResults
        );
        let summary = compressed_summary(&result.session, "candidate-0001");
        assert!(summary.contains("Rule strategy: search-results reducer"));
        assert!(summary.contains("Search matches: total=18"));
        assert!(summary.contains("Matched files:"));
        assert!(summary.contains("src/error.rs"));
        assert!(summary.contains("Match: src/error.rs:91 error: important failure anchor"));
        assert!(summary.contains("Omitted search matches:"));
    }

    #[test]
    fn apply_reduces_log_output_with_errors_warnings_stack_and_tail() {
        let archive_dir = tempfile::tempdir().unwrap();
        let mut session = sample_session();
        let stdout = [
            "Compiling memorph",
            "debug: low signal line 1",
            "debug: low signal line 2",
            "warning: unused import in src/core.rs",
            "thread 'main' panicked at src/main.rs:42",
            "stack backtrace:",
            "at memorph::core::run",
            "error: build failed",
            "test result: FAILED. 1 passed; 1 failed",
            "tail context one",
            "tail context two",
            "tail context three",
        ]
        .join("\n");
        session.events = vec![command_result_event_with_status(
            "command-output",
            Some("cargo test -p memorph".to_string()),
            Some(101),
            &stdout,
        )];

        let result = apply_active_compression_with_archive_dir(
            &session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 0,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::Auto,
                },
                candidate_ids: Vec::new(),
            },
            archive_dir.path(),
        )
        .unwrap();

        assert_eq!(
            result.report.candidates[0].kind,
            CompressionCandidateKind::LargeLogOutput
        );
        let summary = compressed_summary(&result.session, "candidate-0001");
        assert!(summary.contains("Rule strategy: log reducer"));
        assert!(summary.contains("Commands: cargo test -p memorph"));
        assert!(summary.contains("Exit codes: 101"));
        assert!(summary.contains("Log lines: total=12"));
        assert!(summary.contains("warning: unused import in src/core.rs"));
        assert!(summary.contains("error: build failed"));
        assert!(summary.contains("tail context three"));
    }

    fn sample_session() -> Session {
        let now = Utc::now();
        Session {
            schema: Schema::default(),
            identity: Identity {
                canonical_id: "active-compression-sample".to_string(),
                source_title: Some("Active compression sample".to_string()),
            },
            provenance: Provenance {
                imported_at: now,
                imported_by: Some("test".to_string()),
                primary_source: ProviderRef {
                    provider_id: "claude".to_string(),
                    session_id: "s1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: Context::default(),
            events: vec![
                text_event("u1", Role::User, "original task"),
                text_event("a1", Role::Assistant, "implementation notes"),
                compressed_event("c1"),
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        }
    }

    fn planner_sample_session() -> Session {
        let mut session = sample_session();
        session.events = vec![
            text_event("system", Role::System, &"system instruction ".repeat(4)),
            text_event("old-user", Role::User, &"historical context ".repeat(40)),
            tool_result_event("tool-output", &"tool output line\n".repeat(12)),
            command_result_event("command-output", &"compiler warning\n".repeat(12)),
            text_event(
                "search",
                Role::Tool,
                &[
                    "src/lib.rs:10:match one repeated context",
                    "src/core.rs:22:match two repeated context",
                    "src/api.rs:33:match three repeated context",
                ]
                .join("\n")
                .repeat(8),
            ),
            compressed_event("compressed"),
            text_event("small", Role::Assistant, "tiny"),
            text_event("recent", Role::User, &"latest active request ".repeat(8)),
        ];
        session
    }

    fn text_event(id: &str, role: Role, text: &str) -> Event {
        Event {
            id: id.to_string(),
            kind: EventKind::Message,
            role,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::Text {
                text: text.to_string(),
            }],
            metadata: event_metadata("claude", id),
        }
    }

    fn compressed_event(id: &str) -> Event {
        Event {
            id: id.to_string(),
            kind: EventKind::Message,
            role: Role::Assistant,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::Compressed {
                source_provider_id: "claude".to_string(),
                summary: "compressed summary".to_string(),
                source_event_ids: vec!["u0".to_string()],
                source_event_count: Some(1),
                archive_ref: Some("memorph-archive://s1/a.json.gz".to_string()),
            }],
            metadata: event_metadata("memorph", id),
        }
    }

    fn tool_result_event(id: &str, content: &str) -> Event {
        Event {
            id: id.to_string(),
            kind: EventKind::ToolResult,
            role: Role::Tool,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::ToolResult {
                tool_call_id: "tool-1".to_string(),
                content: content.to_string(),
                is_error: false,
            }],
            metadata: event_metadata("claude", id),
        }
    }

    fn command_result_event(id: &str, stdout: &str) -> Event {
        command_result_event_with_status(id, Some("cargo check".to_string()), Some(0), stdout)
    }

    fn command_result_event_with_status(
        id: &str,
        command: Option<String>,
        exit_code: Option<i32>,
        stdout: &str,
    ) -> Event {
        Event {
            id: id.to_string(),
            kind: EventKind::CommandResult,
            role: Role::Tool,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::CommandResult {
                command,
                exit_code,
                stdout: Some(stdout.to_string()),
                stderr: None,
            }],
            metadata: event_metadata("claude", id),
        }
    }

    fn event_metadata(provider_id: &str, original_id: &str) -> Metadata {
        Metadata {
            source: Source {
                provider_id: provider_id.to_string(),
                original_id: Some(original_id.to_string()),
                original_role: None,
                phase: None,
            },
            model: None,
            usage: None,
            fidelity: Fidelity::Preserved,
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

    fn compressed_summary(session: &Session, candidate_id: &str) -> String {
        let event_id = format!("memorph-active-compressed-{}", candidate_id);
        let event = session
            .events
            .iter()
            .find(|event| event.id == event_id)
            .expect("compressed replacement event");
        let Block::Compressed { summary, .. } = event.blocks.first().expect("compressed block")
        else {
            panic!("expected compressed block");
        };
        summary.clone()
    }
}
