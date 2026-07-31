use super::*;

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
        .get_session_meta(session_id)?
        .with_context(|| format!("Session not found: {}", session_id))?;

    load_canonical_session_from_meta(prov.as_ref(), provider_id, meta)
}

pub fn get_session_detail_view(provider_id: &str, session_id: &str) -> Result<SessionDetailView> {
    Ok(get_session_detail_view_page_result(
        provider_id,
        session_id,
        0,
        None,
        None,
        SessionEventOrder::Asc,
    )?
    .view)
}

#[derive(Debug, Clone)]
pub struct SessionDetailPageResult {
    pub view: SessionDetailView,
    pub returned_event_indices: Vec<usize>,
    pub matched_event_count: Option<usize>,
}

pub fn get_session_detail_view_page(
    provider_id: &str,
    session_id: &str,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<SessionDetailView> {
    Ok(get_session_detail_view_page_result(
        provider_id,
        session_id,
        event_offset,
        event_limit,
        None,
        SessionEventOrder::Asc,
    )?
    .view)
}

pub fn get_session_detail_view_page_result(
    provider_id: &str,
    session_id: &str,
    event_offset: usize,
    event_limit: Option<usize>,
    event_search: Option<&str>,
    event_order: SessionEventOrder,
) -> Result<SessionDetailPageResult> {
    let search_query = event_search
        .map(str::trim)
        .filter(|query| !query.is_empty());
    let reverse = matches!(event_order, SessionEventOrder::Desc);
    let mut conn = crate::storage::local_store::open_database()?;
    let identity = match crate::storage::snapshot_store::SnapshotStore::new(&conn)
        .find_session_identity(provider_id, session_id)?
    {
        Some(identity) => identity,
        None => {
            // On-demand single-session indexing. Avoids a full provider scan on
            // the read path. If the provider cannot resolve the session by id,
            // index_single_session returns false and we surface the original
            // "not indexed" error so the caller can map it to a 404.
            if !crate::core::projection::index_single_session(provider_id, session_id)? {
                anyhow::bail!("Session is not indexed: {provider_id}/{session_id}");
            }
            crate::storage::snapshot_store::SnapshotStore::new(&conn)
                .find_session_identity(provider_id, session_id)?
                .with_context(|| {
                    format!("Session is not indexed: {provider_id}/{session_id}")
                })?
        }
    };
    let source_path = identity
        .source_path
        .as_deref()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Session has no source locator: {provider_id}/{session_id}"))?;
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    let source_fingerprint = provider
        .session_source_fingerprint(source_path)?
        .with_context(|| format!("Session source is missing: {source_path}"))?;
    if !provider.capabilities().import {
        anyhow::bail!("Provider does not support session detail reads: {provider_id}");
    }
    let provider_session_id = identity
        .provider_session_id
        .as_deref()
        .unwrap_or(session_id)
        .to_string();

    let (mut page, events, returned_event_indices, matched_event_count, full_events) =
        if let Some(query) = search_query {
            let page = provider.import_session_page(source_path, 0, None)?;
            let full_events = page.imported.session.events.clone();
            let mut matching_indices =
                session_event_search::find_matching_event_indices(&full_events, query);
            if reverse {
                matching_indices.reverse();
            }
            let matched_count = matching_indices.len();
            let offset = event_offset.min(matched_count);
            let limit = event_limit.unwrap_or(matched_count);
            let indices = matching_indices
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect::<Vec<_>>();
            let events = indices
                .iter()
                .map(|&index| full_events[index].clone())
                .collect::<Vec<_>>();
            (page, events, indices, Some(matched_count), full_events)
        } else if reverse {
            let count_page = provider.import_session_page(source_path, 0, Some(0))?;
            let total = count_page.event_count;
            let available = total.saturating_sub(event_offset);
            let take = event_limit.unwrap_or(available).min(available);
            let chrono_offset = total.saturating_sub(event_offset + take);
            let mut page = provider.import_session_page(source_path, chrono_offset, Some(take))?;
            page.event_count = count_page.event_count;
            page.message_count = count_page.message_count;
            if count_page.turn_count.is_some() {
                page.turn_count = count_page.turn_count;
            }
            let mut events = std::mem::take(&mut page.imported.session.events);
            events.reverse();
            let indices = (chrono_offset..chrono_offset + events.len())
                .rev()
                .collect::<Vec<_>>();
            (page, events, indices, None, Vec::new())
        } else {
            let page = provider.import_session_page(source_path, event_offset, event_limit)?;
            let events = page.imported.session.events.clone();
            (page, events.clone(), Vec::new(), None, events)
        };
    page.imported.session.events = events;
    let meta = ProviderSessionSummary {
        session_id: provider_session_id.clone(),
        title: identity.title.clone(),
        project_dir: identity.workspace_dir.clone(),
        created_at: None,
        last_active_at: identity.last_active_at_ms,
        source_path: Some(source_path.to_string()),
    };
    enrich_imported_session_from_meta(&mut page.imported, provider_id, &meta);
    page.turns = crate::session_projection::project_session_turns(
        &identity.canonical_session_id,
        &page.imported.session.events,
        provider.capabilities().turn_quality,
    );
    for turn in &mut page.turns {
        turn.session_id = identity.canonical_session_id.clone();
        turn.id = format!(
            "turn_{:x}",
            md5::compute(
                format!(
                    "{}\0{}",
                    identity.canonical_session_id,
                    turn.provider_turn_id
                        .as_deref()
                        .map(|value| format!("provider:{value}"))
                        .unwrap_or_else(|| turn.turn_order.to_string())
                )
                .as_bytes()
            )
        );
    }
    let local_state_store = session_state::load_state_store()?;
    let local_state = session_state::resolve_session_state(
        &local_state_store,
        provider_id,
        &provider_session_id,
        identity.workspace_dir.as_deref(),
    );
    let stale =
        identity.stale || identity.source_fingerprint.as_deref() != Some(&source_fingerprint.value);
    if let Some(turn_count) = page.turn_count {
        crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .record_complete_counts(
                &identity.canonical_session_id,
                &source_fingerprint.value,
                page.event_count,
                page.message_count,
                turn_count,
            )?;
    }
    let display_title = local_state
        .display_title
        .clone()
        .or_else(|| identity.display_title.clone());
    let title = display_title.clone().or_else(|| identity.title.clone());
    let last_active_at = page
        .imported
        .session
        .context
        .last_active_at
        .filter(utils::is_plausible_session_time)
        .or_else(|| {
            identity
                .last_active_at_ms
                .and_then(utils::datetime_from_timestamp_ms)
        });
    let created_at = resolve_session_created_at(
        page.imported.session.context.created_at,
        &page.imported.session.events,
    );
    let compressed_archive_refs = compression::compressed_archive_refs(&page.imported.session);
    let mut metrics_session = page.imported.session.clone();
    if !full_events.is_empty() {
        metrics_session.events = full_events;
    }
    let length_metrics = session_length_metrics(
        provider.session_size(&provider_session_id)?,
        &metrics_session,
        page.event_count,
        page.message_count,
        page.turn_count.unwrap_or(page.turns.len()),
    )?;

    Ok(SessionDetailPageResult {
        view: SessionDetailView {
            provider_id: provider_id.to_string(),
            provider_name: provider.name().to_string(),
            session_id: provider_session_id,
            canonical_id: identity.canonical_session_id.clone(),
            title,
            native_title: identity.title,
            display_title,
            workspace_dir: page
                .imported
                .session
                .context
                .workspace
                .as_deref()
                .map(utils::user_visible_path),
            created_at,
            last_active_at,
            source_path: Some(utils::user_visible_path(source_path)),
            resume_command: provider.resume_command(
                identity
                    .provider_session_id
                    .as_deref()
                    .unwrap_or(session_id),
            ),
            local_state: local_state.clone(),
            event_count: page.event_count,
            message_count: page.message_count,
            length_metrics,
            stale,
            projection_report: Some(source_mapping_report_view(
                provider_id,
                identity.source_id.as_deref(),
                &page.imported,
                page.event_count,
            )),
            turns: page.turns,
            events: page.imported.session.events,
            compressed_archive_refs,
        },
        returned_event_indices,
        matched_event_count,
    })
}

pub(super) fn session_length_metrics(
    provider_source_bytes: u64,
    session: &Session,
    event_count: usize,
    message_count: usize,
    turn_count: usize,
) -> Result<SessionLengthMetrics> {
    let model_visible_bytes = serde_json::to_vec(&session.events)?.len() as u64;
    let archive_count = compression::compressed_archive_refs(session).len();
    Ok(SessionLengthMetrics {
        provider_source_bytes_measured: provider_source_bytes,
        model_visible_bytes_measured: model_visible_bytes,
        estimated_tokens: model_visible_bytes.div_ceil(4),
        event_count,
        message_count,
        turn_count,
        compressed_segment_count: archive_count,
        archive_count,
    })
}

fn source_mapping_report_view(
    provider_id: &str,
    source_id: Option<&str>,
    imported: &ImportedSession,
    event_count: usize,
) -> SessionProjectionReportView {
    let mut preserved_count = 0;
    let mut normalized_count = 0;
    let mut dropped_count = 0;
    for disposition in imported
        .event_meta
        .iter()
        .map(|meta| meta.fidelity)
        .chain(imported.report.issues.iter().map(|issue| issue.disposition))
    {
        match disposition {
            crate::session::Fidelity::Preserved => preserved_count += 1,
            crate::session::Fidelity::Normalized | crate::session::Fidelity::Downgraded => {
                normalized_count += 1
            }
            crate::session::Fidelity::Dropped | crate::session::Fidelity::Unsupported => {
                dropped_count += 1
            }
        }
    }
    SessionProjectionReportView {
        id: format!(
            "source-read:{provider_id}:{}",
            imported.session.identity.id
        ),
        provider_id: provider_id.to_string(),
        source_id: source_id.map(str::to_string),
        operation_kind: crate::session_projection::ProjectionOperationKind::Import,
        projection_version: crate::session_projection::SESSION_PROJECTION_VERSION,
        status: if dropped_count > 0 {
            crate::session_projection::ProjectionStatus::CompletedWithLoss
        } else {
            crate::session_projection::ProjectionStatus::Succeeded
        },
        created_at: chrono::Utc::now(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
        summary: SessionProjectionReportSummaryView {
            canonical_event_count: Some(event_count),
            mapping_direction: Some(imported.report.direction),
            mapping_overall: Some(imported.report.overall),
            preserved_count,
            normalized_count,
            dropped_count,
        },
        item_count: imported.report.issues.len(),
        items: imported
            .report
            .issues
            .iter()
            .enumerate()
            .map(|(index, issue)| SessionProjectionReportItemView {
                item_order: index as i64,
                fidelity: match issue.disposition {
                    crate::session::Fidelity::Preserved => {
                        crate::session_projection::ProjectionFidelity::Preserved
                    }
                    crate::session::Fidelity::Normalized
                    | crate::session::Fidelity::Downgraded => {
                        crate::session_projection::ProjectionFidelity::Normalized
                    }
                    crate::session::Fidelity::Dropped
                    | crate::session::Fidelity::Unsupported => {
                        crate::session_projection::ProjectionFidelity::Dropped
                    }
                },
                scope: crate::session_projection::ProjectionItemScope::ProviderPayload,
                field_path: issue.path.clone(),
                reason: Some(issue.message.clone()),
                details: issue.raw.clone(),
            })
            .collect(),
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStats {
    pub event_id: String,
    pub char_count: usize,
    pub byte_size: usize,
    pub visible_char_count: usize,
    pub visible_byte_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub provider_id: String,
    pub session_id: String,
    pub events: Vec<EventStats>,
    pub total_char_count: usize,
    pub total_byte_size: usize,
    pub total_visible_char_count: usize,
    pub total_visible_byte_size: usize,
}

pub fn compute_session_stats(provider_id: &str, session_id: &str) -> Result<SessionStats> {
    let detail = get_session_detail_view(provider_id, session_id)?;
    let mut events = Vec::with_capacity(detail.events.len());
    let mut total_char_count = 0usize;
    let mut total_byte_size = 0usize;
    let mut total_visible_char_count = 0usize;
    let mut total_visible_byte_size = 0usize;

    for event in &detail.events {
        let full_text = provider::event_text(event);
        let visible_text = provider::event_visible_text(event);
        let char_count = full_text.chars().count();
        let byte_size = full_text.len();
        let visible_char_count = visible_text.chars().count();
        let visible_byte_size = visible_text.len();

        total_char_count = total_char_count.saturating_add(char_count);
        total_byte_size = total_byte_size.saturating_add(byte_size);
        total_visible_char_count = total_visible_char_count.saturating_add(visible_char_count);
        total_visible_byte_size = total_visible_byte_size.saturating_add(visible_byte_size);

        events.push(EventStats {
            event_id: event.id.clone(),
            char_count,
            byte_size,
            visible_char_count,
            visible_byte_size,
        });
    }

    Ok(SessionStats {
        provider_id: provider_id.to_string(),
        session_id: session_id.to_string(),
        events,
        total_char_count,
        total_byte_size,
        total_visible_char_count,
        total_visible_byte_size,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionActivityBucketUnit {
    Minute,
    Hour,
    TwelveHour,
    Adaptive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionActivityBucket {
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
    pub event_count: usize,
    pub message_count: usize,
    pub activity_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionActivityTimeline {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    pub bucket_unit: SessionActivityBucketUnit,
    pub bucket_seconds: i64,
    pub buckets: Vec<SessionActivityBucket>,
    pub total_events: usize,
    pub total_messages: usize,
    pub total_activity: f64,
}

pub fn compute_session_activity_timeline(
    provider_id: &str,
    session_id: &str,
) -> Result<SessionActivityTimeline> {
    let conn = local_store::open_database()?;
    compute_session_activity_timeline_in_connection(&conn, provider_id, session_id)
}

pub(super) fn compute_session_activity_timeline_in_connection(
    conn: &rusqlite::Connection,
    provider_id: &str,
    session_id: &str,
) -> Result<SessionActivityTimeline> {
    use chrono::TimeDelta;

    let identity = crate::storage::snapshot_store::SnapshotStore::new(conn)
        .find_session_identity(provider_id, session_id)?
        .with_context(|| format!("Session is not indexed: {provider_id}/{session_id}"))?;
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    let activity = import_session_activity(provider_id, provider.as_ref(), &identity)?;
    let event_timestamps = activity
        .events
        .iter()
        .map(|event| event.timestamp)
        .filter(utils::is_plausible_session_time)
        .collect::<Vec<_>>();
    let first_event_at = event_timestamps.iter().copied().min();
    let last_event_at = event_timestamps.iter().copied().max();
    let created_at = match (
        activity.created_at.filter(utils::is_plausible_session_time),
        first_event_at,
    ) {
        (Some(source), Some(event)) => Some(source.min(event)),
        (source, event) => source.or(event),
    };
    let last_active_at = match (
        activity
            .last_active_at
            .filter(utils::is_plausible_session_time),
        last_event_at,
    ) {
        (Some(source), Some(event)) => Some(source.max(event)),
        (source, event) => source.or(event),
    };

    let range_start = created_at
        .or_else(|| event_timestamps.first().copied())
        .unwrap_or_else(chrono::Utc::now);
    let range_end = last_active_at
        .or_else(|| event_timestamps.last().copied())
        .unwrap_or(range_start);
    let range_end = if range_end < range_start {
        range_start
    } else {
        range_end
    };

    let span = range_end.signed_duration_since(range_start);
    let (_, mut bucket_seconds) = choose_activity_bucket(span);
    let bucket_count = activity_bucket_count(span, &mut bucket_seconds);
    let bucket_unit = activity_bucket_unit(bucket_seconds);
    let mut buckets = Vec::with_capacity(bucket_count);
    for index in 0..bucket_count {
        let start = range_start + TimeDelta::seconds(index as i64 * bucket_seconds);
        let end = if index + 1 == bucket_count {
            range_end
        } else {
            range_start + TimeDelta::seconds((index as i64 + 1) * bucket_seconds)
        };
        buckets.push(SessionActivityBucket {
            start,
            end,
            event_count: 0,
            message_count: 0,
            activity_score: 0.0,
        });
    }

    let mut total_activity = 0.0;
    let mut total_messages = 0usize;
    for event in &activity.events {
        let weight = event_activity_weight(&event.kind, event.visible_message);
        total_activity += weight;
        if event.visible_message {
            total_messages += 1;
        }
        if let Some(bucket) = bucket_for_timestamp(
            event.timestamp,
            range_start,
            range_end,
            bucket_seconds,
            &mut buckets,
        ) {
            bucket.event_count += 1;
            bucket.activity_score += weight;
            if event.visible_message {
                bucket.message_count += 1;
            }
        }
    }

    Ok(SessionActivityTimeline {
        provider_id: provider_id.to_string(),
        session_id: session_id.to_string(),
        created_at,
        last_active_at,
        bucket_unit,
        bucket_seconds,
        buckets,
        total_events: activity.events.len(),
        total_messages,
        total_activity,
    })
}

fn resolve_session_created_at(
    context: Option<chrono::DateTime<chrono::Utc>>,
    events: &[Event],
) -> Option<chrono::DateTime<chrono::Utc>> {
    let from_context = context.filter(utils::is_plausible_session_time);
    let from_events = events
        .iter()
        .map(|event| event.timestamp)
        .filter(utils::is_plausible_session_time)
        .min();
    match (from_context, from_events) {
        (Some(context), Some(event)) => Some(context.min(event)),
        (context, event) => context.or(event),
    }
}

#[derive(Debug)]
struct SourceActivityEvent {
    kind: EventKind,
    timestamp: chrono::DateTime<chrono::Utc>,
    visible_message: bool,
}

#[derive(Debug)]
struct SourceSessionActivity {
    canonical_session_id: String,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    events: Vec<SourceActivityEvent>,
}

fn import_session_activity(
    provider_id: &str,
    provider: &dyn Provider,
    identity: &ProjectedSessionIdentityRow,
) -> Result<SourceSessionActivity> {
    let source_path = identity
        .source_path
        .as_deref()
        .filter(|value| !value.is_empty())
        .with_context(|| {
            format!(
                "Session has no source locator: {provider_id}/{}",
                identity.canonical_session_id
            )
        })?;
    if !provider.capabilities().import {
        anyhow::bail!("Provider does not support session activity reads: {provider_id}");
    }
    let imported = provider.import_session(source_path).with_context(|| {
        format!(
            "Failed to read session activity from provider source: {provider_id}/{}",
            identity
                .provider_session_id
                .as_deref()
                .unwrap_or(&identity.canonical_session_id)
        )
    })?;
    let events = imported
        .session
        .events
        .iter()
        .map(|event| SourceActivityEvent {
            kind: event.kind,
            timestamp: event.timestamp,
            visible_message: provider::event_is_visible_message(event),
        })
        .collect();
    let last_active_at = imported.session.context.last_active_at.or_else(|| {
        identity
            .last_active_at_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
    });

    Ok(SourceSessionActivity {
        canonical_session_id: identity.canonical_session_id.clone(),
        created_at: imported.session.context.created_at,
        last_active_at,
        events,
    })
}

pub const PROVIDER_ACTIVITY_DEFAULT_HOURS: i64 = 72;
const PROVIDER_ACTIVITY_MAX_HOURS: i64 = 24 * 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderActivityTimeline {
    pub provider_id: String,
    pub hours: i64,
    pub bucket_seconds: i64,
    pub range_start: chrono::DateTime<chrono::Utc>,
    pub range_end: chrono::DateTime<chrono::Utc>,
    pub buckets: Vec<SessionActivityBucket>,
    pub total_activity: f64,
    pub projected_sessions: usize,
    pub sessions_with_activity: usize,
}

pub fn compute_provider_activity_timeline(
    provider_id: &str,
    workspace: Option<&str>,
    hours: i64,
    all_workspaces: bool,
    all_time: bool,
) -> Result<ProviderActivityTimeline> {
    let conn = local_store::open_database()?;
    compute_provider_activity_timeline_in_connection(
        &conn,
        provider_id,
        workspace,
        hours,
        all_workspaces,
        all_time,
    )
}

pub(super) fn compute_provider_activity_timeline_in_connection(
    conn: &rusqlite::Connection,
    provider_id: &str,
    workspace: Option<&str>,
    hours: i64,
    all_workspaces: bool,
    all_time: bool,
) -> Result<ProviderActivityTimeline> {
    use chrono::TimeDelta;

    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let hours = hours.clamp(1, PROVIDER_ACTIVITY_MAX_HOURS);
    let range_end = chrono::Utc::now();
    let requested_range_start = (!all_time).then(|| range_end - TimeDelta::hours(hours));
    let sessions = crate::storage::snapshot_store::SnapshotStore::new(conn)
        .list_provider_session_identities(provider_id)?
        .into_iter()
        .filter(|session| {
            if all_workspaces {
                return true;
            }
            prov.normalized_workspace_key(workspace).as_deref()
                == prov
                    .normalized_workspace_key(session.workspace_dir.as_deref())
                    .as_deref()
        })
        .collect::<Vec<_>>();
    let projected_sessions = sessions.len();
    let mut activities = Vec::with_capacity(sessions.len());
    for session in &sessions {
        activities.push(import_session_activity(
            provider_id,
            prov.as_ref(),
            session,
        )?);
    }
    let events = activities
        .iter()
        .flat_map(|activity| {
            activity.events.iter().filter_map(|event| {
                if requested_range_start.is_none_or(|start| event.timestamp >= start)
                    && event.timestamp <= range_end
                {
                    Some((&activity.canonical_session_id, event))
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();
    let range_start = requested_range_start.unwrap_or_else(|| {
        events
            .iter()
            .map(|(_, event)| event.timestamp)
            .min()
            .unwrap_or(range_end)
    });
    let span = range_end.signed_duration_since(range_start);
    let mut bucket_seconds = if all_time {
        choose_activity_bucket(span).1
    } else if hours <= 7 * 24 {
        60 * 60
    } else {
        12 * 60 * 60
    };
    let bucket_count = if all_time {
        activity_bucket_count(span, &mut bucket_seconds)
    } else {
        ((span.num_seconds().max(0) + bucket_seconds - 1) / bucket_seconds).max(1) as usize
    };

    let mut buckets = Vec::with_capacity(bucket_count);
    for index in 0..bucket_count {
        let start = range_start + TimeDelta::seconds(index as i64 * bucket_seconds);
        let end = if index + 1 == bucket_count {
            range_end
        } else {
            range_start + TimeDelta::seconds((index as i64 + 1) * bucket_seconds)
        };
        buckets.push(SessionActivityBucket {
            start,
            end,
            event_count: 0,
            message_count: 0,
            activity_score: 0.0,
        });
    }
    let mut sessions_with_events = HashSet::new();
    let mut total_activity = 0.0f64;
    for (canonical_session_id, event) in &events {
        let timestamp = event.timestamp;
        sessions_with_events.insert(canonical_session_id.as_str());
        let weight = event_activity_weight(&event.kind, event.visible_message);
        total_activity += weight;
        if let Some(bucket) = bucket_for_timestamp(
            timestamp,
            range_start,
            range_end,
            bucket_seconds,
            &mut buckets,
        ) {
            bucket.event_count += 1;
            bucket.activity_score += weight;
            if event.visible_message {
                bucket.message_count += 1;
            }
        }
    }
    let actual_hours = ((span.num_seconds().max(0) + 3599) / 3600).max(1);

    Ok(ProviderActivityTimeline {
        provider_id: provider_id.to_string(),
        hours: if all_time { actual_hours } else { hours },
        bucket_seconds,
        range_start,
        range_end,
        buckets,
        total_activity,
        projected_sessions,
        sessions_with_activity: sessions_with_events.len(),
    })
}

fn event_activity_weight(kind: &EventKind, visible_message: bool) -> f64 {
    match kind {
        EventKind::Lifecycle => 0.25,
        EventKind::Message if visible_message => 3.0,
        EventKind::Message => 1.5,
        EventKind::Action => 2.0,
        EventKind::Observation => 1.75,
        EventKind::Other => 0.5,
    }
}

pub(super) fn choose_activity_bucket(span: chrono::TimeDelta) -> (SessionActivityBucketUnit, i64) {
    if span < chrono::TimeDelta::hours(1) {
        (SessionActivityBucketUnit::Minute, 60)
    } else if span < chrono::TimeDelta::days(1) {
        (SessionActivityBucketUnit::Hour, 60 * 60)
    } else {
        (SessionActivityBucketUnit::TwelveHour, 12 * 60 * 60)
    }
}

pub(super) fn activity_bucket_unit(bucket_seconds: i64) -> SessionActivityBucketUnit {
    match bucket_seconds {
        60 => SessionActivityBucketUnit::Minute,
        3_600 => SessionActivityBucketUnit::Hour,
        43_200 => SessionActivityBucketUnit::TwelveHour,
        _ => SessionActivityBucketUnit::Adaptive,
    }
}

pub(super) fn activity_bucket_count(span: chrono::TimeDelta, bucket_seconds: &mut i64) -> usize {
    const MAX_BUCKETS: i64 = 96;
    let span_seconds = span.num_seconds().max(0);
    if span_seconds == 0 {
        return 1;
    }
    let mut count = (span_seconds + *bucket_seconds - 1) / *bucket_seconds;
    while count > MAX_BUCKETS {
        *bucket_seconds *= 2;
        count = (span_seconds + *bucket_seconds - 1) / *bucket_seconds;
    }
    count.max(1) as usize
}

fn bucket_for_timestamp(
    timestamp: chrono::DateTime<chrono::Utc>,
    range_start: chrono::DateTime<chrono::Utc>,
    range_end: chrono::DateTime<chrono::Utc>,
    bucket_seconds: i64,
    buckets: &mut [SessionActivityBucket],
) -> Option<&mut SessionActivityBucket> {
    if timestamp < range_start || timestamp > range_end {
        return None;
    }
    let offset = timestamp
        .signed_duration_since(range_start)
        .num_seconds()
        .max(0);
    let index = (offset / bucket_seconds).min(buckets.len().saturating_sub(1) as i64) as usize;
    buckets.get_mut(index)
}

pub(super) fn load_canonical_session_from_meta(
    provider: &dyn provider::Provider,
    provider_id: &str,
    meta: ProviderSessionSummary,
) -> Result<ImportedSession> {
    let source_path = meta
        .source_path
        .as_deref()
        .context("Session has no source path")?;
    let mut imported = provider.import_session(source_path)?;
    enrich_imported_session_from_meta(&mut imported, provider_id, &meta);
    Ok(imported)
}

fn enrich_imported_session_from_meta(
    imported: &mut ImportedSession,
    provider_id: &str,
    meta: &ProviderSessionSummary,
) {
    let display_title = resolved_display_title(provider_id, meta);
    apply_imported_session_title(imported, meta, display_title);
    if imported.session.context.workspace.is_none() {
        imported.session.context.workspace = meta.project_dir.clone();
    }
    if imported.session.context.last_active_at.is_none() {
        imported.session.context.last_active_at = meta
            .last_active_at
            .and_then(chrono::DateTime::from_timestamp_millis);
    }
    if imported
        .provenance
        .aliases
        .iter()
        .all(|alias| alias.provider_id != provider_id || alias.session_id != meta.session_id)
    {
        imported
            .provenance
            .aliases
            .push(crate::session::ProviderRef {
                provider_id: provider_id.to_string(),
                session_id: meta.session_id.clone(),
                source_path: meta.source_path.clone(),
            });
    }
}

pub(super) fn apply_imported_session_title(
    imported: &mut ImportedSession,
    meta: &ProviderSessionSummary,
    display_title: Option<String>,
) {
    imported.session.identity.title =
        display_title.or(imported.session.identity.title.clone()).or(meta.title.clone());
}

fn resolved_display_title(provider_id: &str, meta: &ProviderSessionSummary) -> Option<String> {
    let session_states = session_state::load_state_store().unwrap_or_default();
    let workspace_dir =
        session_management::normalized_workspace_key(provider_id, meta.project_dir.as_deref());
    session_state::resolve_session_state(
        &session_states,
        provider_id,
        &meta.session_id,
        workspace_dir.as_deref(),
    )
    .display_title
}
