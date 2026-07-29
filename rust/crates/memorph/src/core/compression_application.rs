use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpandCompressionSessionParams {
    pub file: String,
    pub output_prefix: Option<String>,
    pub format: String,
}

pub fn expand_compression_session(
    params: &ExpandCompressionSessionParams,
    actor: ActivityActor,
) -> Result<ExportResult> {
    let mut conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "source_file": params.file,
        "format": params.format,
        "output_prefix": params.output_prefix,
    });
    let activity_id = ActivityStore::new(&conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Compress,
        actor,
        summary: "Expanding compressed session".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let session = session_management::read_session_export_file(&params.file)?;
        let provider_id = session.provenance.primary_source.provider_id.trim();
        let provider_id = if provider_id.is_empty() {
            "memorph".to_string()
        } else {
            provider_id.to_string()
        };
        let provider_session_id = session.provenance.primary_source.session_id.trim();
        let provider_session_id = if provider_session_id.is_empty() {
            session.session.identity.id.clone()
        } else {
            provider_session_id.to_string()
        };
        let export = session_management::expand_compression_session(params, &session)?;
        let artifacts = register_session_export_artifacts(
            &mut conn,
            &activity_id,
            &provider_id,
            &provider_session_id,
            &params.format,
            &export,
        )?;
        Ok((export, artifacts, provider_id, provider_session_id))
    })();
    match result {
        Ok((export, artifacts, provider_id, provider_session_id)) => {
            let mut completion = ActivityCompletion::success(
                "Expanded compressed session",
                serde_json::json!({
                    "source_file": params.file,
                    "format": params.format,
                    "files": export.files,
                    "artifact_ids": artifacts.iter().map(|artifact| artifact.id.clone()).collect::<Vec<_>>(),
                }),
            );
            completion.provider_id = Some(provider_id);
            completion.provider_session_id = Some(provider_session_id);
            ActivityStore::new(&conn).finish(&activity_id, completion)?;
            Ok(export)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to expand compressed session",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreNativeCompressionParams {
    pub provider_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreNativeCompressionResult {
    pub restored_segments: usize,
    pub restored_events: usize,
    pub remaining_archive_refs: Vec<String>,
    pub source_bytes_before: u64,
    pub source_bytes_after: u64,
}

pub fn restore_native_compression(
    params: &RestoreNativeCompressionParams,
    actor: ActivityActor,
) -> Result<RestoreNativeCompressionResult> {
    let mut conn = local_store::open_database()?;
    let details = serde_json::to_value(params)?;
    let activity_id = ActivityStore::new(&conn).start(NewActivity {
        provider_id: Some(params.provider_id.clone()),
        provider_session_id: Some(params.session_id.clone()),
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Compress,
        actor,
        summary: "Restoring compressed segments in native session".to_string(),
        details: details.clone(),
    })?;
    let result = (|| {
        let session =
            sessions::get_canonical_session(&params.provider_id, &params.session_id)?.session;
        let (restored, report) = compression::restore_compressed_segments_in_place(
            &session,
            params.archive_ref.as_deref(),
        )?;
        if report.expanded_segments == 0 {
            anyhow::bail!("Session has no restorable compressed segments");
        }
        let remaining_archive_refs = compression::compressed_archive_refs(&restored);
        let backup_root = crate::config::memorph_dir()?
            .join("artifacts")
            .join("backups");
        let replaced = session_management::replace_native_session(
            &params.provider_id,
            &params.session_id,
            &restored,
            &remaining_archive_refs,
            &activity_id,
            &backup_root,
            &mut conn,
        )?;
        session_state::update_session_state(
            &params.provider_id,
            &params.session_id,
            &session_state::SessionLocalStateUpdate {
                compressed_archive_refs: Some(remaining_archive_refs.clone()),
                ..Default::default()
            },
        )?;
        transfer::refresh_target_provider_sessions(&params.provider_id)?;
        Ok(RestoreNativeCompressionResult {
            restored_segments: report.expanded_segments,
            restored_events: report.restored_events,
            remaining_archive_refs,
            source_bytes_before: replaced.source_bytes_before,
            source_bytes_after: replaced.source_bytes_after,
        })
    })();
    match result {
        Ok(restored) => {
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Restored compressed segments in native session",
                    serde_json::to_value(&restored)?,
                ),
            )?;
            Ok(restored)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to restore compressed segments in native session",
                    details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreCompressionArchiveParams {
    pub archive_ref: String,
    pub output_prefix: Option<String>,
    pub format: String,
}

pub fn restore_compression_archive(
    params: &RestoreCompressionArchiveParams,
    actor: ActivityActor,
) -> Result<ExportResult> {
    let mut conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "archive_ref": params.archive_ref,
        "format": params.format,
        "output_prefix": params.output_prefix,
    });
    let activity_id = ActivityStore::new(&conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Compress,
        actor,
        summary: "Restoring compression archive".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let archive = compression::load_archive(&params.archive_ref)?;
        let provider_id = archive.source_provider_id.trim();
        let provider_id = if provider_id.is_empty() {
            "memorph".to_string()
        } else {
            provider_id.to_string()
        };
        let session =
            session_management::session_from_compression_archive(&params.archive_ref, archive)?;
        let provider_session_id = session.identity.id.clone();
        let export = session_management::restore_compression_archive(params, &session)?;
        let artifacts = register_session_export_artifacts(
            &mut conn,
            &activity_id,
            &provider_id,
            &provider_session_id,
            &params.format,
            &export,
        )?;
        Ok((export, artifacts, provider_id, provider_session_id))
    })();
    match result {
        Ok((export, artifacts, provider_id, provider_session_id)) => {
            let mut completion = ActivityCompletion::success(
                "Restored compression archive",
                serde_json::json!({
                    "archive_ref": params.archive_ref,
                    "format": params.format,
                    "files": export.files,
                    "artifact_ids": artifacts.iter().map(|artifact| artifact.id.clone()).collect::<Vec<_>>(),
                }),
            );
            completion.provider_id = Some(provider_id);
            completion.provider_session_id = Some(provider_session_id);
            ActivityStore::new(&conn).finish(&activity_id, completion)?;
            Ok(export)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to restore compression archive",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
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
    pub returned_event_ids: Vec<String>,
    pub returned_event_count: usize,
    pub omitted_event_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<RetrievedCompressionArchiveMatch>,
    pub events: Vec<Event>,
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
                "events contains every original archived Event".to_string(),
                "source_event_count equals the archive's full original event count".to_string(),
                "returned_event_ids contains every returned event id".to_string(),
                "returned_event_count equals events.length".to_string(),
                "omitted_event_count is 0".to_string(),
            ],
            query_retrieval: vec![
                "retrieval_mode is query_matches or query_no_matches".to_string(),
                "events contains only matching archived Event values".to_string(),
                "returned_event_ids contains only matching event ids".to_string(),
                "omitted_event_count reports archived events not returned by the query".to_string(),
                "matches contains event_id, event_index, score, and snippets for each returned event".to_string(),
                "source_event_count still reports the archive's full original event count".to_string(),
            ],
        },
        usage_rules: vec![
            "Do not expand a compressed archive unconditionally when switching or continuing a session.".to_string(),
            "Prefer query retrieval before full retrieval to avoid putting large archived history back into context.".to_string(),
            "Query scores prioritize exact phrase matches, then coverage of distinct query terms, with repeated single-term hits treated as weak evidence.".to_string(),
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
            "When multiple matches are returned, prefer higher scores; scoring favors exact phrases and broader term coverage over repeated single-term noise.".to_string(),
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
pub(super) fn retrieve_compression_archive_in_dir(
    params: &RetrieveCompressionArchiveParams,
    archive_dir: &std::path::Path,
) -> Result<RetrievedCompressionArchive> {
    let archive = compression::load_archive_from_dir(archive_dir, &params.archive_ref)?;
    Ok(retrieved_compression_archive(params, archive))
}

pub(super) fn retrieved_compression_archive(
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
    let returned_event_ids = events
        .iter()
        .map(|event| event.id.clone())
        .collect::<Vec<_>>();
    let omitted_event_count = source_event_count.saturating_sub(returned_event_count);
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
        returned_event_ids,
        returned_event_count,
        omitted_event_count,
        matches,
        events,
    }
}

pub(super) fn retrieval_next_action(mode: CompressionRetrievalMode) -> String {
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

pub(super) fn search_archive_events(
    events: &[Event],
    query: &str,
    max_results: usize,
) -> (Vec<Event>, Vec<RetrievedCompressionArchiveMatch>) {
    if max_results == 0 {
        return (Vec::new(), Vec::new());
    }
    let query_lower = query.to_ascii_lowercase();
    let mut terms = Vec::new();
    for term in query_lower
        .split_whitespace()
        .filter(|term| !term.is_empty())
    {
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    if terms.is_empty() {
        return (events.to_vec(), Vec::new());
    }

    let mut ranked = events
        .iter()
        .enumerate()
        .filter_map(|(event_index, event)| {
            let text = provider::canonical_event_text(event);
            let text_lower = text.to_ascii_lowercase();
            let score = archive_query_score(&text_lower, &query_lower, &terms);
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

pub(super) fn archive_query_score(text_lower: &str, query_lower: &str, terms: &[&str]) -> usize {
    let mut matched_terms = 0;
    let mut capped_occurrences = 0;
    for term in terms {
        let count = text_lower.matches(term).count();
        if count > 0 {
            matched_terms += 1;
            capped_occurrences += count.min(3);
        }
    }
    if matched_terms == 0 {
        return 0;
    }

    let mut score = matched_terms * 20 + capped_occurrences;
    if matched_terms == terms.len() {
        score += 50;
    }
    if text_lower.contains(query_lower) {
        score += 100;
    }
    score
}

pub(super) fn archive_search_snippets(
    text: &str,
    query_lower: &str,
    terms: &[&str],
) -> Vec<String> {
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

pub(super) fn truncate_search_snippet(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

pub fn list_compression_archives(
    workspace: Option<&str>,
) -> Result<Vec<compression::CompressionArchiveSummary>> {
    session_management::list_compression_archives(workspace)
}

pub fn get_compression_archive(archive_ref: &str) -> Result<compression::CompressionArchive> {
    compression::load_archive(archive_ref)
}

pub fn list_compression_provider_support() -> Vec<crate::provider::ProviderCompressionSupport> {
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
    pub source_bytes_before: u64,
    pub source_bytes_after: u64,
}

pub fn active_compression_apply(
    params: &ActiveCompressionApplyCommandParams,
    actor: ActivityActor,
) -> Result<ActiveCompressionApplyCommandResult> {
    let mut activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "provider_session_id": params.session_id,
        "source_file": params.file,
        "source_provider_id": params.source_provider_id,
        "target_provider_id": params.target_provider_id,
        "candidate_ids": params.candidate_ids,
        "format": params.format,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(params.source_provider_id.clone()),
        provider_session_id: params.session_id.clone(),
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Compress,
        actor,
        summary: "Applying active session compression".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        if params.session_id.is_none() || params.file.is_some() {
            anyhow::bail!("Native compression requires session_id and does not accept file");
        }
        if params.source_provider_id != params.target_provider_id {
            anyhow::bail!("Native compression target must match the source provider");
        }
        let session = load_active_compression_source_session(
            &params.source_provider_id,
            params.session_id.as_deref(),
            params.file.as_deref(),
        )?;
        let archive_dir = compression::archive_base_dir()?;
        let applied = apply_active_compression_to_session(params, &session, archive_dir.as_path())?;
        let artifacts = register_active_compression_archive_artifacts(
            &mut activity_conn,
            &activity_id,
            params,
            &session,
            archive_dir.as_path(),
            &applied.report.archive_refs,
        )?;
        let result = write_active_compression_application(
            params,
            applied,
            &activity_id,
            &mut activity_conn,
        )?;
        Ok((
            result,
            artifacts
                .into_iter()
                .map(|artifact| artifact.id)
                .collect::<Vec<_>>(),
        ))
    })();
    match result {
        Ok((applied, artifact_ids)) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Applied active session compression",
                    serde_json::json!({
                        "provider_session_id": params.session_id,
                        "source_provider_id": params.source_provider_id,
                        "target_provider_id": params.target_provider_id,
                        "files": applied.files,
                        "archive_refs": applied.archive_refs,
                        "artifact_ids": artifact_ids,
                        "candidate_count": applied.report.candidates.len(),
                        "source_bytes_before": applied.source_bytes_before,
                        "source_bytes_after": applied.source_bytes_after,
                    }),
                ),
            )?;
            Ok(applied)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to apply active session compression",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

pub(super) fn apply_active_compression_to_session(
    params: &ActiveCompressionApplyCommandParams,
    session: &Session,
    archive_dir: &std::path::Path,
) -> Result<active_compression::ActiveCompressionApplyResult> {
    let apply_params = ActiveCompressionApplyParams {
        source_provider_id: params.source_provider_id.clone(),
        target_provider_id: params.target_provider_id.clone(),
        policy: params.policy.clone(),
        candidate_ids: params.candidate_ids.clone(),
    };
    active_compression::apply_active_compression_with_archive_dir(
        session,
        apply_params,
        archive_dir,
    )
}

pub(super) fn write_active_compression_application(
    params: &ActiveCompressionApplyCommandParams,
    applied: active_compression::ActiveCompressionApplyResult,
    operation_id: &str,
    artifact_conn: &mut rusqlite::Connection,
) -> Result<ActiveCompressionApplyCommandResult> {
    let session_id = params
        .session_id
        .as_deref()
        .context("Native compression requires session_id")?;
    let backup_root = crate::config::memorph_dir()?
        .join("artifacts")
        .join("backups");
    let replaced = session_management::replace_native_session(
        &params.source_provider_id,
        session_id,
        &applied.session,
        &applied.report.archive_refs,
        operation_id,
        &backup_root,
        artifact_conn,
    )?;
    session_state::update_session_state(
        &params.source_provider_id,
        session_id,
        &session_state::SessionLocalStateUpdate {
            compressed_archive_refs: Some(applied.report.archive_refs.clone()),
            ..Default::default()
        },
    )?;
    transfer::refresh_target_provider_sessions(&params.source_provider_id)?;

    Ok(ActiveCompressionApplyCommandResult {
        files: Vec::new(),
        archive_refs: applied.report.archive_refs.clone(),
        report: applied.report,
        source_bytes_before: replaced.source_bytes_before,
        source_bytes_after: replaced.source_bytes_after,
    })
}

pub(super) fn register_active_compression_archive_artifacts(
    conn: &mut rusqlite::Connection,
    operation_id: &str,
    params: &ActiveCompressionApplyCommandParams,
    session: &Session,
    archive_dir: &std::path::Path,
    archive_refs: &[String],
) -> Result<Vec<crate::storage::artifact_store::ArtifactManifest>> {
    let manifests = archive_refs
        .iter()
        .map(|archive_ref| {
            Ok(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::CompressionArchive,
                operation_id: Some(operation_id.to_string()),
                provider_id: Some(params.source_provider_id.clone()),
                provider_session_id: Some(
                    params
                        .session_id
                        .clone()
                        .unwrap_or_else(|| session.identity.id.clone()),
                ),
                session_id: None,
                projection_report_id: None,
                path: compression::archive_path_from_ref_in_dir(archive_dir, archive_ref)?,
                mime_type: Some("application/gzip".to_string()),
                format: Some("json.gz".to_string()),
                metadata: serde_json::json!({
                    "role": "active_compression_recovery_archive",
                    "archive_ref": archive_ref,
                    "canonical_id": session.identity.id,
                    "source_provider_id": params.source_provider_id,
                    "target_provider_id": params.target_provider_id,
                }),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ArtifactStore::new(conn).register_paths(manifests)
}

pub(super) fn load_active_compression_source_session(
    source_provider_id: &str,
    session_id: Option<&str>,
    file: Option<&str>,
) -> Result<Session> {
    match (session_id, file) {
        (Some(_), Some(_)) => anyhow::bail!("Use either session_id or file, not both"),
        (Some(session_id), None) => {
            Ok(sessions::get_canonical_session(source_provider_id, session_id)?.session)
        }
        (None, Some(file)) => Ok(session_management::read_session_export_file(file)?.session),
        (None, None) => anyhow::bail!("Either session_id or file is required"),
    }
}
