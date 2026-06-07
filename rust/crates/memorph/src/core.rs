use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::canonical::{
    CanonicalSession, ImportedSession, SessionArtifact, SessionEvent, SessionEventKind,
};
use crate::core::active_compression::{
    ActiveCompressionApplyParams, ActiveCompressionMode, ActiveCompressionParams,
    ActiveCompressionPolicy, ActiveCompressionReport,
};
use crate::provider::ProviderSessionSummary;
use crate::storage::session_state::{self, SessionStateStore};
use crate::{provider, providers, utils};

pub mod active_compression;
pub mod compression;
pub mod manager;
pub mod session_management;

const MEMORPH_ARCHIVE_SCHEME: &str = "memorph-archive://";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListParams {
    pub all: bool,
    pub providers: Vec<String>,
    pub cwd: Option<String>,
    pub include_message_counts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGroup {
    pub provider_id: String,
    pub provider_name: String,
    pub sessions: Vec<SessionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionItem {
    pub session_id: String,
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_targets: Vec<String>,
    pub project_dir: Option<String>,
    pub last_active_at: Option<i64>,
    pub source_path: Option<String>,
    pub provider_id: String,
    pub message_count: Option<usize>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetailView {
    pub provider_id: String,
    pub provider_name: String,
    pub session_id: String,
    pub canonical_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    pub local_state: session_state::ResolvedLocalSessionState,
    pub event_count: usize,
    pub message_count: usize,
    pub artifact_count: usize,
    pub events: Vec<SessionEvent>,
    pub artifacts: Vec<SessionArtifact>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedSessionState {
    native_title: Option<String>,
    local: session_state::ResolvedLocalSessionState,
}

impl ResolvedSessionState {
    fn resolved_title(&self) -> Option<&str> {
        self.local
            .display_title
            .as_deref()
            .or(self.native_title.as_deref())
    }
}

impl From<(&ProviderSessionSummary, &str)> for SessionItem {
    fn from((meta, provider_id): (&ProviderSessionSummary, &str)) -> Self {
        Self {
            session_id: meta.session_id.clone(),
            title: meta.title.clone(),
            native_title: meta.title.clone(),
            display_title: None,
            hidden: false,
            pinned: false,
            preferred_targets: Vec::new(),
            project_dir: meta.project_dir.as_deref().map(utils::user_visible_path),
            last_active_at: meta.last_active_at,
            source_path: meta.source_path.as_deref().map(utils::user_visible_path),
            provider_id: provider_id.to_string(),
            message_count: None,
            size_bytes: None,
        }
    }
}

pub fn resolve_providers(filter: &[String]) -> Vec<String> {
    if filter.is_empty() {
        providers::all_provider_ids()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        filter.to_vec()
    }
}

pub fn list_sessions(params: &SessionListParams) -> Result<Vec<SessionGroup>> {
    let provider_ids = resolve_providers(&params.providers);
    let explicit_provider_filter = !params.providers.is_empty();
    let session_states = session_state::load_state_store().unwrap_or_default();
    let mut groups = Vec::new();

    for pid in &provider_ids {
        let prov = match providers::find_provider(pid) {
            Some(p) => p,
            None => continue,
        };
        let capabilities = prov.capabilities();
        if !capabilities.scan {
            continue;
        }
        let Some(sessions) = scan_sessions_for_aggregate(prov.as_ref(), explicit_provider_filter)?
        else {
            continue;
        };
        let filtered_summaries: Vec<&ProviderSessionSummary> = if params.all {
            sessions.iter().collect()
        } else {
            let cwd = params.cwd.as_deref().unwrap_or("");
            sessions
                .iter()
                .filter(|s| prov.workspace_matches(s.project_dir.as_deref(), Some(cwd)))
                .collect()
        };
        let session_ids: Vec<&str> = filtered_summaries
            .iter()
            .map(|s| s.session_id.as_str())
            .collect();
        let sizes = prov.session_sizes(&session_ids);
        let mut filtered: Vec<SessionItem> = filtered_summaries
            .iter()
            .map(|s| {
                enrich_session_item(
                    prov.as_ref(),
                    capabilities,
                    pid.as_str(),
                    s,
                    &session_states,
                    &sizes,
                    params.include_message_counts,
                )
            })
            .collect();
        filtered.sort_by_key(|s| {
            (
                std::cmp::Reverse(s.pinned),
                std::cmp::Reverse(s.last_active_at),
            )
        });

        if !filtered.is_empty() {
            groups.push(SessionGroup {
                provider_id: pid.clone(),
                provider_name: prov.name().to_string(),
                sessions: filtered,
            });
        }
    }

    Ok(groups)
}

fn scan_sessions_for_aggregate(
    provider: &dyn provider::Provider,
    explicit_provider_filter: bool,
) -> Result<Option<Vec<ProviderSessionSummary>>> {
    match provider.scan_sessions() {
        Ok(sessions) => Ok(Some(sessions)),
        Err(err) if explicit_provider_filter => Err(err),
        Err(_) => Ok(None),
    }
}

fn enrich_session_item(
    provider: &dyn provider::Provider,
    capabilities: provider::ProviderCapabilities,
    provider_id: &str,
    meta: &ProviderSessionSummary,
    session_states: &SessionStateStore,
    sizes: &HashMap<String, u64>,
    include_message_count: bool,
) -> SessionItem {
    let mut item = SessionItem::from((meta, provider_id));
    let state = resolve_session_state(
        provider_id,
        &meta.session_id,
        meta.title.clone(),
        meta.project_dir.as_deref(),
        session_states,
    );
    apply_session_item_state(&mut item, &state);
    item.size_bytes = sizes.get(&meta.session_id).copied().or_else(|| {
        meta.source_path.as_deref().and_then(|path| {
            std::fs::metadata(path)
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len())
        })
    });

    if include_message_count && capabilities.import {
        if let Some(source_path) = meta.source_path.as_deref() {
            item.message_count = provider
                .import_session(source_path)
                .ok()
                .map(|imported| session_message_count(&imported.session));
        }
    }

    item
}

pub fn get_canonical_session(provider_id: &str, session_id: &str) -> Result<ImportedSession> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let capabilities = prov.capabilities();
    if !capabilities.scan || !capabilities.import {
        anyhow::bail!(
            "Provider does not support loading sessions: {}",
            provider_id
        );
    }

    let meta = prov
        .scan_sessions()?
        .into_iter()
        .find(|session| session.session_id == session_id)
        .with_context(|| format!("Session not found: {}", session_id))?;

    load_canonical_session_from_meta(prov.as_ref(), provider_id, meta)
}

pub fn get_session_detail_view(provider_id: &str, session_id: &str) -> Result<SessionDetailView> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let capabilities = prov.capabilities();
    if !capabilities.scan || !capabilities.import {
        anyhow::bail!(
            "Provider does not support loading sessions: {}",
            provider_id
        );
    }

    let meta = prov
        .scan_sessions()?
        .into_iter()
        .find(|session| session.session_id == session_id)
        .with_context(|| format!("Session not found: {}", session_id))?;
    let source_path = meta.source_path.clone();
    let workspace_dir = meta.project_dir.clone();
    let native_title = meta.title.clone();
    let imported = load_canonical_session_from_meta(prov.as_ref(), provider_id, meta)?;
    let local_state =
        get_resolved_local_session_state(provider_id, session_id, workspace_dir.as_deref());

    Ok(build_session_detail_view(
        provider_id,
        prov.name(),
        session_id,
        source_path,
        prov.resume_command(session_id),
        native_title,
        local_state,
        imported,
    ))
}

pub fn get_resolved_local_session_state(
    provider_id: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> session_state::ResolvedLocalSessionState {
    let session_states = session_state::load_state_store().unwrap_or_default();
    let workspace_dir = session_management::normalized_workspace_key(provider_id, workspace_dir);
    session_state::resolve_session_state(
        &session_states,
        provider_id,
        session_id,
        workspace_dir.as_deref(),
    )
}

fn build_session_detail_view(
    provider_id: &str,
    provider_name: &str,
    session_id: &str,
    source_path: Option<String>,
    resume_command: Option<String>,
    native_title: Option<String>,
    local_state: session_state::ResolvedLocalSessionState,
    imported: ImportedSession,
) -> SessionDetailView {
    let session = imported.session;
    let display_title = local_state.display_title.clone();
    let title = display_title
        .clone()
        .or_else(|| native_title.clone())
        .or_else(|| session.identity.source_title.clone());
    let message_count = session_message_count(&session);
    let source_path = source_path
        .or_else(|| session.provenance.primary_source.source_path.clone())
        .as_deref()
        .map(utils::user_visible_path);

    SessionDetailView {
        provider_id: provider_id.to_string(),
        provider_name: provider_name.to_string(),
        session_id: session_id.to_string(),
        canonical_id: session.identity.canonical_id.clone(),
        title,
        native_title,
        display_title,
        workspace_dir: session
            .context
            .workspace_dir
            .as_deref()
            .map(utils::user_visible_path),
        created_at: session.context.created_at,
        last_active_at: session.context.last_active_at,
        source_path,
        resume_command,
        local_state,
        event_count: session.events.len(),
        message_count,
        artifact_count: session.artifacts.len(),
        events: session.events,
        artifacts: session.artifacts,
    }
}

fn resolve_session_state(
    provider_id: &str,
    session_id: &str,
    native_title: Option<String>,
    workspace_dir: Option<&str>,
    session_states: &SessionStateStore,
) -> ResolvedSessionState {
    let workspace_dir = session_management::normalized_workspace_key(provider_id, workspace_dir);
    ResolvedSessionState {
        native_title,
        local: session_state::resolve_session_state(
            session_states,
            provider_id,
            session_id,
            workspace_dir.as_deref(),
        ),
    }
}

fn apply_session_item_state(item: &mut SessionItem, state: &ResolvedSessionState) {
    item.native_title = state.native_title.clone();
    item.display_title = state.local.display_title.clone();
    item.title = state.resolved_title().map(str::to_string);
    item.hidden = state.local.hidden;
    item.pinned = state.local.pinned;
    item.preferred_targets = state.local.preferred_targets.clone();
}

fn session_message_count(session: &CanonicalSession) -> usize {
    session
        .events
        .iter()
        .filter(|event| !matches!(event.kind, SessionEventKind::Lifecycle))
        .count()
}

fn load_canonical_session_from_meta(
    provider: &dyn provider::Provider,
    provider_id: &str,
    meta: ProviderSessionSummary,
) -> Result<ImportedSession> {
    let source_path = meta
        .source_path
        .as_deref()
        .context("Session has no source path")?;
    let mut imported = provider.import_session(source_path)?;
    if imported.session.identity.source_title.is_none() {
        imported.session.identity.source_title = meta.title.clone();
    }
    if imported.session.context.workspace_dir.is_none() {
        imported.session.context.workspace_dir = meta.project_dir.clone();
    }
    if imported.session.context.last_active_at.is_none() {
        imported.session.context.last_active_at = meta
            .last_active_at
            .and_then(chrono::DateTime::from_timestamp_millis);
    }
    if imported
        .session
        .provenance
        .aliases
        .iter()
        .all(|alias| alias.provider_id != provider_id || alias.session_id != meta.session_id)
    {
        imported
            .session
            .provenance
            .aliases
            .push(crate::canonical::ProviderSessionRef {
                provider_id: provider_id.to_string(),
                session_id: meta.session_id,
                source_path: meta.source_path,
            });
    }
    Ok(imported)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportParams {
    pub provider: String,
    pub session_id: String,
    pub output_prefix: Option<String>,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub files: Vec<String>,
}

pub fn export_session(params: &ExportParams) -> Result<ExportResult> {
    let imported = get_canonical_session(&params.provider, &params.session_id)?;

    let prefix = params
        .output_prefix
        .as_deref()
        .unwrap_or(&params.session_id);
    session_management::write_session_export_files(&imported.session, prefix, &params.format)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpandCompressionSessionParams {
    pub file: String,
    pub output_prefix: Option<String>,
    pub format: String,
}

pub fn expand_compression_session(params: &ExpandCompressionSessionParams) -> Result<ExportResult> {
    session_management::expand_compression_session(params)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreCompressionArchiveParams {
    pub archive_ref: String,
    pub output_prefix: Option<String>,
    pub format: String,
}

pub fn restore_compression_archive(
    params: &RestoreCompressionArchiveParams,
) -> Result<ExportResult> {
    session_management::restore_compression_archive(params)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveCompressionArchiveParams {
    pub archive_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedCompressionArchive {
    pub archive_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<usize>,
    pub retrieval_mode: CompressionRetrievalMode,
    pub recommended_next_action: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub canonical_id: String,
    pub source_provider_id: String,
    pub target_provider_id: String,
    pub summary_event_id: String,
    pub source_event_ids: Vec<String>,
    pub source_event_count: usize,
    pub returned_event_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<RetrievedCompressionArchiveMatch>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionRetrievalMode {
    FullArchive,
    QueryMatches,
    QueryNoMatches,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedCompressionArchiveMatch {
    pub event_id: String,
    pub event_index: usize,
    pub score: usize,
    pub snippets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalToolSpec {
    pub name: String,
    pub description: String,
    pub archive_ref_scheme: String,
    pub api: CompressionRetrievalToolApiSpec,
    pub cli: CompressionRetrievalToolCliSpec,
    pub input_schema: serde_json::Value,
    pub output_contract: CompressionRetrievalToolOutputContract,
    pub usage_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalToolApiSpec {
    pub method: String,
    pub path: String,
    pub body_example: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalToolCliSpec {
    pub command: String,
    pub query_command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalToolOutputContract {
    pub full_retrieval: Vec<String>,
    pub query_retrieval: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalInstructions {
    pub archive_ref: String,
    pub summary: String,
    pub query_first_cli: String,
    pub full_cli: String,
    pub api_query_body: serde_json::Value,
    pub api_full_body: serde_json::Value,
    pub suggested_steps: Vec<String>,
}

pub fn compression_retrieval_tool_spec() -> CompressionRetrievalToolSpec {
    CompressionRetrievalToolSpec {
        name: "memorph_retrieve_compression_archive".to_string(),
        description: "Retrieve original events from a durable memorph compression archive. Use query retrieval first when only specific details are needed.".to_string(),
        archive_ref_scheme: MEMORPH_ARCHIVE_SCHEME.to_string(),
        api: CompressionRetrievalToolApiSpec {
            method: "POST".to_string(),
            path: "/api/v1/compression/retrieve".to_string(),
            body_example: serde_json::json!({
                "archive_ref": "memorph-archive://canonical-id/archive.json.gz",
                "query": "optional search terms",
                "max_results": 5
            }),
        },
        cli: CompressionRetrievalToolCliSpec {
            command: "memorph compression retrieve <ARCHIVE_REF>".to_string(),
            query_command:
                "memorph compression retrieve <ARCHIVE_REF> --query <QUERY> --max-results 5"
                    .to_string(),
        },
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "archive_ref": {
                    "type": "string",
                    "description": "Durable archive reference from a compressed session block. Must start with memorph-archive://."
                },
                "query": {
                    "type": "string",
                    "description": "Optional search query. When provided, retrieval returns only matching archived events and snippets."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Maximum matching events returned in query mode. Omit to use the default."
                }
            },
            "required": ["archive_ref"]
        }),
        output_contract: CompressionRetrievalToolOutputContract {
            full_retrieval: vec![
                "retrieval_mode is full_archive".to_string(),
                "events contains every original archived SessionEvent".to_string(),
                "source_event_count equals the archive's full original event count".to_string(),
                "returned_event_count equals events.length".to_string(),
            ],
            query_retrieval: vec![
                "retrieval_mode is query_matches or query_no_matches".to_string(),
                "events contains only matching archived SessionEvent values".to_string(),
                "matches contains event_id, event_index, score, and snippets for each returned event".to_string(),
                "source_event_count still reports the archive's full original event count".to_string(),
            ],
        },
        usage_rules: vec![
            "Do not expand a compressed archive unconditionally when switching or continuing a session.".to_string(),
            "Prefer query retrieval before full retrieval to avoid putting large archived history back into context.".to_string(),
            "Use full retrieval only when the task explicitly requires the complete original segment.".to_string(),
            "Archive retrieval is lossless; summaries are model-visible hints, not the source of truth.".to_string(),
        ],
    }
}

pub fn compression_retrieval_instructions(
    archive_ref: &str,
) -> Result<CompressionRetrievalInstructions> {
    let archive_ref = archive_ref.trim();
    if !archive_ref.starts_with(MEMORPH_ARCHIVE_SCHEME) {
        anyhow::bail!("Unsupported compression archive ref: {}", archive_ref);
    }

    Ok(CompressionRetrievalInstructions {
        archive_ref: archive_ref.to_string(),
        summary: "Use query retrieval first. Full retrieval should be reserved for tasks that explicitly need the entire original compressed segment.".to_string(),
        query_first_cli: format!(
            "memorph compression retrieve {} --query <terms> --max-results 5",
            archive_ref
        ),
        full_cli: format!("memorph compression retrieve {}", archive_ref),
        api_query_body: serde_json::json!({
            "archive_ref": archive_ref,
            "query": "<terms>",
            "max_results": 5
        }),
        api_full_body: serde_json::json!({
            "archive_ref": archive_ref
        }),
        suggested_steps: vec![
            "Extract the memorph-archive://... value from the compressed block.".to_string(),
            "Choose a narrow query from the current user question or missing detail.".to_string(),
            "Run query retrieval and use only the returned matching events/snippets.".to_string(),
            "Use full retrieval only if query retrieval is insufficient and complete original context is required.".to_string(),
        ],
    })
}

pub fn retrieve_compression_archive(
    params: &RetrieveCompressionArchiveParams,
) -> Result<RetrievedCompressionArchive> {
    let archive = compression::load_archive(&params.archive_ref)?;
    Ok(retrieved_compression_archive(params, archive))
}

#[cfg(test)]
fn retrieve_compression_archive_in_dir(
    params: &RetrieveCompressionArchiveParams,
    archive_dir: &std::path::Path,
) -> Result<RetrievedCompressionArchive> {
    let archive = compression::load_archive_from_dir(archive_dir, &params.archive_ref)?;
    Ok(retrieved_compression_archive(params, archive))
}

fn retrieved_compression_archive(
    params: &RetrieveCompressionArchiveParams,
    archive: compression::CompressionArchive,
) -> RetrievedCompressionArchive {
    let source_event_count = archive.events.len();
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty());
    let max_results = params.max_results;
    let (events, matches) = if let Some(query) = query {
        search_archive_events(&archive.events, query, max_results.unwrap_or(20))
    } else {
        (archive.events, Vec::new())
    };
    let returned_event_count = events.len();
    let retrieval_mode = match (query, returned_event_count) {
        (Some(_), 0) => CompressionRetrievalMode::QueryNoMatches,
        (Some(_), _) => CompressionRetrievalMode::QueryMatches,
        (None, _) => CompressionRetrievalMode::FullArchive,
    };
    let recommended_next_action = retrieval_next_action(retrieval_mode);
    RetrievedCompressionArchive {
        archive_ref: params.archive_ref.clone(),
        query: query.map(str::to_string),
        max_results,
        retrieval_mode,
        recommended_next_action,
        created_at: archive.created_at,
        canonical_id: archive.canonical_id,
        source_provider_id: archive.source_provider_id,
        target_provider_id: archive.target_provider_id,
        summary_event_id: archive.summary_event_id,
        source_event_ids: archive.source_event_ids,
        source_event_count,
        returned_event_count,
        matches,
        events,
    }
}

fn retrieval_next_action(mode: CompressionRetrievalMode) -> String {
    match mode {
        CompressionRetrievalMode::FullArchive => {
            "This is the complete archived segment. Use only the needed parts in the active context."
        }
        CompressionRetrievalMode::QueryMatches => {
            "This is a query-filtered partial retrieval. Treat it as relevant snippets/events, not the complete archived history."
        }
        CompressionRetrievalMode::QueryNoMatches => {
            "No archived events matched this query. Try a broader query or use full retrieval only if the complete original segment is required."
        }
    }
    .to_string()
}

fn search_archive_events(
    events: &[SessionEvent],
    query: &str,
    max_results: usize,
) -> (Vec<SessionEvent>, Vec<RetrievedCompressionArchiveMatch>) {
    if max_results == 0 {
        return (Vec::new(), Vec::new());
    }
    let query_lower = query.to_ascii_lowercase();
    let terms = query_lower
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return (events.to_vec(), Vec::new());
    }

    let mut ranked = events
        .iter()
        .enumerate()
        .filter_map(|(event_index, event)| {
            let text = provider::canonical_event_text(event);
            let text_lower = text.to_ascii_lowercase();
            let mut score = terms
                .iter()
                .map(|term| text_lower.matches(term).count())
                .sum::<usize>();
            if text_lower.contains(&query_lower) {
                score += 5;
            }
            if score == 0 {
                return None;
            }
            let snippets = archive_search_snippets(&text, &query_lower, &terms);
            Some((
                event_index,
                score,
                event.clone(),
                RetrievedCompressionArchiveMatch {
                    event_id: event.id.clone(),
                    event_index,
                    score,
                    snippets,
                },
            ))
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ranked.truncate(max_results);

    let events = ranked
        .iter()
        .map(|(_, _, event, _)| event.clone())
        .collect::<Vec<_>>();
    let matches = ranked
        .into_iter()
        .map(|(_, _, _, search_match)| search_match)
        .collect::<Vec<_>>();
    (events, matches)
}

fn archive_search_snippets(text: &str, query_lower: &str, terms: &[&str]) -> Vec<String> {
    let mut snippets = text
        .lines()
        .filter_map(|line| {
            let line_lower = line.to_ascii_lowercase();
            if line_lower.contains(query_lower)
                || terms.iter().any(|term| line_lower.contains(term))
            {
                Some(truncate_search_snippet(line.trim(), 240))
            } else {
                None
            }
        })
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>();
    if snippets.is_empty() {
        snippets.push(truncate_search_snippet(text.trim(), 240));
    }
    snippets
}

fn truncate_search_snippet(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

pub fn list_compression_archives() -> Result<Vec<compression::CompressionArchiveSummary>> {
    session_management::list_compression_archives()
}

pub fn list_compression_provider_support() -> Vec<compression::ProviderCompressionSupport> {
    session_management::list_compression_provider_support()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCompressionDryRunParams {
    pub source_provider_id: String,
    pub target_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default)]
    pub policy: ActiveCompressionPolicy,
}

pub fn active_compression_dry_run(
    params: &ActiveCompressionDryRunParams,
) -> Result<ActiveCompressionReport> {
    let session = load_active_compression_source_session(
        &params.source_provider_id,
        params.session_id.as_deref(),
        params.file.as_deref(),
    )?;

    Ok(active_compression::build_dry_run_report(
        &session,
        ActiveCompressionParams {
            source_provider_id: params.source_provider_id.clone(),
            target_provider_id: params.target_provider_id.clone(),
            policy: params.policy.clone(),
            dry_run: true,
        },
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCompressionApplyCommandParams {
    pub source_provider_id: String,
    pub target_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default)]
    pub policy: ActiveCompressionPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_prefix: Option<String>,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCompressionApplyCommandResult {
    pub files: Vec<String>,
    pub archive_refs: Vec<String>,
    pub report: ActiveCompressionReport,
}

pub fn active_compression_apply(
    params: &ActiveCompressionApplyCommandParams,
) -> Result<ActiveCompressionApplyCommandResult> {
    let session = load_active_compression_source_session(
        &params.source_provider_id,
        params.session_id.as_deref(),
        params.file.as_deref(),
    )?;
    active_compression_apply_session(params, &session, None)
}

fn active_compression_apply_session(
    params: &ActiveCompressionApplyCommandParams,
    session: &CanonicalSession,
    archive_dir: Option<&std::path::Path>,
) -> Result<ActiveCompressionApplyCommandResult> {
    let apply_params = ActiveCompressionApplyParams {
        source_provider_id: params.source_provider_id.clone(),
        target_provider_id: params.target_provider_id.clone(),
        policy: params.policy.clone(),
        candidate_ids: params.candidate_ids.clone(),
    };
    let applied = if let Some(archive_dir) = archive_dir {
        active_compression::apply_active_compression_with_archive_dir(
            session,
            apply_params,
            archive_dir,
        )?
    } else {
        active_compression::apply_active_compression(session, apply_params)?
    };
    let default_prefix = format!("{}_active_compressed", session.identity.canonical_id);
    let prefix = params.output_prefix.as_deref().unwrap_or(&default_prefix);
    let export =
        session_management::write_session_export_files(&applied.session, prefix, &params.format)?;

    Ok(ActiveCompressionApplyCommandResult {
        files: export.files,
        archive_refs: applied.report.archive_refs.clone(),
        report: applied.report,
    })
}

#[cfg(test)]
fn active_compression_apply_with_archive_dir(
    params: &ActiveCompressionApplyCommandParams,
    archive_dir: &std::path::Path,
) -> Result<ActiveCompressionApplyCommandResult> {
    let session = load_active_compression_source_session(
        &params.source_provider_id,
        params.session_id.as_deref(),
        params.file.as_deref(),
    )?;
    active_compression_apply_session(params, &session, Some(archive_dir))
}

fn load_active_compression_source_session(
    source_provider_id: &str,
    session_id: Option<&str>,
    file: Option<&str>,
) -> Result<CanonicalSession> {
    match (session_id, file) {
        (Some(_), Some(_)) => anyhow::bail!("Use either session_id or file, not both"),
        (Some(session_id), None) => {
            Ok(get_canonical_session(source_provider_id, session_id)?.session)
        }
        (None, Some(file)) => session_management::read_session_export_file(file),
        (None, None) => anyhow::bail!("Either session_id or file is required"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportParams {
    pub provider: String,
    pub file_or_id: String,
    pub to_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub provider_name: String,
    pub new_session_id: String,
    pub resume_command: Option<String>,
}

pub fn import_session(params: &ImportParams) -> Result<ImportResult> {
    let session = if params.file_or_id.ends_with(".morph")
        || params.file_or_id.ends_with(".json")
        || params.file_or_id.ends_with(".md")
        || params.file_or_id.ends_with(".html")
    {
        session_management::read_session_export_file(&params.file_or_id)?
    } else {
        get_canonical_session(&params.provider, &params.file_or_id)?.session
    };

    let target_prov = providers::find_provider(&params.provider)
        .with_context(|| format!("Target provider not available: {}", params.provider))?;
    let target_capabilities = target_prov.capabilities();
    if !target_capabilities.export {
        anyhow::bail!(
            "Provider does not support writing sessions: {}",
            params.provider
        );
    }
    let target_dir = target_prov.resolve_workspace_dir(params.to_dir.as_deref())?;
    let (session, _) =
        session_management::prepare_session_for_target_provider(&session, &params.provider)?;
    let exported = target_prov.export_session(&session, &target_dir)?;

    Ok(ImportResult {
        provider_name: target_prov.name().to_string(),
        new_session_id: exported.session_id,
        resume_command: exported.resume_command,
    })
}

pub fn delete_session(provider_id: &str, session_id: &str) -> Result<()> {
    session_management::delete_session(provider_id, session_id)
}

pub fn delete_sessions(provider_id: &str, session_ids: &[&str]) -> Vec<Result<()>> {
    session_management::delete_sessions(provider_id, session_ids)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameResult {
    pub provider_name: String,
    pub session_id: String,
    pub display_title: String,
    pub native_updated: bool,
    pub warning: Option<String>,
}

pub fn rename_session(
    provider_id: &str,
    session_id: &str,
    new_title: &str,
) -> Result<RenameResult> {
    session_management::rename_session(provider_id, session_id, new_title)
}

pub fn update_session_local_state(
    provider_id: &str,
    session_id: &str,
    update: &session_state::SessionLocalStateUpdate,
) -> Result<session_state::ResolvedLocalSessionState> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let capabilities = prov.capabilities();
    if capabilities.scan {
        let exists = prov
            .scan_sessions()?
            .into_iter()
            .any(|session| session.session_id == session_id);
        if !exists {
            anyhow::bail!("Session not found: {}", session_id);
        }
    }

    let mut normalized_update = update.clone();
    if let Some(workspace_override) = normalized_update.workspace_override.as_mut() {
        let workspace = workspace_override.workspace_dir.trim();
        if workspace.is_empty() {
            anyhow::bail!("Workspace path cannot be empty");
        }
        workspace_override.workspace_dir = prov
            .normalized_workspace_key(Some(workspace))
            .with_context(|| format!("Failed to normalize workspace: {}", workspace))?;
    }

    session_state::update_session_state(provider_id, session_id, &normalized_update)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchParams {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_compression: Option<ActiveCompressionPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchResult {
    pub from_name: String,
    pub to_name: String,
    pub source_session_id: String,
    pub target_session_id: String,
    pub resume_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_compression_report: Option<ActiveCompressionReport>,
}

pub fn switch_session(params: &SwitchParams) -> Result<SwitchResult> {
    let cwd = std::env::current_dir()?;

    let source_prov = providers::find_provider(&params.from)
        .with_context(|| format!("Unknown source provider: {}", params.from))?;
    let source_capabilities = source_prov.capabilities();
    if !source_capabilities.scan || !source_capabilities.import {
        anyhow::bail!(
            "Source provider does not support reading sessions: {}",
            params.from
        );
    }
    let sessions = source_prov.scan_sessions()?;
    let cwd_str = cwd.to_string_lossy().to_string();

    let session_meta = if let Some(id) = &params.session_id {
        sessions
            .into_iter()
            .find(|s| s.session_id == *id)
            .with_context(|| format!("Session not found: {}", id))?
    } else {
        let mut candidates: Vec<_> = sessions
            .into_iter()
            .filter(|s| source_prov.workspace_matches(s.project_dir.as_deref(), Some(&cwd_str)))
            .collect();
        candidates.sort_by_key(|s| std::cmp::Reverse(s.last_active_at));
        candidates.into_iter().next().with_context(|| {
            format!(
                "No {} session found in current workspace: {}\nUse --session-id to specify one, or run from the project directory.",
                source_prov.name(),
                cwd_str
            )
        })?
    };

    let source_session_id = session_meta.session_id.clone();
    let imported =
        load_canonical_session_from_meta(source_prov.as_ref(), &params.from, session_meta)?;

    let target_prov = providers::find_provider(&params.to)
        .with_context(|| format!("Unknown target provider: {}", params.to))?;
    let target_capabilities = target_prov.capabilities();
    if !target_capabilities.export {
        anyhow::bail!(
            "Target provider does not support writing sessions: {}",
            params.to
        );
    }
    let target_dir = target_prov.resolve_workspace_dir(params.to_dir.as_deref())?;
    let (source_session, active_compression_report) = apply_switch_active_compression(
        &imported.session,
        &params.from,
        &params.to,
        params.active_compression.clone(),
    )?;
    let (session, _) =
        session_management::prepare_session_for_export(&source_session, &params.from, &params.to)?;
    let exported = target_prov.export_session(&session, &target_dir)?;

    Ok(SwitchResult {
        from_name: source_prov.name().to_string(),
        to_name: target_prov.name().to_string(),
        source_session_id,
        target_session_id: exported.session_id,
        resume_command: exported.resume_command,
        active_compression_report,
    })
}

fn apply_switch_active_compression(
    session: &CanonicalSession,
    source_provider_id: &str,
    target_provider_id: &str,
    policy: Option<ActiveCompressionPolicy>,
) -> Result<(CanonicalSession, Option<ActiveCompressionReport>)> {
    let Some(mut policy) = policy else {
        return Ok((session.clone(), None));
    };
    policy.mode = ActiveCompressionMode::Auto;
    let applied = active_compression::apply_active_compression(
        session,
        ActiveCompressionApplyParams {
            source_provider_id: source_provider_id.to_string(),
            target_provider_id: target_provider_id.to_string(),
            policy,
            candidate_ids: Vec::new(),
        },
    )?;
    Ok((applied.session, Some(applied.report)))
}

#[cfg(test)]
fn apply_switch_active_compression_with_archive_dir(
    session: &CanonicalSession,
    source_provider_id: &str,
    target_provider_id: &str,
    policy: Option<ActiveCompressionPolicy>,
    archive_dir: &std::path::Path,
) -> Result<(CanonicalSession, Option<ActiveCompressionReport>)> {
    let Some(mut policy) = policy else {
        return Ok((session.clone(), None));
    };
    policy.mode = ActiveCompressionMode::Auto;
    let applied = active_compression::apply_active_compression_with_archive_dir(
        session,
        ActiveCompressionApplyParams {
            source_provider_id: source_provider_id.to_string(),
            target_provider_id: target_provider_id.to_string(),
            policy,
            candidate_ids: Vec::new(),
        },
        archive_dir,
    )?;
    Ok((applied.session, Some(applied.report)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindParams {
    pub dir: Option<String>,
    pub session: Option<String>,
    pub providers: Vec<String>,
}

pub fn find_sessions(params: &FindParams) -> Result<Vec<SessionGroup>> {
    let provider_ids = resolve_providers(&params.providers);
    let explicit_provider_filter = !params.providers.is_empty();
    let session_states = session_state::load_state_store().unwrap_or_default();
    let mut groups = Vec::new();

    for pid in &provider_ids {
        let prov = match providers::find_provider(pid) {
            Some(p) => p,
            None => continue,
        };
        let capabilities = prov.capabilities();
        if !capabilities.scan {
            continue;
        }
        let Some(sessions) = scan_sessions_for_aggregate(prov.as_ref(), explicit_provider_filter)?
        else {
            continue;
        };
        let filtered: Vec<SessionItem> = sessions
            .iter()
            .map(|s| {
                let mut item = SessionItem::from((s, pid.as_str()));
                let state = resolve_session_state(
                    pid,
                    &s.session_id,
                    s.title.clone(),
                    s.project_dir.as_deref(),
                    &session_states,
                );
                apply_session_item_state(&mut item, &state);
                item
            })
            .filter(|s| {
                let dir_match = params.dir.as_ref().map_or(true, |d| {
                    s.project_dir
                        .as_ref()
                        .map(|pd| pd.contains(d.as_str()))
                        .unwrap_or(false)
                });
                let session_match = params.session.as_ref().map_or(true, |pat| {
                    s.session_id.contains(pat.as_str())
                        || s.title
                            .as_ref()
                            .map(|t| t.contains(pat.as_str()))
                            .unwrap_or(false)
                        || s.native_title
                            .as_ref()
                            .map(|t| t.contains(pat.as_str()))
                            .unwrap_or(false)
                });
                dir_match && session_match
            })
            .collect();

        if !filtered.is_empty() {
            groups.push(SessionGroup {
                provider_id: pid.clone(),
                provider_name: prov.name().to_string(),
                sessions: filtered,
            });
        }
    }

    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{
        CanonicalSchema, EventBlock, EventLinks, EventMetadata, EventRole, EventSource,
        MappingDirection, MappingDisposition, MappingReport, ProviderSessionRef, SessionContext,
        SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
    };
    use crate::storage::session_state::SessionStateStore;
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::io::Write;
    use tempfile::Builder;

    struct FailingProvider;

    impl provider::Provider for FailingProvider {
        fn id(&self) -> &'static str {
            "failing"
        }

        fn name(&self) -> &'static str {
            "Failing"
        }

        fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
            anyhow::bail!("scan failed")
        }

        fn import_session(&self, _source_path: &str) -> Result<ImportedSession> {
            anyhow::bail!("unused")
        }
    }

    #[test]
    fn aggregate_scan_skips_provider_error_without_explicit_filter() {
        let sessions = scan_sessions_for_aggregate(&FailingProvider, false).unwrap();

        assert!(sessions.is_none());
    }

    #[test]
    fn aggregate_scan_keeps_provider_error_with_explicit_filter() {
        let error = scan_sessions_for_aggregate(&FailingProvider, true).unwrap_err();

        assert!(error.to_string().contains("scan failed"));
    }

    #[test]
    fn session_item_overlay_prefers_memorph_display_title() {
        let meta = ProviderSessionSummary {
            session_id: "session-1".to_string(),
            title: Some("Native".to_string()),
            project_dir: Some("/tmp/project".to_string()),
            last_active_at: Some(42),
            source_path: Some("/tmp/session.jsonl".to_string()),
        };
        let mut item = SessionItem::from((&meta, "codex"));
        let mut session_states = SessionStateStore::default();
        session_state::set_display_title_in_store(
            &mut session_states,
            "codex",
            "session-1",
            "Display",
        );

        let state = resolve_session_state(
            "codex",
            "session-1",
            meta.title.clone(),
            meta.project_dir.as_deref(),
            &session_states,
        );
        apply_session_item_state(&mut item, &state);

        assert_eq!(item.native_title.as_deref(), Some("Native"));
        assert_eq!(item.display_title.as_deref(), Some("Display"));
        assert_eq!(item.title.as_deref(), Some("Display"));
    }

    #[test]
    fn session_detail_view_prefers_local_display_title_and_counts_non_lifecycle_messages() {
        let imported = ImportedSession {
            session: crate::canonical::CanonicalSession {
                schema: CanonicalSchema::default(),
                identity: SessionIdentity {
                    canonical_id: "canonical-1".to_string(),
                    source_title: Some("Native".to_string()),
                },
                provenance: SessionProvenance {
                    imported_at: Utc::now(),
                    imported_by: Some("test".to_string()),
                    primary_source: ProviderSessionRef {
                        provider_id: "codex".to_string(),
                        session_id: "session-1".to_string(),
                        source_path: Some("/tmp/session.jsonl".to_string()),
                    },
                    aliases: Vec::new(),
                },
                context: SessionContext {
                    workspace_dir: Some("/tmp/project".to_string()),
                    created_at: Some(Utc::now()),
                    last_active_at: Some(Utc::now()),
                    tags: Vec::new(),
                },
                events: vec![
                    SessionEvent {
                        id: "e1".to_string(),
                        kind: SessionEventKind::Lifecycle,
                        role: EventRole::System,
                        timestamp: Utc::now(),
                        links: EventLinks::default(),
                        blocks: vec![EventBlock::Text {
                            text: "started".to_string(),
                        }],
                        metadata: EventMetadata {
                            source: EventSource {
                                provider_id: "codex".to_string(),
                                original_id: None,
                                original_role: None,
                                phase: None,
                            },
                            model: None,
                            usage: None,
                            fidelity: MappingDisposition::Preserved,
                            provider_ext: BTreeMap::new(),
                        },
                    },
                    SessionEvent {
                        id: "e2".to_string(),
                        kind: SessionEventKind::Message,
                        role: EventRole::Assistant,
                        timestamp: Utc::now(),
                        links: EventLinks::default(),
                        blocks: vec![EventBlock::Text {
                            text: "hello".to_string(),
                        }],
                        metadata: EventMetadata {
                            source: EventSource {
                                provider_id: "codex".to_string(),
                                original_id: None,
                                original_role: None,
                                phase: None,
                            },
                            model: None,
                            usage: None,
                            fidelity: MappingDisposition::Preserved,
                            provider_ext: BTreeMap::new(),
                        },
                    },
                ],
                artifacts: Vec::new(),
                extensions: BTreeMap::new(),
            },
            report: MappingReport::new("codex", MappingDirection::Import),
        };

        let view = build_session_detail_view(
            "codex",
            "codex",
            "session-1",
            Some("/tmp/session.jsonl".to_string()),
            Some("codex resume session-1".to_string()),
            Some("Native".to_string()),
            session_state::ResolvedLocalSessionState {
                display_title: Some("Display".to_string()),
                archived: false,
                hidden: false,
                pinned: false,
                notes: None,
                tags: Vec::new(),
                preferred_targets: Vec::new(),
            },
            imported,
        );

        assert_eq!(view.title.as_deref(), Some("Display"));
        assert_eq!(view.native_title.as_deref(), Some("Native"));
        assert_eq!(view.display_title.as_deref(), Some("Display"));
        assert_eq!(view.event_count, 2);
        assert_eq!(view.message_count, 1);
        assert_eq!(view.source_path.as_deref(), Some("/tmp/session.jsonl"));
    }

    #[test]
    fn session_from_compression_archive_restores_source_events() {
        let now = Utc::now();
        let archive = compression::CompressionArchive {
            version: 1,
            created_at: now,
            canonical_id: "canonical-archive".to_string(),
            source_provider_id: "opencode".to_string(),
            target_provider_id: "codex".to_string(),
            summary_event_id: "summary-event".to_string(),
            source_event_ids: vec!["old-event".to_string()],
            events: vec![SessionEvent {
                id: "old-event".to_string(),
                kind: SessionEventKind::Message,
                role: EventRole::User,
                timestamp: now,
                links: EventLinks::default(),
                blocks: vec![EventBlock::Text {
                    text: "restored source context".to_string(),
                }],
                metadata: EventMetadata {
                    source: EventSource {
                        provider_id: "opencode".to_string(),
                        original_id: Some("old-event".to_string()),
                        original_role: None,
                        phase: None,
                    },
                    model: None,
                    usage: None,
                    fidelity: MappingDisposition::Preserved,
                    provider_ext: BTreeMap::new(),
                },
            }],
        };

        let session = session_management::session_from_compression_archive_for_tests(
            "memorph-archive://test/archive.json",
            archive,
        )
        .unwrap();

        assert_eq!(session.identity.canonical_id, "canonical-archive");
        assert_eq!(session.events.len(), 1);
        assert_eq!(session.provenance.primary_source.provider_id, "memorph");
        assert_eq!(
            session.provenance.primary_source.source_path.as_deref(),
            Some("memorph-archive://test/archive.json")
        );
        assert_eq!(session.context.tags, vec!["compression-archive"]);
        assert!(session.extensions.contains_key("compression_archive"));
    }

    #[test]
    fn list_compression_provider_support_marks_native_and_portable_providers() {
        let support = list_compression_provider_support();
        let opencode = support
            .iter()
            .find(|item| item.provider_id == "opencode")
            .expect("opencode support profile");
        assert_eq!(
            opencode.default_projection,
            compression::CompressionProjection::Native
        );
        assert!(opencode.detects_native_source);
        assert!(opencode.native_target_projection);

        let codex = support
            .iter()
            .find(|item| item.provider_id == "codex")
            .expect("codex support profile");
        assert_eq!(
            codex.default_projection,
            compression::CompressionProjection::Native
        );
        assert!(codex.detects_native_source);
        assert!(codex.native_target_projection);
    }

    #[test]
    fn compression_retrieval_tool_spec_is_machine_readable_and_query_first() {
        let spec = compression_retrieval_tool_spec();

        assert_eq!(spec.name, "memorph_retrieve_compression_archive");
        assert_eq!(spec.archive_ref_scheme, "memorph-archive://");
        assert_eq!(spec.api.method, "POST");
        assert_eq!(spec.api.path, "/api/v1/compression/retrieve");
        assert_eq!(
            spec.input_schema["required"],
            serde_json::json!(["archive_ref"])
        );
        assert!(spec.input_schema["properties"].get("query").is_some());
        assert!(spec
            .usage_rules
            .iter()
            .any(|rule| rule.contains("Do not expand")));
        assert!(spec
            .usage_rules
            .iter()
            .any(|rule| rule.contains("Prefer query retrieval")));
    }

    #[test]
    fn compression_retrieval_instructions_are_archive_specific_and_query_first() {
        let instructions =
            compression_retrieval_instructions("memorph-archive://session/archive.json.gz")
                .unwrap();

        assert_eq!(
            instructions.archive_ref,
            "memorph-archive://session/archive.json.gz"
        );
        assert!(instructions
            .query_first_cli
            .contains("--query <terms> --max-results 5"));
        assert_eq!(
            instructions.full_cli,
            "memorph compression retrieve memorph-archive://session/archive.json.gz"
        );
        assert_eq!(
            instructions.api_query_body["archive_ref"],
            "memorph-archive://session/archive.json.gz"
        );
        assert_eq!(instructions.api_query_body["max_results"], 5);
        assert!(instructions
            .suggested_steps
            .iter()
            .any(|step| step.contains("full retrieval only")));
    }

    #[test]
    fn compression_retrieval_instructions_reject_invalid_refs() {
        let error = compression_retrieval_instructions("not-an-archive-ref").unwrap_err();
        assert!(error
            .to_string()
            .contains("Unsupported compression archive ref"));
    }

    #[test]
    fn active_compression_dry_run_from_file_returns_candidates_and_skips() {
        let file = write_active_compression_source_file(&active_compression_source_session());

        let report = active_compression_dry_run(&ActiveCompressionDryRunParams {
            source_provider_id: "claude".to_string(),
            target_provider_id: "codex".to_string(),
            session_id: None,
            file: Some(file.path().to_string_lossy().to_string()),
            policy: active_compression::ActiveCompressionPolicy {
                protect_recent_message_events: 1,
                min_candidate_bytes: 16,
                min_savings_ratio_percent: 20,
                mode: active_compression::ActiveCompressionMode::PlanOnly,
            },
        })
        .unwrap();

        assert!(report.dry_run);
        assert_eq!(report.source_provider_id, "claude");
        assert_eq!(report.target_provider_id, "codex");
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].event_ids, vec!["old-user"]);
        assert!(report.candidates[0].estimated_bytes_saved > 0);
        assert!(matches!(
            report.candidates[0].reason,
            active_compression::CompressionSelectionReason::HistoricalContext
        ));
        assert!(matches!(
            report.candidates[0].risk,
            active_compression::CompressionRisk::Medium
        ));
        assert!(report.skipped.iter().any(|skipped| {
            skipped.event_id == "recent-user"
                && matches!(
                    skipped.reason,
                    active_compression::CompressionSkipReason::ProtectedRecentMessage
                )
        }));
    }

    #[test]
    fn active_compression_apply_from_file_writes_archive_and_expandable_output() {
        let archive_dir = tempfile::tempdir().unwrap();
        let output_dir = tempfile::tempdir().unwrap();
        let source = active_compression_source_session();
        let source_file = write_active_compression_source_file(&source);
        let output_prefix = output_dir
            .path()
            .join("compressed")
            .to_string_lossy()
            .to_string();

        let result = active_compression_apply_with_archive_dir(
            &ActiveCompressionApplyCommandParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                session_id: None,
                file: Some(source_file.path().to_string_lossy().to_string()),
                policy: active_compression::ActiveCompressionPolicy {
                    protect_recent_message_events: 1,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: active_compression::ActiveCompressionMode::Auto,
                },
                candidate_ids: vec!["candidate-0001".to_string()],
                output_prefix: Some(output_prefix),
                format: "json".to_string(),
            },
            archive_dir.path(),
        )
        .unwrap();

        assert_eq!(result.files.len(), 1);
        assert_eq!(result.archive_refs.len(), 1);
        assert_eq!(result.report.candidates.len(), 1);
        assert!(result.report.compressed_estimated_bytes < result.report.original_estimated_bytes);

        let compressed = session_management::read_session_export_file(&result.files[0]).unwrap();
        assert!(compressed.events.iter().any(|event| {
            event.blocks.iter().any(|block| {
                matches!(
                    block,
                    EventBlock::Compressed {
                        archive_ref: Some(archive_ref),
                        ..
                    } if archive_ref == &result.archive_refs[0]
                )
            })
        }));
        assert!(!compressed.events.iter().any(|event| {
            event.blocks.iter().any(|block| {
                matches!(
                    block,
                    EventBlock::Text { text }
                        if text.contains("historical context historical context historical context")
                )
            })
        }));

        let (expanded, expand_report) = compression::expand_compressed_segments_in_dir(
            &compressed,
            "claude",
            "codex",
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(expand_report.expanded_segments, 1);
        assert_eq!(expand_report.restored_events, 1);
        assert!(expanded.events.iter().any(|event| event.id == "old-user"));

        let retrieved = retrieve_compression_archive_in_dir(
            &RetrieveCompressionArchiveParams {
                archive_ref: result.archive_refs[0].clone(),
                query: None,
                max_results: None,
            },
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(retrieved.source_provider_id, "claude");
        assert_eq!(retrieved.target_provider_id, "codex");
        assert_eq!(retrieved.source_event_ids, vec!["old-user"]);
        assert_eq!(retrieved.source_event_count, 1);
        assert_eq!(retrieved.returned_event_count, 1);
        assert_eq!(
            retrieved.retrieval_mode,
            CompressionRetrievalMode::FullArchive
        );
        assert!(retrieved
            .recommended_next_action
            .contains("complete archived segment"));
        assert!(retrieved.events.iter().any(|event| event.id == "old-user"));

        let searched = retrieve_compression_archive_in_dir(
            &RetrieveCompressionArchiveParams {
                archive_ref: result.archive_refs[0].clone(),
                query: Some("historical context".to_string()),
                max_results: Some(5),
            },
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(searched.query.as_deref(), Some("historical context"));
        assert_eq!(searched.source_event_count, 1);
        assert_eq!(searched.returned_event_count, 1);
        assert_eq!(
            searched.retrieval_mode,
            CompressionRetrievalMode::QueryMatches
        );
        assert!(searched
            .recommended_next_action
            .contains("query-filtered partial retrieval"));
        assert_eq!(searched.events[0].id, "old-user");
        assert_eq!(searched.matches.len(), 1);
        assert_eq!(searched.matches[0].event_id, "old-user");
        assert!(searched.matches[0]
            .snippets
            .iter()
            .any(|snippet| snippet.contains("historical context")));

        let no_match = retrieve_compression_archive_in_dir(
            &RetrieveCompressionArchiveParams {
                archive_ref: result.archive_refs[0].clone(),
                query: Some("not present".to_string()),
                max_results: Some(5),
            },
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(no_match.source_event_count, 1);
        assert_eq!(no_match.returned_event_count, 0);
        assert_eq!(
            no_match.retrieval_mode,
            CompressionRetrievalMode::QueryNoMatches
        );
        assert!(no_match
            .recommended_next_action
            .contains("Try a broader query"));
        assert!(no_match.events.is_empty());
        assert!(no_match.matches.is_empty());
    }

    #[test]
    fn switch_params_omit_active_compression_by_default() {
        let params = SwitchParams {
            from: "claude".to_string(),
            to: "codex".to_string(),
            session_id: None,
            to_dir: None,
            active_compression: None,
        };

        let value = serde_json::to_value(params).unwrap();

        assert!(value.get("active_compression").is_none());
    }

    #[test]
    fn switch_active_compression_is_opt_in_and_archive_backed() {
        let archive_dir = tempfile::tempdir().unwrap();
        let source = active_compression_source_session();

        let (unchanged, report) = apply_switch_active_compression_with_archive_dir(
            &source,
            "claude",
            "codex",
            None,
            archive_dir.path(),
        )
        .unwrap();
        assert!(report.is_none());
        assert_eq!(
            serde_json::to_value(&unchanged.events).unwrap(),
            serde_json::to_value(&source.events).unwrap()
        );

        let (compressed, report) = apply_switch_active_compression_with_archive_dir(
            &source,
            "claude",
            "codex",
            Some(active_compression::ActiveCompressionPolicy {
                protect_recent_message_events: 1,
                min_candidate_bytes: 16,
                min_savings_ratio_percent: 20,
                mode: active_compression::ActiveCompressionMode::PlanOnly,
            }),
            archive_dir.path(),
        )
        .unwrap();
        let report = report.expect("switch active compression report");

        assert!(!report.dry_run);
        assert_eq!(report.source_provider_id, "claude");
        assert_eq!(report.target_provider_id, "codex");
        assert_eq!(
            report.policy.mode,
            active_compression::ActiveCompressionMode::Auto
        );
        assert_eq!(report.archive_refs.len(), 1);
        assert!(compressed.events.iter().any(|event| {
            event.blocks.iter().any(|block| {
                matches!(
                    block,
                    EventBlock::Compressed {
                        archive_ref: Some(archive_ref),
                        ..
                    } if archive_ref == &report.archive_refs[0]
                )
            })
        }));
        assert!(!compressed.events.iter().any(|event| event.id == "old-user"));
        assert!(compressed
            .events
            .iter()
            .any(|event| event.id == "recent-user"));
    }

    fn active_compression_source_session() -> CanonicalSession {
        let now = Utc::now();
        CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "dry-run-file".to_string(),
                source_title: Some("Dry Run File".to_string()),
            },
            provenance: SessionProvenance {
                imported_at: now,
                imported_by: None,
                primary_source: ProviderSessionRef {
                    provider_id: "claude".to_string(),
                    session_id: "dry-run-file".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext::default(),
            events: vec![
                SessionEvent {
                    id: "old-user".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: now,
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "historical context ".repeat(80),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: "claude".to_string(),
                            original_id: Some("old-user".to_string()),
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
                SessionEvent {
                    id: "recent-user".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: now,
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "latest active request".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: "claude".to_string(),
                            original_id: Some("recent-user".to_string()),
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        }
    }

    fn write_active_compression_source_file(session: &CanonicalSession) -> tempfile::NamedTempFile {
        let mut file = Builder::new().suffix(".json").tempfile().unwrap();
        write!(file, "{}", serde_json::to_string(session).unwrap()).unwrap();
        file
    }
}
